//! Clipboard history: the last 200 copies, kept on this computer only,
//! encrypted at rest.
//!
//! `iliada-searchd` watches the clipboard (the data-control protocol, which
//! only an unsandboxed program can use) and records text, images and files.
//! Super+V shows them in the search overlay. Sandboxed apps cannot read
//! this history: they are on a sandboxed display socket that is refused
//! data-control.
//!
//! Copies marked secret (the `x-kde-passwordManagerHint` and
//! `password-manager` MIME types password managers set) are never saved,
//! nor are copies from apps the person excluded in Settings → Privacy.
//! History can be turned off there too.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

use crate::{Action, Hit, Kind};

/// How many copies are kept (pinned ones are never dropped to make room).
pub const MAX_ITEMS: usize = 200;
/// A single copy larger than this is not saved.
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

const MAGIC: &[u8] = b"ILCB1\n";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// What a saved copy is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    Text,
    Image,
    Files,
}

/// One saved copy. The image bytes, when any, live in a sibling file, not
/// in this struct, so listing the history stays small.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub kind: ClipKind,
    pub mime: String,
    pub preview: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob_file: Option<String>,
    pub created: u64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A copy just taken from the clipboard, before it is saved.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub mimes: Vec<String>,
    pub mime: String,
    pub data: Vec<u8>,
    pub source: Option<String>,
}

/// Settings → Privacy: whether history is kept, and which apps are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "on")]
    pub enabled: bool,
    /// App ids (and last components) whose copies are never saved.
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn on() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config { enabled: true, exclude: Vec::new() }
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        let text = fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        let header = format!(
            "# Clipboard history (Super+V). The search service keeps the last {MAX_ITEMS} copies,\n\
             # encrypted, on this computer only. Passwords marked secret are never saved.\n"
        );
        atomic_write(path, format!("{header}{text}").as_bytes(), 0o600)
    }

    pub fn excludes(&self, app: &str) -> bool {
        let app = app.trim();
        if app.is_empty() {
            return false;
        }
        let lower = app.to_ascii_lowercase();
        let short = lower.rsplit(['.', '-']).next().unwrap_or(&lower);
        self.exclude.iter().any(|e| {
            let e = e.trim().to_ascii_lowercase();
            if e.is_empty() {
                return false;
            }
            let e_short = e.rsplit(['.', '-']).next().unwrap_or(&e);
            lower == e || short == e || short == e_short || lower.ends_with(&format!(".{e}"))
        })
    }
}

/// Whether these MIME types mean a password manager marked the copy secret.
pub fn is_secret(mimes: &[String]) -> bool {
    mimes.iter().any(|m| mime_is_secret(m))
}

pub fn mime_is_secret(mime: &str) -> bool {
    let l = mime.trim().to_ascii_lowercase();
    let base = l.split(';').next().unwrap_or(&l).trim();
    base.contains("passwordmanagerhint")
        || base.contains("password-manager")
        || base.contains("password_manager")
        || base == "x-kde-passwordmanagerhint"
        || l.contains("x-kde-passwordmanagerhint")
}

/// The MIME type we would save from this offer, if any.
pub fn choose_mime(mimes: &[String]) -> Option<String> {
    let useful: Vec<&String> = mimes.iter().filter(|m| !mime_is_secret(m) && !meta_mime(m)).collect();
    let pick = |want: &[&str]| {
        useful.iter().find(|m| {
            let base = m.split(';').next().unwrap_or(m).trim();
            want.iter().any(|w| base.eq_ignore_ascii_case(w))
        })
    };
    pick(&["text/uri-list"])
        .or_else(|| {
            useful.iter().find(|m| {
                let base = m.split(';').next().unwrap_or(m).trim();
                base.eq_ignore_ascii_case("image/png")
                    || base.eq_ignore_ascii_case("image/jpeg")
                    || base.eq_ignore_ascii_case("image/jpg")
                    || base.eq_ignore_ascii_case("image/webp")
                    || base.eq_ignore_ascii_case("image/bmp")
                    || base.eq_ignore_ascii_case("image/gif")
                    || base.eq_ignore_ascii_case("image/tiff")
            })
        })
        .or_else(|| pick(&["text/plain"]))
        .or_else(|| pick(&["UTF8_STRING", "TEXT", "STRING"]))
        .or_else(|| useful.iter().find(|m| m.to_ascii_lowercase().starts_with("text/")))
        .map(|m| (*m).clone())
}

fn meta_mime(mime: &str) -> bool {
    let base = mime.split(';').next().unwrap_or(mime).trim();
    matches!(
        base,
        "TIMESTAMP" | "TARGETS" | "MULTIPLE" | "SAVE_TARGETS" | "DELETE" | "INSERT_TARGET" | "application/x-qt-image"
    ) || base.eq_ignore_ascii_case("text/x-moz-url")
}

fn kind_of(mime: &str) -> ClipKind {
    let base = mime.split(';').next().unwrap_or(mime).trim();
    if base.eq_ignore_ascii_case("text/uri-list") {
        ClipKind::Files
    } else if base.to_ascii_lowercase().starts_with("image/") {
        ClipKind::Image
    } else {
        ClipKind::Text
    }
}

fn preview_text(text: &str) -> String {
    let one: String = text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    let t = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= 200 { t } else { format!("{}...", t.chars().take(200).collect::<String>()) }
}

fn preview_files(text: &str) -> String {
    let names: Vec<String> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let p = l.trim().trim_start_matches("file://");
            let decoded = url_path(p);
            Path::new(&decoded).file_name().and_then(|n| n.to_str()).unwrap_or(&decoded).to_owned()
        })
        .collect();
    match names.as_slice() {
        [] => "Files".into(),
        [one] => one.clone(),
        [a, b] => format!("{a}, {b}"),
        [a, rest @ ..] => format!("{a} and {} more", rest.len()),
    }
}

fn url_path(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Ok(h), Ok(l)) = (
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 2]).unwrap_or(""), 16),
                u8::from_str_radix(std::str::from_utf8(&b[i + 2..i + 3]).unwrap_or(""), 16),
            )
        {
            out.push(char::from((h << 4) | l));
            i += 3;
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn preview_image(mime: &str, n: usize) -> String {
    let base = mime.split(';').next().unwrap_or(mime).trim();
    let kind = base.split('/').nth(1).unwrap_or("image").to_ascii_uppercase();
    format!("Image ({kind}, {})", bytes(n))
}

fn bytes(n: usize) -> String {
    if n < 1000 {
        format!("{n} B")
    } else if n < 1_000_000 {
        format!("{:.0} KB", n as f64 / 1000.0)
    } else {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The history on disk, and the settings that govern it.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    config_path: PathBuf,
    config: Config,
    key: [u8; KEY_LEN],
    next_id: u64,
    items: Vec<Item>,
}

#[derive(Serialize, Deserialize)]
struct Disk {
    next_id: u64,
    items: Vec<Item>,
}

impl Store {
    /// Default folders: `~/.local/state/iliada/clipboard` and
    /// `~/.config/iliada/clipboard.toml`.
    pub fn default_paths() -> Option<(PathBuf, PathBuf)> {
        Some((crate::state_dir()?.join("clipboard"), crate::config_dir()?.join("clipboard.toml")))
    }

    pub fn open(dir: PathBuf, config_path: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        fs::create_dir_all(dir.join("blobs"))?;
        let key = load_or_create_key(&dir.join("key"))?;
        let config = Config::load(&config_path);
        let mut store = Store { dir, config_path, config, key, next_id: 1, items: Vec::new() };
        if let Err(e) = store.load() {
            tracing::warn!("clipboard history could not be read (starting empty): {e:#}");
        }
        Ok(store)
    }

    pub fn reload_config(&mut self) {
        self.config = Config::load(&self.config_path);
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn set_enabled(&mut self, on: bool) -> Result<()> {
        self.reload_config();
        self.config.enabled = on;
        self.config.save(&self.config_path)
    }

    pub fn set_excluded(&mut self, app: &str, exclude: bool) -> Result<()> {
        self.reload_config();
        let app = app.trim();
        if app.is_empty() {
            return Ok(());
        }
        if exclude {
            if !self.config.excludes(app) {
                self.config.exclude.push(app.to_owned());
            }
        } else {
            self.config.exclude.retain(|e| !ids_match(e, app));
        }
        self.config.save(&self.config_path)
    }

    pub fn record(&mut self, incoming: Incoming) -> bool {
        self.reload_config();
        if !self.config.enabled {
            return false;
        }
        if is_secret(&incoming.mimes) || mime_is_secret(&incoming.mime) {
            return false;
        }
        if let Some(src) = incoming.source.as_deref()
            && self.config.excludes(src)
        {
            return false;
        }
        if incoming.data.is_empty() || incoming.data.len() > MAX_BYTES {
            return false;
        }
        let kind = kind_of(&incoming.mime);
        let (text, blob, preview) = match kind {
            ClipKind::Text => {
                let t = String::from_utf8_lossy(&incoming.data).trim_end_matches('\0').to_string();
                if t.trim().is_empty() {
                    return false;
                }
                let p = preview_text(&t);
                (Some(t), None, p)
            }
            ClipKind::Files => {
                let t = String::from_utf8_lossy(&incoming.data).trim_end_matches('\0').to_string();
                if t.lines().all(|l| l.is_empty() || l.starts_with('#')) {
                    return false;
                }
                let p = preview_files(&t);
                (Some(t), None, p)
            }
            ClipKind::Image => (None, Some(incoming.data.clone()), preview_image(&incoming.mime, incoming.data.len())),
        };
        if let Some(last) = self.items.first().cloned()
            && last.kind == kind
            && last.mime == incoming.mime
            && last.text == text
            && (blob.is_none() || blob.as_deref() == self.blob(&last).as_deref())
        {
            return false;
        }
        let id = self.next_id;
        self.next_id += 1;
        let blob_file = if blob.is_some() { Some(id.to_string()) } else { None };
        if let (Some(name), Some(bytes)) = (&blob_file, &blob)
            && let Err(e) = self.write_blob(name, bytes)
        {
            tracing::warn!("clipboard image not saved: {e:#}");
            return false;
        }
        self.items.insert(
            0,
            Item {
                id,
                kind,
                mime: incoming.mime,
                preview,
                text,
                blob_file,
                created: now(),
                pinned: false,
                source: incoming.source.filter(|s| !s.trim().is_empty()),
            },
        );
        self.evict();
        if let Err(e) = self.save() {
            tracing::warn!("clipboard history not written: {e:#}");
        }
        true
    }

    fn evict(&mut self) {
        while self.items.len() > MAX_ITEMS {
            let pos = self.items.iter().rposition(|i| !i.pinned);
            let Some(pos) = pos else { break };
            let gone = self.items.remove(pos);
            self.remove_blob(&gone);
        }
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The image bytes for `item`, if it is an image we still have.
    pub fn blob(&self, item: &Item) -> Option<Vec<u8>> {
        let name = item.blob_file.as_deref()?;
        self.read_blob(name).ok()
    }

    pub fn pin(&mut self, id: u64, pinned: bool) -> bool {
        let Some(item) = self.items.iter_mut().find(|i| i.id == id) else { return false };
        item.pinned = pinned;
        let _ = self.save();
        true
    }

    pub fn delete(&mut self, id: u64) -> bool {
        let Some(pos) = self.items.iter().position(|i| i.id == id) else { return false };
        let gone = self.items.remove(pos);
        self.remove_blob(&gone);
        let _ = self.save();
        true
    }

    pub fn clear(&mut self) {
        let items = std::mem::take(&mut self.items);
        for item in items {
            self.remove_blob(&item);
        }
        let _ = self.save();
    }

    /// Results for Super+V, newest first, pinned at the top, filtered when
    /// `query` is not empty.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q = query.trim();
        let lower = q.to_lowercase();
        let mut hits: Vec<(i64, &Item)> = self
            .items
            .iter()
            .filter_map(|item| {
                let score = if q.is_empty() { 0 } else { score_item(item, q, &lower)? };
                Some((score, item))
            })
            .collect();
        if !q.is_empty() {
            hits.sort_by(|a, b| {
                b.0.cmp(&a.0).then_with(|| b.1.pinned.cmp(&a.1.pinned)).then_with(|| b.1.id.cmp(&a.1.id))
            });
        } else {
            hits.sort_by(|a, b| b.1.pinned.cmp(&a.1.pinned).then_with(|| b.1.id.cmp(&a.1.id)));
        }
        hits.into_iter().take(limit).map(|(_, item)| item_hit(item)).collect()
    }

    pub fn extras(&self, id: u64) -> Vec<Hit> {
        let Some(item) = self.get(id) else { return Vec::new() };
        let mut v = vec![
            Hit::new(Kind::Action, "Paste", Action::RestoreClipboard { id, paste: true })
                .icon_name("edit-paste-symbolic"),
            Hit::new(Kind::Action, "Copy", Action::RestoreClipboard { id, paste: false })
                .icon_name("edit-copy-symbolic"),
        ];
        if item.pinned {
            v.push(
                Hit::new(Kind::Action, "Unpin", Action::ClipboardPin { id, pinned: false })
                    .icon_name("view-pin-symbolic"),
            );
        } else {
            v.push(
                Hit::new(Kind::Action, "Pin", Action::ClipboardPin { id, pinned: true }).icon_name("view-pin-symbolic"),
            );
        }
        v.push(Hit::new(Kind::Action, "Delete", Action::ClipboardDelete(id)).icon_name("edit-delete-symbolic"));
        v.push(Hit::new(Kind::Action, "Clear All", Action::ClipboardClear).icon_name("edit-clear-all-symbolic"));
        v
    }

    fn load(&mut self) -> Result<()> {
        let path = self.dir.join("store");
        if !path.exists() {
            return Ok(());
        }
        let plain = decrypt(&self.key, &fs::read(&path)?)?;
        let disk: Disk = serde_json::from_slice(&plain).context("clipboard history is not readable")?;
        self.next_id = disk.next_id.max(disk.items.iter().map(|i| i.id + 1).max().unwrap_or(1));
        self.items = disk.items;
        Ok(())
    }

    fn save(&self) -> Result<()> {
        let disk = Disk { next_id: self.next_id, items: self.items.clone() };
        let plain = serde_json::to_vec(&disk)?;
        let sealed = encrypt(&self.key, &plain)?;
        atomic_write(&self.dir.join("store"), &sealed, 0o600)
    }

    fn write_blob(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let sealed = encrypt(&self.key, bytes)?;
        atomic_write(&self.dir.join("blobs").join(safe_name(name)), &sealed, 0o600)
    }

    fn read_blob(&self, name: &str) -> Result<Vec<u8>> {
        decrypt(&self.key, &fs::read(self.dir.join("blobs").join(safe_name(name)))?)
    }

    fn remove_blob(&self, item: &Item) {
        if let Some(name) = &item.blob_file {
            let _ = fs::remove_file(self.dir.join("blobs").join(safe_name(name)));
        }
    }
}

fn ids_match(listed: &str, app: &str) -> bool {
    Config { enabled: true, exclude: vec![listed.to_owned()] }.excludes(app)
}

fn safe_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn score_item(item: &Item, q: &str, lower: &str) -> Option<i64> {
    let preview = item.preview.to_lowercase();
    let text = item.text.as_deref().unwrap_or("").to_lowercase();
    let source = item.source.as_deref().unwrap_or("").to_lowercase();
    if preview.contains(lower) || text.contains(lower) || source.contains(lower) {
        let mut s = 1000i64;
        if preview.starts_with(lower) {
            s += 500;
        }
        if item.pinned {
            s += 100;
        }
        s += q.len() as i64;
        Some(s)
    } else {
        None
    }
}

fn item_hit(item: &Item) -> Hit {
    let icon = match item.kind {
        ClipKind::Image => "image-x-generic-symbolic",
        ClipKind::Files => "folder-documents-symbolic",
        ClipKind::Text => "edit-paste-symbolic",
    };
    Hit::new(Kind::Clipboard, item.preview.clone(), Action::RestoreClipboard { id: item.id, paste: true })
        .icon_name(icon)
        .key(format!("clipboard:{}", item.id))
        .clip(item.id, item.pinned, item.created, item.source.clone().unwrap_or_default())
}

fn load_or_create_key(path: &Path) -> Result<[u8; KEY_LEN]> {
    if path.exists() {
        let b = fs::read(path)?;
        anyhow::ensure!(b.len() == KEY_LEN, "clipboard key is the wrong size");
        let mut k = [0u8; KEY_LEN];
        k.copy_from_slice(&b);
        return Ok(k);
    }
    let k = random_bytes::<KEY_LEN>()?;
    atomic_write(path, &k, 0o600)?;
    Ok(k)
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("no random bytes: {e}"))?;
    Ok(b)
}

fn cipher(key: &[u8; KEY_LEN]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(Key::from_slice(key))
}

fn encrypt(key: &[u8; KEY_LEN], plain: &[u8]) -> Result<Vec<u8>> {
    let nonce = random_bytes::<NONCE_LEN>()?;
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + plain.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    let sealed = cipher(key)
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: MAGIC })
        .map_err(|_| anyhow::anyhow!("clipboard encryption failed"))?;
    out.extend_from_slice(&sealed);
    Ok(out)
}

fn decrypt(key: &[u8; KEY_LEN], sealed: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(sealed.starts_with(MAGIC), "clipboard history is not in a known format");
    anyhow::ensure!(sealed.len() > MAGIC.len() + NONCE_LEN, "clipboard history is truncated");
    let nonce = &sealed[MAGIC.len()..MAGIC.len() + NONCE_LEN];
    let body = &sealed[MAGIC.len() + NONCE_LEN..];
    cipher(key)
        .decrypt(XNonce::from_slice(nonce), Payload { msg: body, aad: MAGIC })
        .map_err(|_| anyhow::anyhow!("clipboard history could not be decrypted"))
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(mode));
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Write `item` back to the clipboard with `wl-copy` (text, images, files).
pub fn restore(item: &Item, blob: Option<&[u8]>) -> Result<()> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-copy")
        .args(["-t", &item.mime, "--"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("putting the copy back on the clipboard (wl-copy)")?;
    {
        let mut stdin = child.stdin.take().context("wl-copy stdin")?;
        match item.kind {
            ClipKind::Image => stdin.write_all(blob.context("the image is no longer on disk")?)?,
            ClipKind::Text | ClipKind::Files => stdin.write_all(item.text.as_deref().unwrap_or("").as_bytes())?,
        }
    }
    let status = child.wait()?;
    anyhow::ensure!(status.success(), "wl-copy did not accept the copy");
    Ok(())
}

/// Paste into the window that had the focus (`wtype` sends Ctrl+V). The
/// overlay must already be hidden.
pub fn paste() -> Result<()> {
    use std::process::{Command, Stdio};
    let status = Command::new("wtype")
        .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("pasting (wtype)")?;
    anyhow::ensure!(status.success(), "the paste key was not sent");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(d.path().join("c"), d.path().join("clipboard.toml")).unwrap();
        (d, store)
    }

    fn text(s: &str) -> Incoming {
        Incoming {
            mimes: vec!["text/plain".into()],
            mime: "text/plain".into(),
            data: s.as_bytes().to_vec(),
            source: None,
        }
    }

    #[test]
    fn secret_mimes_are_recognised() {
        assert!(is_secret(&["text/plain".into(), "x-kde-passwordManagerHint".into()]));
        assert!(is_secret(&["application/x-kde-passwordManagerHint".into()]));
        assert!(is_secret(&["text/x-password-manager-secret".into()]));
        assert!(mime_is_secret("X-KDE-PasswordManagerHint; charset=utf-8"));
        assert!(!is_secret(&["text/plain".into(), "text/html".into()]));
    }

    #[test]
    fn mime_choice_prefers_files_then_images_then_text() {
        assert_eq!(
            choose_mime(&["text/plain".into(), "text/uri-list".into(), "image/png".into()]).unwrap(),
            "text/uri-list"
        );
        assert_eq!(choose_mime(&["text/plain".into(), "image/png".into()]).unwrap(), "image/png");
        assert_eq!(choose_mime(&["UTF8_STRING".into(), "text/plain".into()]).unwrap(), "text/plain");
        assert!(choose_mime(&["TARGETS".into(), "TIMESTAMP".into()]).is_none());
        assert!(choose_mime(&["x-kde-passwordManagerHint".into()]).is_none());
    }

    #[test]
    fn secrets_and_excluded_apps_and_the_off_switch_are_not_saved() {
        let (_d, mut s) = tmp();
        let mut secret = text("hunter2");
        secret.mimes.push("x-kde-passwordManagerHint".into());
        assert!(!s.record(secret));
        assert!(s.items.is_empty());

        let mut from_pw = text("from keepass");
        from_pw.source = Some("org.keepassxc.KeePassXC".into());
        s.set_excluded("org.keepassxc.KeePassXC", true).unwrap();
        assert!(!s.record(from_pw));
        assert!(s.config().excludes("keepassxc"));

        s.set_enabled(false).unwrap();
        assert!(!s.record(text("hello")));
        s.set_enabled(true).unwrap();
        assert!(s.record(text("hello")));
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].preview, "hello");
    }

    #[test]
    fn history_is_encrypted_capped_and_searchable() {
        let (d, mut s) = tmp();
        assert!(s.record(text("alpha one")));
        assert!(s.record(text("beta two")));
        // The same copy again is not duplicated.
        assert!(!s.record(text("beta two")));
        assert_eq!(s.items.len(), 2);
        s.pin(s.items[0].id, true);
        assert!(s.items[0].pinned);

        let raw = fs::read(d.path().join("c/store")).unwrap();
        assert!(raw.starts_with(MAGIC));
        assert!(!String::from_utf8_lossy(&raw).contains("beta two"));

        let s2 = Store::open(d.path().join("c"), d.path().join("clipboard.toml")).unwrap();
        assert_eq!(s2.items.len(), 2);
        assert_eq!(s2.items[0].preview, "beta two");
        assert!(s2.items[0].pinned);
        let hits = s2.search("alp", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "alpha one");
        assert_eq!(hits[0].kind, Kind::Clipboard);
        match &hits[0].action {
            Action::RestoreClipboard { paste: true, .. } => {}
            other => panic!("{other:?}"),
        }

        // Pinned items survive when the list is full.
        let (_d2, mut full) = tmp();
        for i in 0..(MAX_ITEMS + 5) {
            assert!(full.record(text(&format!("item {i}"))));
        }
        assert_eq!(full.items.len(), MAX_ITEMS);
        let first_id = full.items.last().unwrap().id;
        full.pin(first_id, true);
        for i in 0..10 {
            assert!(full.record(text(&format!("newer {i}"))));
        }
        assert_eq!(full.items.len(), MAX_ITEMS);
        assert!(full.get(first_id).is_some_and(|i| i.pinned));

        full.delete(full.items[0].id);
        assert_eq!(full.items.len(), MAX_ITEMS - 1);
        full.clear();
        assert!(full.items.is_empty());
        assert!(full.search("", 10).is_empty());
    }

    #[test]
    fn files_and_images_get_readable_previews() {
        assert_eq!(preview_files("file:///home/a/Notes.txt\r\nfile:///tmp/b%20c.png\n"), "Notes.txt, b c.png");
        assert_eq!(preview_files("# comment\nfile:///one/a\nfile:///two/b\nfile:///three/c\n"), "a and 2 more");
        assert_eq!(preview_image("image/png", 12_400), "Image (PNG, 12 KB)");
        let (_d, mut s) = tmp();
        let added = s.record(Incoming {
            mimes: vec!["image/png".into()],
            mime: "image/png".into(),
            data: vec![0x89, 0x50, 0x4E, 0x47, 1, 2, 3, 4],
            source: Some("org.gnome.eog".into()),
        });
        assert!(added);
        let item = &s.items[0];
        assert_eq!(item.kind, ClipKind::Image);
        assert_eq!(s.blob(item).unwrap()[0], 0x89);
        assert!(item.preview.contains("PNG"));
    }

    #[test]
    fn config_roundtrip_and_app_matching() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("clipboard.toml");
        let c = Config { exclude: vec!["org.keepassxc.KeePassXC".into(), "Bank".into()], ..Default::default() };
        c.save(&p).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("enabled = true"));
        let loaded = Config::load(&p);
        assert!(loaded.excludes("org.keepassxc.KeePassXC"));
        assert!(loaded.excludes("keepassxc"));
        assert!(loaded.excludes("com.Bank"));
        assert!(!loaded.excludes("firefox"));
    }
}
