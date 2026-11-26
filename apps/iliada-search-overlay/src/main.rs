//! Iliada's search overlay: Spotlight for Iliada.
//!
//! Started with the session and kept running, hidden, so the Super key
//! only has to show it: `iliada-search` (what the key runs) writes one
//! line to this program's socket and the panel appears, with no process to
//! start and nothing to load. Each keystroke asks `iliada-searchd`, which
//! holds the index in memory, for results (a few milliseconds even with a
//! million files).
//!
//! * Super: search apps, files and folders, Settings, agora, answers.
//! * Super+O: ask Homer.
//! * Super+V: clipboard history (the last 200 copies).
//!
//! Keys: ↑/↓ choose, Enter opens, Tab (or →) shows what else a file
//! offers (Open With, Reveal in Files, Copy Path), Ctrl+Enter reveals a
//! file in Files, Esc closes.
//!
//! It is a layer-shell surface (above every window, like COSMIC's own
//! launcher) covering the screen, transparent except for the panel; a
//! click outside the panel closes it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cosmic::app::{Settings, Task};
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
        match self.surface.take() {
            Some(id) => destroy_layer_surface(id),
            None => Task::none(),
        }
    }

    fn activate(&mut self, hit: Hit) -> Task<Message> {
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
        widget::mouse_area(content).on_press(Message::Activate(Some(i))).into()
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
        let hint = match (&self.extras_of, self.mode) {
            (Some(_), _) => fl!("hint-extras"),
            (None, Mode::Homer) => fl!("hint-homer"),
            (None, Mode::Clipboard) => fl!("hint-clipboard"),
            (None, Mode::Search) if self.query.is_empty() => fl!("hint-search-empty"),
            (None, Mode::Search) => fl!("hint-search", ms = format!("{:.1}", self.micros as f64 / 1000.0)),
        };
        col = col.push(widget::text::caption(hint));
        let panel = widget::container(col.padding(spacing.space_s))
            .width(Length::Fixed(WIDTH))
            .class(cosmic::theme::Container::custom(frosted));
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
                self.query = q;
                self.refresh();
            }
            Message::Up => self.selected = self.selected.saturating_sub(1),
            Message::Down => {
                if self.selected + 1 < self.hits.len().min(ROWS) {
                    self.selected += 1;
                }
            }
            Message::Activate(i) => {
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
                if let Some((hits, selected)) = self.extras_of.take() {
                    self.hits = hits;
                    self.selected = selected;
                } else {
                    return self.hide();
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
