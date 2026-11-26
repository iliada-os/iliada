//! Search extensions: results from a sandboxed bundle under a keyword
//! (ADR 0009).
//!
//! When the query starts with an extension's keyword and a space, the
//! overlay asks for its results ([`crate::Request::ExtensionQuery`]). The
//! search service keeps the extension running while the overlay is open and
//! talks to it over its stdin and stdout, one JSON object per line:
//!
//! ```text
//! → {"query":"ripgrep","id":7}
//! ← {"id":7,"results":[{"title":"BurntSushi/ripgrep","subtitle":"…","icon":"Resources/repo.png",
//!                       "action":{"url":"https://github.com/BurntSushi/ripgrep"}}]}
//! ```
//!
//! An action is one of `{"copy": text}`, `{"url": "https://…"}` or
//! `{"ask_homer": text}`. Nothing else: an extension can never run a
//! command, open a file or paste. Everything is checked here, and anything
//! that does not fit is left out.
//!
//! Keystrokes are debounced ([`DEBOUNCE`]); a newer query makes older ones
//! stale (their replies are dropped); no reply within [`TIMEOUT`] means no
//! results. Replies are capped at [`MAX_RESULTS`] results and [`MAX_REPLY`]
//! bytes. The process is stopped when the overlay closes, or after
//! [`IDLE`] without a query.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub use iliada_bundle::search::Extension;
use serde_json::Value;

use crate::{Action, Hit, Icon, Kind};

/// Wait this long after a keystroke before asking (typing on cancels it).
pub const DEBOUNCE: Duration = Duration::from_millis(120);
/// No reply by then: "No results from …".
pub const TIMEOUT: Duration = Duration::from_millis(1500);
/// Stopped after this long without a query.
pub const IDLE: Duration = Duration::from_secs(30);
pub const MAX_RESULTS: usize = 20;
/// The longest reply line, in bytes.
pub const MAX_REPLY: usize = 64 * 1024;

const MAX_TITLE: usize = 200;
const MAX_SUBTITLE: usize = 300;
const MAX_COPY: usize = 10_000;
const MAX_URL: usize = 2048;
const MAX_ASK: usize = 2000;

/// The line sent for a query.
pub fn request_line(query: &str, id: u64) -> String {
    let query = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".into());
    format!("{{\"query\":{query},\"id\":{id}}}\n")
}

/// Why a reply was refused as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadReply {
    TooLong,
    NotJson,
    /// No `id`, or no `results` list.
    Shape,
}

/// Check one reply line and turn its results into rows. `contents` is the
/// extension's `Contents/` folder (icons must be inside it); `title` labels
/// its group. Results that do not fit are left out; at most
/// [`MAX_RESULTS`] are kept.
pub fn parse_reply(line: &[u8], contents: &Path, title: &str) -> Result<(u64, Vec<Hit>), BadReply> {
    if line.len() > MAX_REPLY {
        return Err(BadReply::TooLong);
    }
    let v: Value = serde_json::from_slice(line).map_err(|_| BadReply::NotJson)?;
    let id = v.get("id").and_then(Value::as_u64).ok_or(BadReply::Shape)?;
    let results = v.get("results").and_then(Value::as_array).ok_or(BadReply::Shape)?;
    let hits = results.iter().take(MAX_RESULTS).filter_map(|r| result_hit(r, contents, title)).collect();
    Ok((id, hits))
}

/// Text that fits: not empty, no longer than `max` characters, no control
/// characters (one line).
fn text(v: &Value, max: usize) -> Option<String> {
    let s = v.as_str()?.trim();
    (!s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)).then(|| s.to_owned())
}

/// An `https://` address with a host, no spaces or control characters.
pub fn safe_url(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    s.len() <= MAX_URL
        && !host.is_empty()
        && !host.contains('@')
        && !s.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\')
}

fn action(v: &Value) -> Option<Action> {
    let obj = v.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let (kind, value) = obj.iter().next()?;
    match kind.as_str() {
        // Copied text may have several lines.
        "copy" => {
            let s = value.as_str()?;
            (!s.is_empty() && s.chars().count() <= MAX_COPY && !s.contains('\0')).then(|| Action::Copy(s.to_owned()))
        }
        "url" => value.as_str().filter(|u| safe_url(u)).map(|u| Action::Url(u.to_owned())),
        "ask_homer" => text(value, MAX_ASK).map(Action::AskHomer),
        _ => None,
    }
}

/// An icon file inside the bundle's `Contents/` (no way out of it, even
/// through a symbolic link), PNG or SVG.
fn icon(rel: &str, contents: &Path) -> Option<Icon> {
    let ok_name = !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains('\0')
        && rel.split('/').all(|c| !c.is_empty() && c != "..");
    let ext = Path::new(rel).extension()?.to_str()?.to_ascii_lowercase();
    if !ok_name || !matches!(ext.as_str(), "png" | "svg") {
        return None;
    }
    let base = contents.canonicalize().ok()?;
    let path = base.join(rel).canonicalize().ok()?;
    (path.starts_with(&base) && path.is_file()).then_some(Icon::Path(path))
}

fn result_hit(r: &Value, contents: &Path, group: &str) -> Option<Hit> {
    let obj = r.as_object()?;
    if obj.keys().any(|k| !matches!(k.as_str(), "title" | "subtitle" | "icon" | "action")) {
        return None;
    }
    let title = text(obj.get("title")?, MAX_TITLE)?;
    let subtitle = match obj.get("subtitle") {
        None | Some(Value::Null) => String::new(),
        Some(s) => text(s, MAX_SUBTITLE)?,
    };
    let action = action(obj.get("action")?)?;
    let mut h = Hit::new(Kind::Extension, title, action).subtitle(subtitle);
    h.icon = obj
        .get("icon")
        .and_then(Value::as_str)
        .and_then(|i| icon(i, contents))
        .or_else(|| Some(Icon::Name("system-search-symbolic".into())));
    // The group's label; never remembered (no key).
    h.source = group.to_owned();
    Some(h)
}

/// What a query got.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Results(Vec<Hit>),
    /// Nothing, a refused reply, no reply in time, or the extension ended.
    NoResults,
    /// A newer query came in: this one no longer matters.
    Stale,
}

#[derive(Debug, Default)]
struct State {
    /// The newest query's id: replies to any other are dropped.
    latest: u64,
    /// Replies to `latest` (`None`: refused).
    replies: HashMap<u64, Option<Vec<Hit>>>,
    /// The process ended (or its pipe broke).
    dead: bool,
}

/// One running extension.
#[derive(Debug)]
struct Proc {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    state: Mutex<State>,
    cond: Condvar,
    last_used: Mutex<Instant>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Read one line of at most `max` bytes into `buf`. `None` at the end;
/// `Some(true)` when the line was longer (the rest of it is skipped).
fn read_line_capped(r: &mut impl BufRead, max: usize, buf: &mut Vec<u8>) -> std::io::Result<Option<bool>> {
    use std::io::Read;
    buf.clear();
    let n = r.by_ref().take(max as u64 + 1).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        return Ok(Some(false));
    }
    if buf.len() <= max {
        return Ok(Some(false)); // the last line, without a newline
    }
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        if let Some(i) = chunk.iter().position(|b| *b == b'\n') {
            r.consume(i + 1);
            break;
        }
        let len = chunk.len();
        r.consume(len);
    }
    Ok(Some(true))
}

impl Proc {
    fn start(mut cmd: Command, contents: PathBuf, title: String) -> std::io::Result<Arc<Proc>> {
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("no pipe to the extension"));
        };
        let p = Arc::new(Proc {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            state: Mutex::new(State::default()),
            cond: Condvar::new(),
            last_used: Mutex::new(Instant::now()),
        });
        let reader = Arc::downgrade(&p);
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                let line = read_line_capped(&mut r, MAX_REPLY, &mut buf);
                let Some(p) = reader.upgrade() else { return };
                let mut st = lock(&p.state);
                match line {
                    Ok(Some(false)) => match parse_reply(&buf, &contents, &title) {
                        Ok((id, hits)) if id == st.latest => {
                            st.replies.insert(id, Some(hits));
                        }
                        Ok(_) => {} // a stale reply
                        Err(_) => {
                            let id = st.latest;
                            st.replies.insert(id, None);
                        }
                    },
                    Ok(Some(true)) => {
                        let id = st.latest;
                        st.replies.insert(id, None);
                    }
                    Ok(None) | Err(_) => {
                        st.dead = true;
                        p.cond.notify_all();
                        return;
                    }
                }
                p.cond.notify_all();
            }
        });
        Ok(p)
    }

    fn dead(&self) -> bool {
        lock(&self.state).dead
    }

    fn query(&self, query: &str, debounce: Duration, timeout: Duration) -> Outcome {
        let id = {
            let mut st = lock(&self.state);
            st.latest += 1;
            st.replies.clear();
            st.latest
        };
        *lock(&self.last_used) = Instant::now();
        std::thread::sleep(debounce);
        {
            let st = lock(&self.state);
            if st.latest != id {
                return Outcome::Stale;
            }
            if st.dead {
                return Outcome::NoResults;
            }
        }
        let sent = {
            let mut stdin = lock(&self.stdin);
            stdin.write_all(request_line(query, id).as_bytes()).and_then(|()| stdin.flush())
        };
        if sent.is_err() {
            lock(&self.state).dead = true;
            return Outcome::NoResults;
        }
        let deadline = Instant::now() + timeout;
        let mut st = lock(&self.state);
        loop {
            if let Some(reply) = st.replies.remove(&id) {
                return match reply {
                    Some(hits) if !hits.is_empty() => Outcome::Results(hits),
                    _ => Outcome::NoResults,
                };
            }
            if st.latest != id {
                return Outcome::Stale;
            }
            if st.dead {
                return Outcome::NoResults;
            }
            let now = Instant::now();
            if now >= deadline {
                return Outcome::NoResults;
            }
            st = self.cond.wait_timeout(st, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    fn idle_for(&self) -> Duration {
        lock(&self.last_used).elapsed()
    }

    fn stop(&self) {
        let mut child = lock(&self.child);
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub type Launcher = Box<dyn Fn(&Extension) -> Command + Send + Sync>;

/// The running extensions, by id.
pub struct Host {
    procs: Mutex<HashMap<String, Arc<Proc>>>,
    launch: Launcher,
    pub debounce: Duration,
    pub timeout: Duration,
    pub idle: Duration,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host").field("running", &lock(&self.procs).keys().collect::<Vec<_>>()).finish()
    }
}

impl Default for Host {
    fn default() -> Self {
        Host::new()
    }
}

impl Host {
    /// Extensions start in their sandbox: `iliada-run --search-extension`.
    /// The `iliada-run` next to this program is preferred, since the
    /// Search service's PATH need not have it.
    pub fn new() -> Self {
        let run = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join("iliada-run")))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("iliada-run"));
        Host::with_launcher(Box::new(move |e: &Extension| {
            let mut c = Command::new(&run);
            c.arg("--search-extension").arg(&e.bundle);
            c
        }))
    }

    /// For tests: start extensions with another command.
    pub fn with_launcher(launch: Launcher) -> Self {
        Host { procs: Mutex::new(HashMap::new()), launch, debounce: DEBOUNCE, timeout: TIMEOUT, idle: IDLE }
    }

    /// The extension's results for `query`, starting it if it is not
    /// running (or ended).
    pub fn query(&self, ext: &Extension, query: &str) -> Outcome {
        let proc = {
            let mut procs = lock(&self.procs);
            match procs.get(&ext.id).filter(|p| !p.dead()) {
                Some(p) => p.clone(),
                None => {
                    let contents = ext.bundle.join("Contents");
                    match Proc::start((self.launch)(ext), contents, ext.title.clone()) {
                        Ok(p) => {
                            procs.insert(ext.id.clone(), p.clone());
                            p
                        }
                        Err(e) => {
                            tracing::warn!(extension = %ext.id, "could not start the Search extension: {e}");
                            return Outcome::NoResults;
                        }
                    }
                }
            }
        };
        proc.query(query, self.debounce, self.timeout)
    }

    /// Whether this extension is running now.
    pub fn running(&self, id: &str) -> bool {
        lock(&self.procs).get(id).is_some_and(|p| !p.dead())
    }

    /// Stop every extension (the overlay closed).
    pub fn stop_all(&self) {
        let procs: Vec<Arc<Proc>> = lock(&self.procs).drain().map(|(_, p)| p).collect();
        for p in procs {
            p.stop();
        }
    }

    /// Stop extensions without a query for [`Host::idle`], and those no
    /// longer in `active` (turned off, removed, or lost their keyword).
    pub fn stop_idle(&self, active: &[&str]) {
        let stale: Vec<Arc<Proc>> = {
            let mut procs = lock(&self.procs);
            let gone: Vec<String> = procs
                .iter()
                .filter(|(id, p)| p.dead() || p.idle_for() >= self.idle || !active.contains(&id.as_str()))
                .map(|(id, _)| id.clone())
                .collect();
            gone.iter().filter_map(|id| procs.remove(id)).collect()
        };
        for p in stale {
            p.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(results: &str) -> Vec<u8> {
        format!(r#"{{"id":3,"results":[{results}]}}"#).into_bytes()
    }

    fn titles(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.title.as_str()).collect()
    }

    #[test]
    fn requests_are_one_json_line() {
        assert_eq!(request_line("a \"b\"\nc", 7), "{\"query\":\"a \\\"b\\\"\\nc\",\"id\":7}\n");
    }

    #[test]
    fn only_the_three_actions_are_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let line = reply(
            r##"{"title":"Copy","subtitle":"hex","action":{"copy":"#ff0000"}},
               {"title":"Open","action":{"url":"https://github.com/BurntSushi/ripgrep"}},
               {"title":"Ask","action":{"ask_homer":"What is ripgrep?"}},
               {"title":"Run","action":{"run":"rm -rf ~"}},
               {"title":"Open a file","action":{"open":"/etc/passwd"}},
               {"title":"Paste","action":{"paste":"x"}},
               {"title":"Two","action":{"copy":"a","url":"https://a.example"}},
               {"title":"No action"},
               {"title":"Extra key","action":{"copy":"a"},"command":"x"},
               {"title":"","action":{"copy":"a"}},
               {"title":"Empty copy","action":{"copy":""}}"##,
        );
        let (id, hits) = parse_reply(&line, dir.path(), "Color").unwrap();
        assert_eq!(id, 3);
        assert_eq!(titles(&hits), ["Copy", "Open", "Ask"]);
        assert_eq!(hits[0].action, Action::Copy("#ff0000".into()));
        assert_eq!(hits[0].subtitle, "hex");
        assert!(hits.iter().all(|h| h.kind == Kind::Extension && h.source == "Color" && h.key.is_empty()));
    }

    #[test]
    fn only_https_addresses() {
        for ok in ["https://github.com", "https://example.org/a?b=c#d"] {
            assert!(safe_url(ok), "{ok}");
        }
        for bad in [
            "http://example.org",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://",
            "https:///path",
            "https://exa mple.org",
            "https://user@example.org",
            "https://example.org\\@evil",
            "HTTPS://example.org",
            "ftp://example.org",
        ] {
            assert!(!safe_url(bad), "{bad}");
        }
        let line = reply(r#"{"title":"x","action":{"url":"http://example.org"}}"#);
        assert!(parse_reply(&line, Path::new("/nonexistent"), "X").unwrap().1.is_empty());
    }

    #[test]
    fn replies_are_capped() {
        let one = r#"{"title":"r","action":{"copy":"x"}}"#;
        let many = vec![one; 30].join(",");
        let (_, hits) = parse_reply(&reply(&many), Path::new("/nonexistent"), "X").unwrap();
        assert_eq!(hits.len(), MAX_RESULTS);
        let huge = format!(r#"{{"title":"r","action":{{"copy":"{}"}}}}"#, "a".repeat(MAX_REPLY));
        assert_eq!(parse_reply(&reply(&huge), Path::new("/"), "X"), Err(BadReply::TooLong));
        assert_eq!(parse_reply(b"not json", Path::new("/"), "X"), Err(BadReply::NotJson));
        assert_eq!(parse_reply(br#"{"results":[]}"#, Path::new("/"), "X"), Err(BadReply::Shape));
        assert_eq!(parse_reply(br#"{"id":1,"results":{}}"#, Path::new("/"), "X"), Err(BadReply::Shape));
    }

    #[test]
    fn icons_stay_inside_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let contents = dir.path().join("X.app/Contents");
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        std::fs::write(contents.join("Resources/repo.png"), b"png").unwrap();
        std::fs::write(dir.path().join("secret.png"), b"png").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.png"), contents.join("Resources/link.png")).unwrap();
        let hit = |icon: &str| {
            let line = reply(&format!(r#"{{"title":"t","icon":{icon:?},"action":{{"copy":"x"}}}}"#));
            parse_reply(&line, &contents, "X").unwrap().1.remove(0).icon
        };
        let fallback = Some(Icon::Name("system-search-symbolic".into()));
        assert_eq!(
            hit("Resources/repo.png"),
            Some(Icon::Path(contents.join("Resources/repo.png").canonicalize().unwrap()))
        );
        for bad in ["../../secret.png", "/etc/passwd.png", "Resources/link.png", "Resources/missing.png", "Info.toml"] {
            assert_eq!(hit(bad), fallback, "{bad}");
        }
    }

    #[test]
    fn long_lines_are_skipped_whole() {
        let mut input = std::io::Cursor::new(format!("{}\nnext\nlast", "x".repeat(100)).into_bytes());
        let mut buf = Vec::new();
        assert_eq!(read_line_capped(&mut input, 10, &mut buf).unwrap(), Some(true));
        assert_eq!(read_line_capped(&mut input, 10, &mut buf).unwrap(), Some(false));
        assert_eq!(buf, b"next");
        assert_eq!(read_line_capped(&mut input, 10, &mut buf).unwrap(), Some(false));
        assert_eq!(buf, b"last");
        assert_eq!(read_line_capped(&mut input, 10, &mut buf).unwrap(), None);
    }

    /// A pretend extension: answers each query with one result titled by
    /// the query; waits first when the query has "slow" in it; never
    /// answers "silent"; answers "huge" with a reply over the limit; ends
    /// on "quit".
    const FAKE: &str = r#"
        while IFS= read -r line; do
            id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
            q=$(printf '%s' "$line" | sed -n 's/.*"query":"\([^"]*\)".*/\1/p')
            case "$q" in
                *quit*) exit 0 ;;
                *silent*) continue ;;
                *huge*) head -c 70000 /dev/zero | tr '\0' 'x'; echo; continue ;;
                *slow*) sleep 0.4 ;;
            esac
            printf '{"id":%s,"results":[{"title":"%s","action":{"copy":"%s"}}]}\n' "$id" "$q" "$q"
        done
    "#;

    fn fake_host(debounce_ms: u64, timeout_ms: u64) -> Host {
        let mut h = Host::with_launcher(Box::new(|_| {
            let mut c = Command::new("sh");
            c.arg("-c").arg(FAKE);
            c
        }));
        h.debounce = Duration::from_millis(debounce_ms);
        h.timeout = Duration::from_millis(timeout_ms);
        h.idle = Duration::from_millis(200);
        h
    }

    fn fake_ext() -> Extension {
        Extension {
            id: "org.example.fake".into(),
            title: "Fake".into(),
            placeholder: None,
            keyword: "fake".into(),
            declared_keyword: "fake".into(),
            enabled: true,
            network: false,
            clashes_with: None,
            bundle: "/nonexistent/Fake.app".into(),
        }
    }

    fn results(o: Outcome) -> Vec<String> {
        match o {
            Outcome::Results(h) => h.into_iter().map(|h| h.title).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn answers_and_keeps_running() {
        let (host, ext) = (fake_host(20, 1000), fake_ext());
        assert_eq!(results(host.query(&ext, "red")), ["red"]);
        assert!(host.running(&ext.id));
        assert_eq!(results(host.query(&ext, "blue")), ["blue"]);
        host.stop_all();
        assert!(!host.running(&ext.id));
    }

    #[test]
    fn no_reply_in_time_means_no_results() {
        let (host, ext) = (fake_host(20, 250), fake_ext());
        let t = Instant::now();
        assert_eq!(host.query(&ext, "silent"), Outcome::NoResults);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert_eq!(host.query(&ext, "huge"), Outcome::NoResults, "over 64 KB");
        // A late reply to an old query is not taken for a new one's.
        assert_eq!(host.query(&ext, "slow one"), Outcome::NoResults);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(results(host.query(&ext, "fresh")), ["fresh"]);
    }

    #[test]
    fn newer_queries_make_older_ones_stale() {
        let host = Arc::new(fake_host(150, 2000));
        let ext = fake_ext();
        // Typed on during the debounce: the first is never sent.
        let (h, e) = (host.clone(), ext.clone());
        let first = std::thread::spawn(move || h.query(&e, "gree"));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(results(host.query(&ext, "green")), ["green"]);
        assert_eq!(first.join().unwrap(), Outcome::Stale);
        // Typed on while waiting for a slow reply: that reply is dropped.
        let (h, e) = (host.clone(), ext.clone());
        let slow = std::thread::spawn(move || h.query(&e, "slow"));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(results(host.query(&ext, "quick")), ["quick"]);
        assert_eq!(slow.join().unwrap(), Outcome::Stale);
    }

    #[test]
    fn ended_extensions_start_again_and_idle_ones_stop() {
        let (host, ext) = (fake_host(20, 1000), fake_ext());
        assert_eq!(host.query(&ext, "quit"), Outcome::NoResults);
        assert_eq!(results(host.query(&ext, "again")), ["again"]);
        host.stop_idle(&[ext.id.as_str()]);
        assert!(host.running(&ext.id), "used just now");
        std::thread::sleep(Duration::from_millis(250));
        host.stop_idle(&[ext.id.as_str()]);
        assert!(!host.running(&ext.id), "idle");
        assert_eq!(results(host.query(&ext, "back")), ["back"]);
        host.stop_idle(&[]);
        assert!(!host.running(&ext.id), "turned off");
    }
}
