//! `iliada-notifyd`: Iliada's notification service (ADR 0010).
//!
//! It owns `org.freedesktop.Notifications` on the session bus, so every
//! notification an app sends comes here first. For each one it:
//!
//! 1. lists the app for Settings → Notifications the first time it sends one;
//! 2. decides, from the app's rules and Do Not Disturb
//!    (`iliada_notify::rules`), whether it pops up, plays a sound and is kept;
//! 3. keeps it in the Notification Center's history (not for private apps);
//! 4. hands the ones that pop up to COSMIC's banners (`cosmic-notifications`,
//!    which answers on [`BANNERS`] on Iliada), and passes clicks on them back
//!    to the app;
//! 5. plays a sound.
//!
//! The panel (Super+N) talks to it on `$XDG_RUNTIME_DIR/iliada-notifyd.sock`
//! (`iliada_notify::proto`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use iliada_notify::dnd::Clock;
use iliada_notify::history::History;
use iliada_notify::proto::{Request, Response};
use iliada_notify::rules::{self, Hints};
use iliada_notify::settings::Settings;
use iliada_notify::{Action, DEFAULT_ACTION, Notification, Urgency};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedVariant};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// COSMIC's banners: `cosmic-notifications`, patched on Iliada to answer
/// here instead of on `org.freedesktop.Notifications`
/// (nix/patches/cosmic-notifications-iliada.patch).
const BANNERS: &str = "org.iliada.NotificationBanners";

/// The freedesktop spec's reasons a notification closed.
const DISMISSED: u32 = 2;
const CLOSED_BY_CALL: u32 = 3;

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.iliada.NotificationBanners",
    default_path = "/org/freedesktop/Notifications"
)]
trait Banners {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[String],
        hints: &HashMap<String, OwnedVariant>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn activation_token(&self, id: u32, activation_token: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// A banner showing now.
#[derive(Debug, Clone, Copy)]
struct Banner {
    id: u32,
    /// Kept in the history: it stays there when the banner goes.
    kept: bool,
}

#[derive(Debug)]
struct State {
    history: History,
    history_path: Option<PathBuf>,
    settings: Settings,
    settings_path: Option<PathBuf>,
    settings_stamp: Option<SystemTime>,
    next_id: u32,
    /// Our id → its banner.
    banners: HashMap<u32, Banner>,
}

impl State {
    fn load() -> State {
        let history_path = iliada_notify::history_path();
        let history = history_path.as_deref().map(History::load).unwrap_or_default();
        let next_id = history.max_id().wrapping_add(1).max(1);
        let mut s = State {
            history,
            history_path,
            settings: Settings::default(),
            settings_path: iliada_notify::settings_path(),
            settings_stamp: None,
            next_id,
            banners: HashMap::new(),
        };
        s.reload_settings();
        s
    }

    /// Settings, the CLI and Homer write the settings file: read it again
    /// when it changed.
    fn reload_settings(&mut self) {
        let Some(path) = &self.settings_path else { return };
        let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if stamp != self.settings_stamp || stamp.is_none() {
            self.settings = Settings::load(path);
            self.settings_stamp = stamp;
        }
    }

    fn save_history(&self) {
        if let Some(path) = &self.history_path
            && let Err(e) = self.history.save(path)
        {
            tracing::warn!("could not save the notification history: {e:#}");
        }
    }

    fn save_settings(&mut self) {
        if let Some(path) = &self.settings_path {
            match self.settings.save(path) {
                Ok(()) => self.settings_stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok(),
                Err(e) => tracing::warn!("could not save the notification settings: {e:#}"),
            }
        }
    }

    fn new_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        id
    }

    fn ours(&self, banner: u32) -> Option<u32> {
        self.banners.iter().find(|(_, b)| b.id == banner).map(|(ours, _)| *ours)
    }
}

#[derive(Debug, Clone)]
struct Service {
    state: Arc<Mutex<State>>,
    /// Set once the connection is up.
    conn: Arc<std::sync::OnceLock<Connection>>,
}

fn hint_str(hints: &HashMap<String, OwnedVariant>, key: &str) -> Option<String> {
    hints.get(key).and_then(|v| <&str>::try_from(v).ok()).map(str::to_owned).filter(|s| !s.trim().is_empty())
}

fn hint_bool(hints: &HashMap<String, OwnedVariant>, key: &str) -> bool {
    hints.get(key).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

/// Action pairs (id, label) from the flat list the spec sends.
fn actions(flat: &[String]) -> Vec<Action> {
    flat.as_chunks::<2>().0.iter().map(|[id, label]| Action { id: id.clone(), label: label.clone() }).collect()
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Service {
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "body-markup", "icon-static", "persistence", "sound"].map(String::from).to_vec()
    }

    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    fn get_server_information(&self) -> (String, String, String, String) {
        ("iliada-notifyd".into(), "Iliada".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }

    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        expire_timeout: i32,
    ) -> u32 {
        self.receive(app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout).await
    }

    async fn close_notification(&self, id: u32) {
        self.remove(&[id], CLOSED_BY_CALL).await;
    }

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(emitter: &SignalEmitter<'_>, id: u32, activation_token: &str) -> zbus::Result<()>;
}

impl Service {
    fn emitter(&self) -> Option<SignalEmitter<'static>> {
        SignalEmitter::new(self.conn.get()?, PATH).ok()
    }

    async fn banners(&self) -> Option<BannersProxy<'static>> {
        BannersProxy::new(self.conn.get()?).await.ok()
    }

    #[allow(clippy::too_many_arguments)]
    async fn receive(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        flat_actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        expire_timeout: i32,
    ) -> u32 {
        let app_id = hint_str(&hints, "desktop-entry").map(|d| iliada_notify::desktop_id(&d)).unwrap_or_default();
        let icon = if app_icon.trim().is_empty() {
            hint_str(&hints, "image-path").or_else(|| hint_str(&hints, "image_path")).unwrap_or_default()
        } else {
            app_icon.clone()
        };
        let urgency =
            hints.get("urgency").and_then(|v| u8::try_from(v).ok()).map(Urgency::from_byte).unwrap_or_default();
        let flags =
            Hints { transient: hint_bool(&hints, "transient"), suppress_sound: hint_bool(&hints, "suppress-sound") };

        let (id, decision, replaces_banner, n) = {
            let mut st = self.state.lock().await;
            st.reload_settings();
            let id = if replaces_id != 0
                && (st.history.get(replaces_id).is_some() || st.banners.contains_key(&replaces_id))
            {
                replaces_id
            } else {
                st.new_id()
            };
            let n = Notification {
                id,
                app_id,
                app_name: app_name.clone(),
                icon,
                summary: summary.clone(),
                body: body.clone(),
                actions: actions(&flat_actions),
                urgency,
                time: iliada_notify::now(),
            };
            let key = n.app_key().to_owned();
            if st.settings.remember(&key, n.app_label(), &n.icon) {
                st.save_settings();
            }
            let decision = rules::decide(&st.settings, &Clock::now(), &n, flags);
            if !decision.allowed {
                return id;
            }
            if decision.keep {
                st.history.push(n.clone());
                st.save_history();
            } else if st.history.remove(id).is_some() {
                // Replaced by one that is not to be kept.
                st.save_history();
            }
            let replaces_banner = st.banners.get(&id).map(|b| b.id).unwrap_or(0);
            (id, decision, replaces_banner, n)
        };

        if decision.banner {
            match self.banners().await {
                Some(b) => {
                    match b
                        .notify(
                            &app_name,
                            replaces_banner,
                            &app_icon,
                            &summary,
                            &body,
                            &flat_actions,
                            &hints,
                            expire_timeout,
                        )
                        .await
                    {
                        Ok(banner) => {
                            self.state.lock().await.banners.insert(id, Banner { id: banner, kept: decision.keep });
                        }
                        Err(e) => tracing::warn!("could not show the banner: {e}"),
                    }
                }
                None => tracing::warn!("no connection for banners"),
            }
        } else if replaces_banner != 0 {
            // It no longer pops up (Do Not Disturb came on): take the old banner down.
            self.state.lock().await.banners.remove(&id);
            if let Some(b) = self.banners().await {
                let _ = b.close_notification(replaces_banner).await;
            }
        }
        if decision.sound {
            play_sound(&hints);
        }
        if !decision.keep && !decision.banner {
            // Nothing shows it and nothing keeps it: it is gone already.
            if let Some(e) = self.emitter() {
                let _ = Service::notification_closed(&e, id, DISMISSED).await;
            }
        }
        tracing::debug!(id, app = n.app_key(), ?decision, "notification");
        id
    }

    /// Take notifications out of the Notification Center (and off the
    /// screen), and tell their apps.
    async fn remove(&self, ids: &[u32], reason: u32) -> usize {
        let mut banners = Vec::new();
        let mut count = 0;
        {
            let mut st = self.state.lock().await;
            for id in ids {
                if st.history.remove(*id).is_some() {
                    count += 1;
                }
                if let Some(b) = st.banners.remove(id) {
                    banners.push(b.id);
                }
            }
            if count > 0 {
                st.save_history();
            }
        }
        if !banners.is_empty()
            && let Some(proxy) = self.banners().await
        {
            for b in banners {
                let _ = proxy.close_notification(b).await;
            }
        }
        if let Some(e) = self.emitter() {
            for id in ids {
                let _ = Service::notification_closed(&e, *id, reason).await;
            }
        }
        count
    }

    /// Clicked in the panel: run its action, then take it out.
    async fn activate(&self, id: u32, action: Option<String>) -> usize {
        let has_default = self.state.lock().await.history.get(id).is_some_and(Notification::has_default);
        let key = action.or_else(|| has_default.then(|| DEFAULT_ACTION.to_owned()));
        if let (Some(key), Some(e)) = (key, self.emitter()) {
            let _ = Service::action_invoked(&e, id, &key).await;
        }
        self.remove(&[id], DISMISSED).await
    }

    async fn handle(&self, request: Request) -> Response {
        match request {
            Request::List => {
                let st = self.state.lock().await;
                Response::Notifications { items: st.history.recent(iliada_notify::history::MAX) }
            }
            Request::Activate { id, action } => Response::Done { count: self.activate(id, action).await },
            Request::Dismiss { id } => Response::Done { count: self.remove(&[id], DISMISSED).await },
            Request::ClearApp { app } => {
                let ids: Vec<u32> = {
                    let st = self.state.lock().await;
                    st.history.items.iter().filter(|n| n.app_key() == app).map(|n| n.id).collect()
                };
                Response::Done { count: self.remove(&ids, DISMISSED).await }
            }
            Request::ClearAll => {
                let ids: Vec<u32> = self.state.lock().await.history.items.iter().map(|n| n.id).collect();
                Response::Done { count: self.remove(&ids, DISMISSED).await }
            }
        }
    }

    /// Clicks and closes on COSMIC's banners, passed back as ours.
    async fn follow_banners(self) -> Result<()> {
        let proxy = self.banners().await.context("no banner proxy")?;
        let mut tokens = proxy.receive_activation_token().await?;
        let mut invoked = proxy.receive_action_invoked().await?;
        let mut closed = proxy.receive_notification_closed().await?;
        loop {
            tokio::select! {
                Some(sig) = tokens.next() => {
                    let Ok(args) = sig.args() else { continue };
                    let ours = self.state.lock().await.ours(args.id);
                    if let (Some(id), Some(e)) = (ours, self.emitter()) {
                        let _ = Service::activation_token(&e, id, &args.activation_token).await;
                    }
                }
                Some(sig) = invoked.next() => {
                    let Ok(args) = sig.args() else { continue };
                    let ours = self.state.lock().await.ours(args.id);
                    if let Some(id) = ours {
                        if let Some(e) = self.emitter() {
                            let _ = Service::action_invoked(&e, id, &args.action_key).await;
                        }
                        // Opened from its banner: done with, as in the panel.
                        self.remove(&[id], DISMISSED).await;
                    }
                }
                Some(sig) = closed.next() => {
                    let Ok(args) = sig.args() else { continue };
                    let gone = {
                        let mut st = self.state.lock().await;
                        match st.ours(args.id) {
                            Some(id) => st.banners.remove(&id).map(|b| (id, b.kept)),
                            None => None,
                        }
                    };
                    // A banner that went away stays in the Notification
                    // Center; one that was not kept is gone for good.
                    if let (Some((id, false)), Some(e)) = (gone, self.emitter()) {
                        let _ = Service::notification_closed(&e, id, args.reason).await;
                    }
                }
                else => return Ok(()),
            }
        }
    }
}

/// The sound for a notification: its own (`sound-file`, `sound-name`), or
/// the default one.
fn sound_file(hints: &HashMap<String, OwnedVariant>) -> Option<PathBuf> {
    if let Some(f) = hint_str(hints, "sound-file").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_file()) {
        return Some(f);
    }
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/run/current-system/sw/share".into());
    if let Some(name) = hint_str(hints, "sound-name").filter(|n| !n.contains('/')) {
        for dir in dirs.split(':').filter(|d| !d.is_empty()) {
            for ext in ["oga", "ogg", "wav"] {
                let p = Path::new(dir).join(format!("sounds/freedesktop/stereo/{name}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    std::env::var_os("ILIADA_NOTIFICATION_SOUND").map(PathBuf::from).filter(|p| p.is_file())
}

fn play_sound(hints: &HashMap<String, OwnedVariant>) {
    let Some(file) = sound_file(hints) else { return };
    for player in ["pw-play", "paplay"] {
        if let Ok(mut child) = tokio::process::Command::new(player)
            .arg(&file)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            return;
        }
    }
}

/// COSMIC's own Do Not Disturb (from its notifications applet, which Iliada
/// replaces) would hide every banner, critical ones too: Iliada's is the
/// one that counts, so switch COSMIC's off.
fn clear_cosmic_dnd() {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    let Some(base) = base else { return };
    let path = base.join("cosmic/com.system76.CosmicNotifications/v1/do_not_disturb");
    if std::fs::read_to_string(&path).is_ok_and(|t| t.trim() == "true") {
        let _ = std::fs::write(&path, "false");
    }
}

async fn serve_socket(service: Service) -> Result<()> {
    let path = iliada_notify::socket_path().context("no XDG_RUNTIME_DIR")?;
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).with_context(|| format!("listening on {}", path.display()))?;
    loop {
        let (stream, _) = listener.accept().await?;
        let service = service.clone();
        tokio::spawn(async move {
            let (r, mut w) = stream.into_split();
            let mut line = String::new();
            if BufReader::new(r).read_line(&mut line).await.is_err() {
                return;
            }
            let reply = match serde_json::from_str::<Request>(&line) {
                Ok(req) => service.handle(req).await,
                Err(e) => Response::Error { message: format!("not a request: {e}") },
            };
            if let Ok(text) = serde_json::to_string(&reply) {
                let _ = w.write_all(format!("{text}\n").as_bytes()).await;
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "iliada_notifyd=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    clear_cosmic_dnd();
    let service = Service { state: Arc::new(Mutex::new(State::load())), conn: Arc::default() };
    let conn = zbus::connection::Builder::session()
        .serve_at(PATH, service.clone())
        .name(NAME)
        .build()
        .await
        .context("taking org.freedesktop.Notifications (is another notification service running?)")?;
    let _ = service.conn.set(conn);
    tracing::info!("serving {NAME}; banners through {BANNERS}");
    let banners = tokio::spawn({
        let s = service.clone();
        async move {
            // COSMIC's banners may come up after this service: try again.
            // The streams end only when the bus connection does: then stop,
            // and let systemd (or the bus) start the service again.
            loop {
                match s.clone().follow_banners().await {
                    Ok(()) => return,
                    Err(e) => tracing::debug!("following banners: {e:#}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    });
    let socket = serve_socket(service);
    tokio::select! {
        r = socket => r,
        _ = banners => {
            tracing::info!("the session bus went away; stopping");
            Ok(())
        }
    }
}
