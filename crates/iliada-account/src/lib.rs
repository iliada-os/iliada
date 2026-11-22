//! The computer side of the Iliada account (see `docs/account.md`).
//!
//! * **Signing in**: the computer shows a code; the person approves it on the
//!   account website (with a passkey or an email code) and the computer gets
//!   its own token. Linux has no passkeys outside web browsers, so this is
//!   how a computer signs in.
//! * **The account key** ([`key`]): made on the first computer and never sent
//!   to the server; a new computer gets it from a computer already signed in
//!   ([`ask_for_key`], [`hand_over_key`]) or from the recovery key.
//! * **Backups** ([`snapshot`]): the desktop settings, Privacy answers and
//!   assistant conversations, sealed with the account key before they leave.
//!
//! Each person signs in for themselves: everything lives in their home
//! folder, readable only by them (`~/.config/iliada/account.json` and
//! `account-key`).

pub mod key;
pub mod server;
pub mod snapshot;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub use key::AccountKey;
pub use server::{DeviceCode, Server};
pub use snapshot::{Manifest, PARTS};

/// What proves a computer holds the account key: this text, sealed.
pub const KEY_CHECK: &str = "key-check";
const KEY_CHECK_TEXT: &[u8] = b"Iliada account key";
/// The latest backup of a person's settings.
pub const SETTINGS: &str = "settings";
/// This computer's apps and command-line tools ([`ComputerSetup`]).
pub const COMPUTER: &str = "computer";
/// Where Iliada keeps a computer's configuration.
pub const SYSTEM_DIR: &str = "/etc/iliada";

/// The server this computer uses, if Iliada was set up with one
/// (`ILIADA_ACCOUNT_SERVER`, set by `iliada.account.server`).
pub fn default_server() -> Option<String> {
    std::env::var("ILIADA_ACCOUNT_SERVER").ok().filter(|s| !s.trim().is_empty())
}

/// This computer's sign-in, as saved.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub server: String,
    pub email: String,
    pub token: String,
    /// The name the website shows for this computer.
    pub computer: String,
    pub last_backup: Option<u64>,
    pub last_error: Option<String>,
    /// Of the files last backed up: skip the upload when nothing changed.
    pub last_digest: Option<String>,
    /// Of the last computer setup backed up (its apps and tools).
    #[serde(default)]
    pub last_computer_digest: Option<String>,
}

/// Where a person's sign-in and key are kept.
#[derive(Debug, Clone)]
pub struct Files {
    pub home: PathBuf,
    pub state: PathBuf,
    pub key: PathBuf,
    /// This computer's configuration (`/etc/iliada`): its apps and
    /// command-line tools are backed up from there.
    pub system: PathBuf,
}

impl Files {
    /// The current user's.
    pub fn user() -> Result<Files> {
        let home = PathBuf::from(std::env::var_os("HOME").context("no home folder")?);
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        Ok(Files::at(&home, &config.join("iliada")))
    }

    pub fn at(home: &Path, dir: &Path) -> Files {
        Files {
            home: home.to_owned(),
            state: dir.join("account.json"),
            key: dir.join("account-key"),
            system: PathBuf::from(SYSTEM_DIR),
        }
    }

    pub fn load(&self) -> Option<State> {
        serde_json::from_slice(&std::fs::read(&self.state).ok()?).ok()
    }

    fn save(&self, s: &State) -> Result<()> {
        write_private(&self.state, &serde_json::to_vec_pretty(s)?)
    }

    pub fn account_key(&self) -> Option<AccountKey> {
        AccountKey::from_recovery_text(&std::fs::read_to_string(&self.key).ok()?).ok()
    }

    fn save_key(&self, k: &AccountKey) -> Result<()> {
        write_private(&self.key, format!("{}\n", k.recovery_text()).as_bytes())
    }

    /// Forgets the sign-in and the key.
    fn forget(&self) -> Result<()> {
        for p in [&self.state, &self.key] {
            match std::fs::remove_file(p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }

    fn signed_in(&self) -> Result<(State, Server)> {
        let s = self.load().context("this computer isn't signed in to an Iliada account")?;
        let server = Server::new(&s.server, Some(s.token.clone()));
        Ok((s, server))
    }

    fn ready(&self) -> Result<(State, Server, AccountKey)> {
        let (s, server) = self.signed_in()?;
        let key = self.account_key().context("this computer needs your recovery key first")?;
        Ok((s, server, key))
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(path.parent().expect("has a parent"))?;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    SignedOut,
    /// Signed in, but this computer doesn't have the account key yet.
    NeedsKey {
        email: String,
    },
    Ready {
        email: String,
        last_backup: Option<u64>,
        last_error: Option<String>,
    },
}

pub fn status(files: &Files) -> Status {
    match (files.load(), files.account_key()) {
        (None, _) => Status::SignedOut,
        (Some(s), None) => Status::NeedsKey { email: s.email },
        (Some(s), Some(_)) => Status::Ready { email: s.email, last_backup: s.last_backup, last_error: s.last_error },
    }
}

/// This computer's name, as the website lists it.
pub fn computer_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Iliada computer".into())
}

/// The first half of signing in: a code to show the person.
pub async fn start_sign_in(server: &str) -> Result<DeviceCode> {
    Server::new(server, None).device_code(&computer_name()).await
}

/// What the account key needs after signing in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyStep {
    /// The account's first computer: a new key, to be shown to the person
    /// once as their recovery key.
    Created(String),
    /// The account has a key this computer doesn't: ask for the recovery key
    /// ([`use_recovery_key`]).
    Needed,
    /// This computer already had it.
    Ready,
}

/// The second half: waits for the person to approve the code, then sets up
/// the account key.
pub async fn finish_sign_in(files: &Files, server: &str, code: &DeviceCode) -> Result<(String, KeyStep)> {
    let token = Server::new(server, None).wait_for_token(code).await?;
    let api = Server::new(server, Some(token.clone()));
    let email = api.account().await?.email;
    // A different account than before: its key isn't this one.
    if files.load().is_some_and(|old| old.email != email || old.server != server.trim_end_matches('/')) {
        files.forget()?;
    }
    files.save(&State {
        server: server.trim_end_matches('/').to_owned(),
        email: email.clone(),
        token,
        computer: computer_name(),
        ..Default::default()
    })?;
    let step = match api.get(KEY_CHECK).await? {
        None => {
            let key = AccountKey::generate()?;
            api.put(KEY_CHECK, key.seal(KEY_CHECK, KEY_CHECK_TEXT)?).await?;
            files.save_key(&key)?;
            KeyStep::Created(key.recovery_text())
        }
        Some(sealed) => match files.account_key() {
            Some(k) if k.open(KEY_CHECK, &sealed).is_ok() => KeyStep::Ready,
            _ => {
                let _ = std::fs::remove_file(&files.key);
                KeyStep::Needed
            }
        },
    };
    Ok((email, step))
}

/// Takes the account key from the recovery key the person typed.
pub async fn use_recovery_key(files: &Files, text: &str) -> Result<()> {
    let key = AccountKey::from_recovery_text(text)?;
    let (_, api) = files.signed_in()?;
    let sealed = api.get(KEY_CHECK).await?.context("this account has no key yet; sign in again")?;
    if key.open(KEY_CHECK, &sealed).is_err() {
        bail!("That isn't this account's recovery key. Check it and try again.");
    }
    files.save_key(&key)
}

/// A new computer asking the account's other computers for the key.
#[derive(Debug, Clone)]
pub struct KeyRequest {
    pub id: String,
    /// The code this computer shows; the other one shows the same.
    pub code: String,
    receiver: key::Receiver,
}

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Asks a computer already signed in to hand over the account key (instead
/// of typing the recovery key). Then wait with [`wait_for_key`].
pub async fn ask_for_key(files: &Files) -> Result<KeyRequest> {
    use base64::Engine;
    let (_, api) = files.signed_in()?;
    let receiver = key::Receiver::new()?;
    let public = receiver.public_key();
    let id = api.ask_for_key(&B64.encode(public)).await?;
    Ok(KeyRequest { id, code: key::match_code(&public), receiver })
}

/// Waits (up to 15 minutes) until another computer hands over the key,
/// checks it is this account's, and keeps it.
pub async fn wait_for_key(files: &Files, request: &KeyRequest) -> Result<()> {
    use base64::Engine;
    let (_, api) = files.signed_in()?;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        match api.key_request(&request.id).await? {
            None => bail!("No computer answered in time. Try again, or use your recovery key."),
            Some((state, _)) if state == "waiting" => {}
            Some((state, _)) if state == "refused" => bail!("The other computer said no."),
            Some((_, sealed)) => {
                let sealed = B64.decode(sealed.unwrap_or_default().trim())?;
                let key = request.receiver.open(&sealed)?;
                let check = api.get(KEY_CHECK).await?.context("this account has no key yet; sign in again")?;
                if key.open(KEY_CHECK, &check).is_err() {
                    bail!("The other computer sent a key that isn't this account's.");
                }
                return files.save_key(&key);
            }
        }
    }
}

/// Another computer of this account waiting for the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub id: String,
    pub computer: String,
    /// The code that computer shows.
    pub code: String,
    public_key: [u8; 32],
}

/// The account's other computers waiting for the key (none unless this
/// computer has it).
pub async fn waiting_for_key(files: &Files) -> Result<Vec<Waiting>> {
    use base64::Engine;
    let Ok((_, api, _)) = files.ready() else { return Ok(Vec::new()) };
    Ok(api
        .key_requests()
        .await?
        .into_iter()
        .filter_map(|r| {
            let public_key: [u8; 32] = B64.decode(r.public_key.trim()).ok()?.try_into().ok()?;
            Some(Waiting { id: r.id, computer: r.computer, code: key::match_code(&public_key), public_key })
        })
        .collect())
}

/// Hands the account key to a waiting computer (after the person saw the
/// same code on both screens).
pub async fn hand_over_key(files: &Files, to: &Waiting) -> Result<()> {
    use base64::Engine;
    let (_, api, key) = files.ready()?;
    api.approve_key_request(&to.id, &B64.encode(key.seal_for(&to.public_key)?)).await
}

/// Says no to a waiting computer.
pub async fn refuse_key(files: &Files, to: &Waiting) -> Result<()> {
    let (_, api) = files.signed_in()?;
    api.refuse_key_request(&to.id).await
}

/// The recovery key, to show the person again.
pub fn recovery_key(files: &Files) -> Option<String> {
    files.account_key().map(|k| k.recovery_text())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// What a backup did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backup {
    Uploaded { files: usize, bytes: usize },
    Unchanged,
}

/// Backs up this person's settings unless nothing changed (or `force`).
pub async fn backup(files: &Files, force: bool) -> Result<Backup> {
    let (mut s, api, key) = files.ready()?;
    let result = async {
        let manifest =
            Manifest { computer: s.computer.clone(), created: now(), iliada: env!("CARGO_PKG_VERSION").into() };
        let home = files.home.clone();
        let packed = tokio::task::spawn_blocking(move || snapshot::pack(&home, &manifest)).await??;
        let mut backup = Backup::Unchanged;
        if force || s.last_digest.as_deref() != Some(packed.digest.as_str()) {
            let sealed = key.seal(SETTINGS, &packed.bytes)?;
            let bytes = sealed.len();
            api.put(SETTINGS, sealed).await?;
            backup = Backup::Uploaded { files: packed.files, bytes };
        }
        // The computer's apps and tools, when they changed (a computer
        // without /etc/iliada, a test VM, has none).
        let mut computer_digest = s.last_computer_digest.clone();
        if let Some(setup) = ComputerSetup::read(&files.system, &s.computer, now()) {
            let digest = setup.digest();
            if force || computer_digest.as_deref() != Some(digest.as_str()) {
                api.put(COMPUTER, key.seal(COMPUTER, &serde_json::to_vec(&setup)?)?).await?;
                computer_digest = Some(digest);
                if backup == Backup::Unchanged {
                    backup = Backup::Uploaded { files: 2, bytes: setup.settings.len() + setup.apps.len() };
                }
            }
        }
        Ok::<_, anyhow::Error>((backup, packed.digest, computer_digest))
    }
    .await;
    match result {
        Ok((b, digest, computer_digest)) => {
            if b != Backup::Unchanged {
                s.last_backup = Some(now());
            }
            s.last_digest = Some(digest);
            s.last_computer_digest = computer_digest;
            s.last_error = None;
            files.save(&s)?;
            Ok(b)
        }
        Err(e) => {
            s.last_error = Some(format!("{e:#}"));
            files.save(&s)?;
            Err(e)
        }
    }
}

async fn latest_bytes(api: &Server, key: &AccountKey) -> Result<Option<Vec<u8>>> {
    match api.get(SETTINGS).await? {
        Some(sealed) => Ok(Some(key.open(SETTINGS, &sealed)?)),
        None => Ok(None),
    }
}

/// A computer's apps and command-line tools, as the account keeps them:
/// its `/etc/iliada/settings.toml` and `apps.toml`. Putting them on another
/// computer goes through the system service (it asks for the
/// administrator's password and rebuilds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerSetup {
    /// The computer it came from.
    pub computer: String,
    /// Seconds since 1970.
    pub created: u64,
    pub settings: String,
    pub apps: String,
}

impl ComputerSetup {
    /// This computer's, from `dir` (`None` if it has no configuration there).
    pub fn read(dir: &Path, computer: &str, created: u64) -> Option<ComputerSetup> {
        let settings = std::fs::read_to_string(dir.join("settings.toml")).ok()?;
        let apps = std::fs::read_to_string(dir.join("apps.toml")).unwrap_or_default();
        Some(ComputerSetup { computer: computer.to_owned(), created, settings, apps })
    }

    /// Of the contents only (not where or when): unchanged files, same digest.
    fn digest(&self) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::new().chain_update(&self.settings).chain_update([0]).chain_update(&self.apps).finalize())
    }

    /// The same apps and tools as this computer has now.
    pub fn matches(&self, dir: &Path) -> bool {
        ComputerSetup::read(dir, "", 0).is_some_and(|here| here.digest() == self.digest())
    }
}

/// The apps and tools the account keeps from its computers (the latest).
pub async fn computer_setup(files: &Files) -> Result<Option<ComputerSetup>> {
    let (_, api, key) = files.ready()?;
    match api.get(COMPUTER).await? {
        Some(sealed) => Ok(Some(serde_json::from_slice(&key.open(COMPUTER, &sealed)?)?)),
        None => Ok(None),
    }
}

/// Where the account's backup came from, if it has one.
pub async fn latest(files: &Files) -> Result<Option<Manifest>> {
    let (_, api, key) = files.ready()?;
    latest_bytes(&api, &key).await?.map(|b| snapshot::manifest(&b)).transpose()
}

/// Puts the account's backup in place. Returns where it came from and how
/// many files were written.
pub async fn restore(files: &Files) -> Result<(Manifest, usize)> {
    let (mut s, api, key) = files.ready()?;
    let bytes = latest_bytes(&api, &key).await?.context("your account has no backup yet")?;
    let home = files.home.clone();
    let (manifest, n, digest) = tokio::task::spawn_blocking(move || -> Result<_> {
        let (m, n) = snapshot::unpack(&home, &bytes)?;
        // What is here now: the next backup only uploads real changes.
        let digest = snapshot::pack(&home, &m)?.digest;
        Ok((m, n, digest))
    })
    .await??;
    s.last_digest = Some(digest);
    files.save(&s)?;
    Ok((manifest, n))
}

/// Signs this computer out: the server forgets its token, and the computer
/// forgets the sign-in and the account key.
pub async fn sign_out(files: &Files) -> Result<()> {
    if let Ok((_, api)) = files.signed_in() {
        // Signed out here even when the server can't be reached; the website
        // can still remove the computer.
        let _ = api.sign_out().await;
    }
    files.forget()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_computer_setup_is_its_settings_and_apps() {
        let d = tempfile::tempdir().unwrap();
        assert!(ComputerSetup::read(d.path(), "pc", 1).is_none(), "no configuration, nothing to keep");
        std::fs::write(d.path().join("settings.toml"), "tools = [\"python3\"]\n").unwrap();
        std::fs::write(d.path().join("apps.toml"), "[apps.vlc]\n").unwrap();
        let a = ComputerSetup::read(d.path(), "pc", 1).unwrap();
        assert_eq!((a.computer.as_str(), a.apps.as_str()), ("pc", "[apps.vlc]\n"));
        assert!(a.matches(d.path()));
        // Where and when don't matter; contents do.
        assert_eq!(a.digest(), ComputerSetup::read(d.path(), "laptop", 9).unwrap().digest());
        std::fs::write(d.path().join("apps.toml"), "[apps.gimp]\n").unwrap();
        assert!(!a.matches(d.path()));
    }

    #[test]
    fn status_follows_the_files() {
        let t = tempfile::tempdir().unwrap();
        let f = Files::at(t.path(), &t.path().join(".config/iliada"));
        assert_eq!(status(&f), Status::SignedOut);
        f.save(&State { email: "ana@example.com".into(), ..Default::default() }).unwrap();
        assert_eq!(status(&f), Status::NeedsKey { email: "ana@example.com".into() });
        f.save_key(&AccountKey::generate().unwrap()).unwrap();
        assert!(matches!(status(&f), Status::Ready { .. }));
        use std::os::unix::fs::PermissionsExt;
        for p in [&f.state, &f.key] {
            assert_eq!(std::fs::metadata(p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        f.forget().unwrap();
        assert_eq!(status(&f), Status::SignedOut);
    }
}
