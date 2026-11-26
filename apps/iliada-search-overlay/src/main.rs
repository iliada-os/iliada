//! Iliada's search overlay: Spotlight for Iliada.
//!
//! Started with the session and kept running, hidden, so the Super key
//! only has to show it: `iliada-search` (what the key runs) writes one
//! line to this program's socket and the panel appears, with no process to
//! start and nothing to load. Each keystroke asks `iliada-searchd`, which
//! holds the index in memory, for results (a few milliseconds even with a
//! million files).
//!
//! * Super: search apps, files and folders, Settings, agora, answers, and
//!   open windows (choosing one switches to it; `w ` lists only windows).
//! * Super+O: ask Homer.
//! * Super+V: clipboard history (the last 200 copies).
//! * Space: Quick Look preview of the selected result.
//! * Drag a file out of the results, or drop a file on the panel for
//!   Open With, Move to, Copy Path, or Ask Homer.
//!
//! Keys: ↑/↓ choose, Enter opens, Tab (or →) shows what else a file
//! offers (Open With, Reveal in Files, Copy Path, Move to), Ctrl+Enter
//! reveals a file in Files, Esc closes.
//!
//! It is a layer-shell surface (above every window, like COSMIC's own
//! launcher) covering the screen, transparent except for the panel; a
//! click outside the panel closes it.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cosmic::app::{Settings, Task};
use cosmic::iced::clipboard::mime::AsMimeTypes;
use cosmic::iced::keyboard::{Key, Modifiers, key::Named};
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::{IcedOutput, SctkLayerSurfaceSettings};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, Layer, destroy_layer_surface, get_layer_surface,
};
use cosmic::iced::{Alignment, Length, Subscription, window};
use cosmic::widget;
use cosmic::{Core, Element, executor};
use iliada_search::proto::{Client, Request};
use iliada_search::{Action, Hit, Icon, Kind, actions};

mod i18n;

/// Rows shown at most.
const ROWS: usize = 9;
const WIDTH: f32 = 680.0;
const PREVIEW_WIDTH: f32 = 360.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Search,
    Homer,
    Clipboard,
}

impl Mode {
    fn parse(s: &str) -> Option<Mode> {
        match s.trim() {
            "toggle" | "search" => Some(Mode::Search),
            "homer" => Some(Mode::Homer),
            "clipboard" => Some(Mode::Clipboard),
            _ => None,
        }
    }
}

fn socket_path() -> Option<PathBuf> {
    iliada_search::runtime_dir().map(|d| d.join("iliada-search-overlay.sock"))
}

/// The socket this program listens on, claimed before anything else so a
/// second start only passes its request on.
static LISTENER: std::sync::OnceLock<std::os::unix::net::UnixListener> = std::sync::OnceLock::new();

/// `false`: one runs already (and was told `mode`).
fn claim(mode: Option<Mode>) -> bool {
    use std::io::Write;
    let Some(path) = socket_path() else { return true };
    for _ in 0..2 {
        match std::os::unix::net::UnixListener::bind(&path) {
            Ok(l) => {
                let _ = l.set_nonblocking(true);
                let _ = LISTENER.set(l);
                return true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if let Ok(mut s) = std::os::unix::net::UnixStream::connect(&path) {
                    if let Some(m) = mode {
                        let _ = writeln!(
                            s,
                            "{}",
                            match m {
                                Mode::Search => "toggle",
                                Mode::Homer => "homer",
                                Mode::Clipboard => "clipboard",
                            }
                        );
                    }
                    return false;
                }
                let _ = std::fs::remove_file(&path);
            }
            Err(_) => return true,
        }
    }
    true
}

fn main() -> cosmic::iced::Result {
    let mut args = std::env::args().skip(1);
    let mut show = None;
    while let Some(a) = args.next() {
        if a == "--show" {
            show = Some(args.next().as_deref().and_then(Mode::parse).unwrap_or_default());
        }
    }
    if !claim(show) {
        return Ok(());
    }
    let settings = Settings::default().no_main_window(true).exit_on_close(false).transparent(true);
    cosmic::app::run::<App>(settings, show)
}

/// Requests from `iliada-search` (one line each: toggle, homer, clipboard).
fn requests() -> impl cosmic::iced::futures::Stream<Item = Message> {
    use cosmic::iced::futures::stream;
    use tokio::io::AsyncBufReadExt;
    stream::unfold(None::<tokio::net::UnixListener>, |listener| async move {
        let listener = match listener {
            Some(l) => l,
            None => match LISTENER.get().and_then(|l| l.try_clone().ok()).map(tokio::net::UnixListener::from_std) {
                Some(Ok(l)) => l,
                _ => std::future::pending().await,
            },
        };
        let mut mode = Mode::Search;
        if let Ok((s, _)) = listener.accept().await {
            let mut line = String::new();
            let mut r = tokio::io::BufReader::new(s);
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), r.read_line(&mut line)).await;
            mode = Mode::parse(&line).unwrap_or_default();
        }
        Some((Message::Toggle(mode), Some(listener)))
    })
}

#[derive(Debug, Clone)]
enum Message {
    Toggle(Mode),
    Input(String),
    Up,
    Down,
    /// Enter: the chosen row (or this one, clicked).
    Activate(Option<usize>),
    /// Ctrl+Enter: reveal a file in Files.
    Reveal,
    /// Tab / →: what else the chosen file offers; Esc / ← goes back.
    Extras,
    Back,
    Hide,
    HomerOn(bool),
    /// After a restore, paste into the previous window (the overlay is gone).
    Paste,
    /// Space: open or close Quick Look.
    TogglePreview,
    /// A preview finished loading; ignore it if `gen` is stale.
    PreviewReady(u64, Option<iliada_search::preview::Preview>),
    /// Files (or text) dropped onto the panel.
    Dropped(String, Vec<u8>),
    Nothing,
}

struct App {
    core: Core,
    surface: Option<window::Id>,
    mode: Mode,
    query: String,
    hits: Vec<Hit>,
    /// Showing a file's extra actions: the file's row, to go back to.
    extras_of: Option<(Vec<Hit>, usize)>,
    selected: usize,
    /// Whether Homer is on (a key is set), as last known.
    homer: bool,
    /// How long the last search took in the service, for the footer.
    micros: u64,
    client: Arc<Mutex<Option<Client>>>,
    field: widget::Id,
    note: Option<String>,
    /// Quick Look is open (Space). The picture or text may still be loading.
    preview_open: bool,
    preview: Option<iliada_search::preview::Preview>,
    preview_loading: bool,
    preview_stamp: u64,
    /// Space was used to toggle the preview: drop the space the field would add.
    ignore_space: bool,
    /// Files just dropped onto the panel, if we are showing their actions.
    dropped: Vec<PathBuf>,
    /// Files waiting for a folder, after "Move to".
    moving: Vec<PathBuf>,
}

impl App {
    /// Ask the service. A few milliseconds: done right here, so the rows
    /// always match what is typed.
    fn call(&self, req: &Request) -> Option<iliada_search::Response> {
        let mut guard = self.client.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..2 {
            if guard.is_none() {
                *guard = Client::connect().ok();
            }
            let c = guard.as_mut()?;
            match c.call(req) {
                Ok(r) => return Some(r),
                Err(_) => *guard = None,
            }
        }
        None
    }

    fn refresh(&mut self) {
        self.extras_of = None;
        self.selected = 0;
        self.note = None;
        self.preview = None;
        self.preview_loading = self.preview_open;
        match self.mode {
            Mode::Homer => {
                self.hits = if self.homer {
                    vec![iliada_search::engine::homer_hit(self.query.trim())]
                } else {
                    vec![homer_off()]
                };
            }
            Mode::Clipboard => match self.call(&Request::ClipboardConfig) {
                Some(iliada_search::Response::ClipboardConfig { enabled: false, .. }) => {
                    self.hits = vec![clipboard_off()];
                    self.note = None;
                }
                _ => {
                    let req = Request::Clipboard { query: self.query.clone(), limit: ROWS };
                    match self.call(&req) {
                        Some(iliada_search::Response::Hits { hits, micros }) => {
                            self.hits = hits;
                            self.micros = micros;
                            self.note = if self.hits.is_empty() && self.query.is_empty() {
                                Some(fl!("clipboard-empty-note"))
                            } else {
                                None
                            };
                            for h in &mut self.hits {
                                h.subtitle = clipboard_subtitle(h);
                            }
                        }
                        _ => {
                            self.hits = Vec::new();
                            self.note = Some(fl!("clipboard-unavailable"));
                        }
                    }
                }
            },
            Mode::Search => {
                let req = Request::Search { query: self.query.clone(), homer: self.homer, limit: ROWS };
                match self.call(&req) {
                    Some(iliada_search::Response::Hits { hits, micros }) => {
                        self.hits = hits;
                        self.micros = micros;
                        if self.hits.is_empty()
                            && let Some(words) = self.query.trim_start().strip_prefix("w ")
                        {
                            self.note = Some(windows_note(words.trim()));
                        }
                    }
                    _ => {
                        self.hits = Vec::new();
                        self.note = Some(fl!("search-starting"));
                    }
                }
            }
        }
    }

    fn show(&mut self, mode: Mode) -> Task<Message> {
        self.mode = mode;
        self.query.clear();
        self.hits.clear();
        self.preview_open = false;
        self.preview = None;
        self.refresh();
        let id = window::Id::unique();
        self.surface = Some(id);
        let surface = get_layer_surface(SctkLayerSurfaceSettings {
            id,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            output: IcedOutput::Active,
            namespace: "iliada-search".into(),
            size: Some((None, None)),
            exclusive_zone: -1,
            ..Default::default()
        });
        let mut tasks = vec![surface, widget::text_input::focus(self.field.clone())];
        // Homer may have been turned on or off since.
        tasks.push(cosmic::task::future(async {
            let on = iliada_system::client::status().await.map(|s| s.assistant.enabled).unwrap_or(false);
            Message::HomerOn(on)
        }));
        Task::batch(tasks)
    }

    fn hide(&mut self) -> Task<Message> {
        self.preview_open = false;
        self.preview = None;
        self.dropped.clear();
        self.moving.clear();
        match self.surface.take() {
            Some(id) => destroy_layer_surface(id),
            None => Task::none(),
        }
    }

    fn activate(&mut self, hit: Hit) -> Task<Message> {
        if let Action::MoveTo(src) = &hit.action {
            self.moving = if self.dropped.contains(src) && self.dropped.len() > 1 {
                self.dropped.clone()
            } else {
                vec![src.clone()]
            };
            self.extras_of = None;
            self.query.clear();
            let name = self
                .moving
                .iter()
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(", ");
            self.note = Some(fl!("move-to-hint", name = name.as_str()));
            self.refresh();
            return Task::none();
        }
        if !self.moving.is_empty()
            && let Some(dest) = hit.path().filter(|p| p.is_dir()).map(|p| p.to_owned())
        {
            return self.finish_move(dest);
        }
        if !hit.key.is_empty() {
            let _ = self.call(&Request::Used { hit: hit.clone() });
        }
        match &hit.action {
            Action::ClipboardPin { id, pinned } => {
                let _ = self.call(&Request::ClipboardPin { id: *id, pinned: *pinned });
                self.refresh();
                Task::none()
            }
            Action::ClipboardDelete(id) => {
                let _ = self.call(&Request::ClipboardDelete { id: *id });
                self.refresh();
                Task::none()
            }
            Action::ClipboardClear => {
                let _ = self.call(&Request::ClipboardClear);
                self.refresh();
                Task::none()
            }
            Action::RestoreClipboard { id, paste } => match self.call(&Request::ClipboardRestore { id: *id }) {
                Some(iliada_search::Response::Error { message }) => {
                    self.note = Some(message);
                    Task::none()
                }
                Some(iliada_search::Response::Done) | Some(iliada_search::Response::Hits { .. }) => {
                    let hide = self.hide();
                    if *paste {
                        Task::batch([
                            hide,
                            cosmic::task::future(async {
                                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                                Message::Paste
                            }),
                        ])
                    } else {
                        hide
                    }
                }
                _ => {
                    self.note = Some(fl!("clipboard-unavailable"));
                    Task::none()
                }
            },
            Action::Copy(text) => {
                let hide = self.hide();
                Task::batch([cosmic::iced::clipboard::write(text.clone()), hide])
            }
            // Only the search service knows the open windows: it switches.
            Action::FocusWindow { identifier } => {
                match self.call(&Request::FocusWindow { identifier: identifier.clone() }) {
                    Some(iliada_search::Response::Done) => self.hide(),
                    Some(iliada_search::Response::Error { message }) => {
                        self.note = Some(fl!("window-focus-failed", error = message));
                        Task::none()
                    }
                    _ => {
                        self.note = Some(fl!("search-starting"));
                        Task::none()
                    }
                }
            }
            other => {
                if let Err(e) = actions::perform(other) {
                    self.note = Some(format!("{e:#}"));
                    Task::none()
                } else {
                    self.hide()
                }
            }
        }
    }

    fn finish_move(&mut self, dest: PathBuf) -> Task<Message> {
        let srcs = std::mem::take(&mut self.moving);
        let mut last = dest.clone();
        for src in &srcs {
            match actions::move_into(src, &dest) {
                Ok(p) => last = p.parent().unwrap_or(&dest).to_owned(),
                Err(e) => {
                    self.note = Some(fl!("move-failed", error = format!("{e:#}")));
                    self.moving = srcs;
                    return Task::none();
                }
            }
        }
        self.dropped.clear();
        self.note = Some(fl!("moved-to", folder = crate::tilde_or_display(&last)));
        self.refresh();
        Task::none()
    }

    fn show_dropped(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p.exists()).collect();
        if paths.is_empty() {
            return Task::none();
        }
        self.mode = Mode::Search;
        self.dropped = paths.clone();
        self.moving.clear();
        self.extras_of = None;
        let mut hits = Vec::new();
        if paths.len() > 1 {
            let n = paths.len();
            let joined = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
            hits.push(
                Hit::new(Kind::Action, fl!("open-n-files", n = n), Action::OpenAll(paths.clone()))
                    .subtitle(fl!("dropped-files", n = n))
                    .icon_name("document-open-symbolic"),
            );
            hits.push(
                Hit::new(Kind::Action, "Copy Path", Action::Copy(joined.clone())).icon_name("edit-copy-symbolic"),
            );
            hits.push(
                Hit::new(Kind::Action, "Move to", Action::MoveTo(paths[0].clone()))
                    .subtitle("Choose a folder")
                    .icon_name("folder-symbolic"),
            );
            hits.push(
                Hit::new(Kind::Action, "Ask Homer About It", Action::AskHomer(format!("About these files: {joined}")))
                    .icon_name("org.iliada.Assistant"),
            );
        }
        if let Some(iliada_search::Response::Hits { hits: more, .. }) =
            self.call(&Request::OpenWith { path: paths[0].clone() })
        {
            if paths.len() == 1 {
                hits = more;
            } else {
                for h in more {
                    if !hits.iter().any(|x| x.title == h.title) {
                        hits.push(h);
                    }
                }
            }
        } else if paths.len() == 1 {
            hits = actions::extras(&iliada_search::engine::file_hit(&paths[0], paths[0].is_dir()));
        }
        if hits.is_empty() {
            return Task::none();
        }
        let before = std::mem::replace(&mut self.hits, hits);
        self.extras_of = Some((before, self.selected));
        self.selected = 0;
        self.note = Some(if paths.len() == 1 {
            fl!("dropped-file", name = paths[0].file_name().and_then(|n| n.to_str()).unwrap_or("file"))
        } else {
            fl!("dropped-files", n = paths.len())
        });
        Task::none()
    }

    fn request_preview(&mut self) -> Task<Message> {
        if !self.preview_open {
            return Task::none();
        }
        let Some(hit) = self.hits.get(self.selected).cloned() else {
            self.preview = None;
            self.preview_loading = false;
            return Task::none();
        };
        self.preview_stamp = self.preview_stamp.wrapping_add(1);
        let stamp = self.preview_stamp;
        self.preview_loading = true;
        let client = self.client.clone();
        cosmic::task::future(async move {
            let preview = tokio::task::spawn_blocking(move || fetch_preview(client, hit)).await.ok().flatten();
            Message::PreviewReady(stamp, preview)
        })
    }

    fn row<'a>(&self, i: usize, hit: &'a Hit) -> Element<'a, Message> {
        let spacing = cosmic::theme::spacing();
        let icon: Element<_> = match &hit.icon {
            Some(Icon::Path(p)) => widget::icon(widget::icon::from_path(p.clone())).size(32).into(),
            Some(Icon::Name(n)) => widget::icon::from_name(n.as_str()).size(32).icon().into(),
            None => widget::icon::from_name("system-search-symbolic").size(32).icon().into(),
        };
        let mut text = widget::column::with_capacity(2).push(widget::text::body(hit.title.as_str()));
        if !hit.subtitle.is_empty() {
            text = text.push(widget::text::caption(hit.subtitle.as_str()));
        }
        let mut row = widget::row::with_capacity(4)
            .push(icon)
            .push(text.width(Length::Fill))
            .spacing(spacing.space_s)
            .align_y(Alignment::Center);
        let more = hit.path().is_some() || (self.mode == Mode::Clipboard && hit.clip_id != 0);
        if i == self.selected && more && self.extras_of.is_none() {
            row = row.push(widget::text::caption(fl!("tab-for-more")));
        }
        row = row.push(widget::text::caption(i18n::kind_label(hit.kind)));
        let selected = i == self.selected;
        let content = widget::container(row)
            .padding([spacing.space_xxs, spacing.space_s])
            .width(Length::Fill)
            .class(cosmic::theme::Container::custom(move |t| row_style(t, selected)));
        let row: Element<'_, Message> = widget::mouse_area(content).on_press(Message::Activate(Some(i))).into();
        if let Some(payload) = drag_payload(hit) {
            let window = self.surface.unwrap_or(window::Id::RESERVED);
            widget::dnd_source(row).drag_content(move || payload.clone()).window(window).into()
        } else {
            row
        }
    }

    fn panel(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let (icon, placeholder) = match self.mode {
            Mode::Search => ("system-search-symbolic", fl!("search-placeholder")),
            Mode::Homer => ("org.iliada.Assistant", fl!("homer-placeholder")),
            Mode::Clipboard => ("edit-paste-symbolic", fl!("clipboard-placeholder")),
        };
        let field = widget::text_input(placeholder, &self.query)
            .id(self.field.clone())
            .on_input(Message::Input)
            .on_submit(|_| Message::Activate(None))
            .size(22)
            .padding([spacing.space_s, spacing.space_s])
            .width(Length::Fill);
        let top = widget::row::with_capacity(2)
            .push(widget::icon::from_name(icon).size(28).icon())
            .push(field)
            .spacing(spacing.space_s)
            .align_y(Alignment::Center);
        let mut col = widget::column::with_capacity(ROWS + 3).push(top).spacing(spacing.space_xxs);
        if let Some(n) = &self.note {
            col = col.push(widget::text::caption(n.as_str()));
        }
        if !self.hits.is_empty() {
            col = col.push(widget::divider::horizontal::light());
        }
        for (i, h) in self.hits.iter().enumerate().take(ROWS) {
            col = col.push(self.row(i, h));
        }
        let hint = if !self.moving.is_empty() {
            fl!("hint-move-to")
        } else {
            match (&self.extras_of, self.mode) {
                (Some(_), _) => fl!("hint-extras"),
                (None, Mode::Homer) => fl!("hint-homer"),
                (None, Mode::Clipboard) => fl!("hint-clipboard"),
                (None, Mode::Search) if self.query.is_empty() => fl!("hint-search-empty"),
                (None, Mode::Search) => fl!("hint-search", ms = format!("{:.1}", self.micros as f64 / 1000.0)),
            }
        };
        col = col.push(widget::text::caption(if self.preview_open { fl!("preview-close") } else { hint }));
        let list = col.padding(spacing.space_s).width(Length::Fixed(WIDTH));
        let body: Element<_> = if self.preview_open {
            widget::row::with_capacity(2)
                .push(list)
                .push(widget::divider::vertical::light())
                .push(self.preview_pane())
                .width(Length::Fixed(WIDTH + PREVIEW_WIDTH + 8.0))
                .into()
        } else {
            list.into()
        };
        let panel = widget::container(body).class(cosmic::theme::Container::custom(frosted));
        let panel = widget::dnd_destination(panel, vec![Cow::from("text/uri-list"), Cow::from("text/plain")])
            .on_finish(|mime, data, _action, _x, _y| Message::Dropped(mime, data));
        // The panel sits in the top third; a click anywhere else closes.
        let page = widget::column::with_capacity(3)
            .push(widget::space::vertical().height(Length::FillPortion(1)))
            .push(widget::mouse_area(panel).on_press(Message::Nothing))
            .push(widget::space::vertical().height(Length::FillPortion(3)))
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fill);
        widget::mouse_area(page).on_press(Message::Hide).into()
    }

    fn preview_pane(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let content: Element<_> = if self.preview_loading && self.preview.is_none() {
            widget::text::caption(fl!("preview-loading")).into()
        } else if let Some(p) = &self.preview {
            preview_content(p)
        } else {
            widget::text::caption(fl!("preview-loading")).into()
        };
        widget::container(widget::scrollable(content).height(Length::Fixed(420.0)))
            .padding(spacing.space_s)
            .width(Length::Fixed(PREVIEW_WIDTH))
            .into()
    }
}

fn tilde_or_display(path: &Path) -> String {
    iliada_search::tilde(path)
}

fn typed_folder(query: &str) -> Option<PathBuf> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let path = if let Some(rest) = q.strip_prefix("~/") {
        iliada_search::home()?.join(rest)
    } else if q == "~" {
        iliada_search::home()?
    } else {
        PathBuf::from(q)
    };
    path.is_dir().then_some(path)
}

#[derive(Clone)]
enum DragPayload {
    Files(Vec<PathBuf>),
    Text(String),
}

fn drag_payload(hit: &Hit) -> Option<DragPayload> {
    if let Some(p) = hit.path() {
        return Some(DragPayload::Files(vec![p.to_owned()]));
    }
    match &hit.action {
        Action::Copy(t) | Action::AskHomer(t) => Some(DragPayload::Text(t.clone())),
        Action::Url(u) => Some(DragPayload::Text(u.clone())),
        _ if hit.kind == Kind::Clipboard && !hit.title.is_empty() => Some(DragPayload::Text(hit.title.clone())),
        _ => None,
    }
}

impl AsMimeTypes for DragPayload {
    fn available(&self) -> Cow<'static, [String]> {
        match self {
            Self::Files(_) => Cow::Owned(vec!["text/uri-list".into(), "text/plain".into()]),
            Self::Text(_) => Cow::Owned(vec!["text/plain".into()]),
        }
    }

    fn as_bytes(&self, mime: &str) -> Option<Cow<'static, [u8]>> {
        match self {
            Self::Files(paths) => {
                let uris = actions::uri_list(paths);
                let plain = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
                if mime == "text/uri-list" {
                    Some(Cow::Owned(uris.into_bytes()))
                } else if mime == "text/plain" {
                    Some(Cow::Owned(plain.into_bytes()))
                } else {
                    None
                }
            }
            Self::Text(t) if mime == "text/plain" => Some(Cow::Owned(t.as_bytes().to_vec())),
            Self::Text(_) => None,
        }
    }
}

fn homer_off() -> Hit {
    Hit::new(
        Kind::Homer,
        fl!("homer-off-title"),
        Action::Run(vec!["cosmic-settings".into(), "iliada".into(), "homer".into()]),
    )
    .subtitle(fl!("homer-off-subtitle"))
    .icon_name("org.iliada.Assistant")
}

fn clipboard_off() -> Hit {
    Hit::new(
        Kind::Clipboard,
        fl!("clipboard-off-title"),
        Action::Run(vec!["cosmic-settings".into(), "iliada".into(), "privacy".into()]),
    )
    .subtitle(fl!("clipboard-off-subtitle"))
    .icon_name("edit-paste-symbolic")
}

/// Why `w ` shows nothing: windows are turned off, none are open, or none
/// match.
fn windows_note(words: &str) -> String {
    let on = iliada_search::windows::Config::default_path()
        .map(|p| iliada_search::windows::Config::load(&p).windows)
        .unwrap_or(true);
    if !on {
        fl!("windows-off")
    } else if words.is_empty() {
        fl!("windows-none")
    } else {
        fl!("windows-no-match")
    }
}

fn clipboard_subtitle(hit: &Hit) -> String {
    let mut parts = Vec::new();
    if hit.pinned {
        parts.push(fl!("pinned"));
    }
    let ago = i18n::ago(hit.at);
    if !ago.is_empty() {
        parts.push(ago);
    }
    if !hit.source.is_empty() {
        parts.push(fl!("from-app", app = hit.source.as_str()));
    }
    parts.join(", ")
}

fn fetch_preview(client: Arc<Mutex<Option<Client>>>, hit: Hit) -> Option<iliada_search::preview::Preview> {
    if hit.clip_id != 0 {
        let mut guard = client.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..2 {
            if guard.is_none() {
                *guard = Client::connect().ok();
            }
            if let Some(c) = guard.as_mut() {
                match c.call(&Request::PreviewClipboard { id: hit.clip_id }) {
                    Ok(iliada_search::Response::Preview { preview }) => return Some(preview),
                    Ok(iliada_search::Response::Error { .. }) | Err(_) => *guard = None,
                    Ok(_) => return None,
                }
            }
        }
        return None;
    }
    if let Some(path) = hit.path() {
        let mut guard = client.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..2 {
            if guard.is_none() {
                *guard = Client::connect().ok();
            }
            if let Some(c) = guard.as_mut() {
                match c.call(&Request::PreviewPath { path: path.to_owned() }) {
                    Ok(iliada_search::Response::Preview { preview }) => return Some(preview),
                    Err(_) => *guard = None,
                    Ok(_) => break,
                }
            }
        }
        return Some(iliada_search::preview::for_path(path));
    }
    Some(iliada_search::preview::for_hit(&hit))
}

fn preview_content(p: &iliada_search::preview::Preview) -> Element<'_, Message> {
    use iliada_search::preview::PreviewKind;
    let spacing = cosmic::theme::spacing();
    let mut col = widget::column::with_capacity(8).spacing(spacing.space_xxs);
    col = col.push(widget::text::body(p.title.as_str()));
    if (p.kind == PreviewKind::Image || p.kind == PreviewKind::Pdf)
        && let Some(path) = &p.image
    {
        col = col.push(widget::image(widget::image::Handle::from_path(path.clone())).width(Length::Fill));
    }
    if !p.lines.is_empty() {
        for line in &p.lines {
            col = col.push(widget::text::caption(line.as_str()).font(cosmic::font::mono()));
        }
    }
    if !p.text.is_empty() {
        if !p.language.is_empty() && (p.language != "txt") {
            col = col.push(highlighted_text(&p.text, &p.language));
        } else {
            col = col.push(widget::text::body(p.text.as_str()).font(cosmic::font::mono()).size(13));
        }
    }
    col.into()
}

fn highlighted_text<'a>(text: &'a str, _language: &'a str) -> Element<'a, Message> {
    let mut col = widget::column::with_capacity(text.lines().count().max(1));
    for line in text.split_inclusive('\n') {
        col = col.push(widget::text::body(line).font(cosmic::font::mono()).size(13));
    }
    col.into()
}

fn frosted(theme: &cosmic::Theme) -> widget::container::Style {
    let c = theme.cosmic();
    let bg: cosmic::iced::Color = c.background(theme.transparent).base.into();
    widget::container::Style {
        background: Some(cosmic::iced::Background::Color(bg)),
        border: cosmic::iced::Border { radius: c.radius_m().into(), width: 1.0, color: c.bg_divider().into() },
        shadow: cosmic::iced::Shadow {
            color: cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: cosmic::iced::Vector::new(0.0, 8.0),
            blur_radius: 32.0,
        },
        text_color: Some(c.on_bg_color().into()),
        icon_color: Some(c.on_bg_color().into()),
        ..Default::default()
    }
}

fn row_style(theme: &cosmic::Theme, selected: bool) -> widget::container::Style {
    let c = theme.cosmic();
    let mut bg: cosmic::iced::Color = c.accent_color().into();
    bg.a = 0.28;
    widget::container::Style {
        background: selected.then_some(cosmic::iced::Background::Color(bg)),
        border: cosmic::iced::Border { radius: c.radius_s().into(), ..Default::default() },
        text_color: Some(c.on_bg_color().into()),
        icon_color: Some(c.on_bg_color().into()),
        ..Default::default()
    }
}

impl cosmic::Application for App {
    type Executor = executor::Default;
    type Flags = Option<Mode>;
    type Message = Message;
    const APP_ID: &'static str = "org.iliada.Search";

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, show: Option<Mode>) -> (Self, Task<Message>) {
        let mut app = App {
            core,
            surface: None,
            mode: Mode::Search,
            query: String::new(),
            hits: Vec::new(),
            extras_of: None,
            selected: 0,
            homer: false,
            micros: 0,
            client: Arc::new(Mutex::new(Client::connect().ok())),
            field: widget::Id::unique(),
            note: None,
            preview_open: false,
            preview: None,
            preview_loading: false,
            preview_stamp: 0,
            ignore_space: false,
            dropped: Vec::new(),
            moving: Vec::new(),
        };
        let homer = cosmic::task::future(async {
            Message::HomerOn(iliada_system::client::status().await.map(|s| s.assistant.enabled).unwrap_or(false))
        });
        let shown = match show {
            Some(m) => app.show(m),
            None => Task::none(),
        };
        (app, Task::batch([homer, shown]))
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = cosmic::iced::event::listen_with(|event, _status, _window| match event {
            cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                match key {
                    Key::Named(Named::Escape) => Some(Message::Back),
                    Key::Named(Named::ArrowUp) => Some(Message::Up),
                    Key::Named(Named::ArrowDown) => Some(Message::Down),
                    Key::Named(Named::Tab) => Some(Message::Extras),
                    Key::Named(Named::Enter) if modifiers.contains(Modifiers::CTRL) => Some(Message::Reveal),
                    Key::Character(c) if c == " " && modifiers.is_empty() => Some(Message::TogglePreview),
                    _ => None,
                }
            }
            _ => None,
        });
        Subscription::batch([Subscription::run(requests), keys])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Toggle(mode) => {
                // The same key again closes; another key switches mode.
                if self.surface.is_some() {
                    if self.mode == mode {
                        return self.hide();
                    }
                    self.mode = mode;
                    self.refresh();
                    return widget::text_input::focus(self.field.clone());
                }
                return self.show(mode);
            }
            Message::Input(q) => {
                let q = if self.ignore_space && q.len() == self.query.len() + 1 && q.ends_with(' ') {
                    self.ignore_space = false;
                    self.query.clone()
                } else {
                    self.ignore_space = false;
                    q
                };
                if q != self.query {
                    self.query = q;
                    self.refresh();
                    return self.request_preview();
                }
            }
            Message::Up => {
                self.selected = self.selected.saturating_sub(1);
                return self.request_preview();
            }
            Message::Down => {
                if self.selected + 1 < self.hits.len().min(ROWS) {
                    self.selected += 1;
                    return self.request_preview();
                }
            }
            Message::Activate(i) => {
                if i.is_none()
                    && !self.moving.is_empty()
                    && let Some(dest) = typed_folder(&self.query)
                {
                    return self.finish_move(dest);
                }
                let i = i.unwrap_or(self.selected);
                if let Some(hit) = self.hits.get(i).cloned() {
                    return self.activate(hit);
                }
            }
            Message::Reveal => {
                if let Some(p) = self.hits.get(self.selected).and_then(Hit::path).map(|p| p.to_owned()) {
                    let parent = p.parent().unwrap_or(&p).display().to_string();
                    return self.activate(Hit::new(
                        Kind::Action,
                        "Reveal",
                        Action::Run(vec!["cosmic-files".into(), parent]),
                    ));
                }
            }
            Message::Extras => {
                if self.extras_of.is_some() {
                    return Task::none();
                }
                let hit = self.hits.get(self.selected).cloned();
                let extras = if let Some(path) = hit.as_ref().and_then(Hit::path).map(|p| p.to_owned()) {
                    self.call(&Request::OpenWith { path })
                } else if let Some(id) = hit.as_ref().map(|h| h.clip_id).filter(|id| *id != 0) {
                    self.call(&Request::ClipboardExtras { id })
                } else {
                    None
                };
                if let Some(iliada_search::Response::Hits { hits, .. }) = extras {
                    let before = std::mem::replace(&mut self.hits, hits);
                    self.extras_of = Some((before, self.selected));
                    self.selected = 0;
                }
            }
            Message::Back => {
                if self.preview_open {
                    self.preview_open = false;
                    self.preview = None;
                    self.preview_loading = false;
                } else if !self.moving.is_empty() {
                    self.moving.clear();
                    self.note = None;
                    self.refresh();
                } else if let Some((hits, selected)) = self.extras_of.take() {
                    self.hits = hits;
                    self.selected = selected;
                    self.dropped.clear();
                    self.note = None;
                } else {
                    return self.hide();
                }
            }
            Message::TogglePreview => {
                self.ignore_space = true;
                if self.preview_open {
                    self.preview_open = false;
                    self.preview = None;
                    self.preview_loading = false;
                } else {
                    self.preview_open = true;
                    return self.request_preview();
                }
            }
            Message::PreviewReady(stamp, preview) => {
                if stamp == self.preview_stamp && self.preview_open {
                    self.preview = preview;
                    self.preview_loading = false;
                }
            }
            Message::Hide => return self.hide(),
            Message::HomerOn(on) => {
                if on != self.homer {
                    self.homer = on;
                    if self.surface.is_some() {
                        self.refresh();
                    }
                }
            }
            Message::Paste => {
                let _ = iliada_search::clipboard::paste();
            }
            Message::Dropped(mime, data) => {
                let paths = actions::parse_uri_list(&data);
                if !paths.is_empty() && (mime.contains("uri-list") || paths.iter().any(|p| p.exists())) {
                    return self.show_dropped(paths);
                }
                if let Ok(text) = String::from_utf8(data) {
                    let text = text.trim();
                    if !text.is_empty() {
                        self.mode = Mode::Search;
                        self.hits = vec![
                            Hit::new(Kind::Action, "Copy", Action::Copy(text.to_owned()))
                                .subtitle(text.to_owned())
                                .icon_name("edit-copy-symbolic"),
                            Hit::new(
                                Kind::Action,
                                "Ask Homer About It",
                                Action::AskHomer(format!("About this: {text}")),
                            )
                            .icon_name("org.iliada.Assistant"),
                        ];
                        self.selected = 0;
                        self.extras_of = Some((Vec::new(), 0));
                    }
                }
            }
            Message::Nothing => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        // No main window: the panel is the layer surface.
        widget::text("").into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if Some(id) == self.surface { self.panel() } else { widget::text("").into() }
    }
}
