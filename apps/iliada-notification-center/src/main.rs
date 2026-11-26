//! Iliada's Notification Center (ADR 0010): the panel that slides in from
//! the right edge of the screen.
//!
//! Started with the session and kept running, hidden, like Search. The top
//! bar's clock and bell (`iliada-notification-applet`) and Super+N run
//! `iliada-notification-center toggle`, which writes one line to this
//! program's socket.
//!
//! * Notifications grouped by app, newest first. A group collapses to its
//!   newest notification; Clear clears it, Clear All clears everything.
//! * Clicking a notification runs its default action and switches to the
//!   app; its buttons run their actions.
//! * Do Not Disturb at the top (also Super+Shift+N).
//! * Esc or a click outside closes it.
//!
//! The list comes from `iliada-notifyd` (`iliada_notify::proto`), which
//! keeps it; this program keeps nothing.

use std::collections::HashSet;
use std::path::PathBuf;

use cosmic::app::{Settings, Task};
use cosmic::iced::keyboard::{Key, key::Named};
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::{IcedOutput, SctkLayerSurfaceSettings};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, Layer, destroy_layer_surface, get_layer_surface,
};
use cosmic::iced::{Alignment, Length, Subscription, window};
use cosmic::widget;
use cosmic::{Core, Element, executor};
use iliada_notification_center::{fl, i18n, socket_path};
use iliada_notify::dnd::{self, Why};
use iliada_notify::history::{self, Group};
use iliada_notify::proto::{self, Request};
use iliada_notify::{Notification, Urgency};

const WIDTH: f32 = 400.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Toggle,
    Show,
    Hide,
}

impl Command {
    fn parse(s: &str) -> Option<Command> {
        match s.trim() {
            "toggle" => Some(Command::Toggle),
            "show" => Some(Command::Show),
            "hide" => Some(Command::Hide),
            _ => None,
        }
    }
}

/// The socket this program listens on, claimed before anything else so a
/// second start only passes its command on.
static LISTENER: std::sync::OnceLock<std::os::unix::net::UnixListener> = std::sync::OnceLock::new();

/// `false`: one runs already (and was told `command`).
fn claim(command: Option<&str>) -> bool {
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
                    if let Some(c) = command {
                        let _ = writeln!(s, "{c}");
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
    let command = std::env::args().nth(1);
    let command = command.as_deref().filter(|c| Command::parse(c).is_some());
    if !claim(command) {
        return Ok(());
    }
    let show = matches!(command.and_then(Command::parse), Some(Command::Toggle | Command::Show));
    let settings = Settings::default().no_main_window(true).exit_on_close(false).transparent(true);
    cosmic::app::run::<App>(settings, show)
}

/// Commands from the top bar button and Super+N (one line each).
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
        let mut command = Command::Toggle;
        if let Ok((s, _)) = listener.accept().await {
            let mut line = String::new();
            let mut r = tokio::io::BufReader::new(s);
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), r.read_line(&mut line)).await;
            command = Command::parse(&line).unwrap_or(Command::Toggle);
        }
        Some((Message::Command(command), Some(listener)))
    })
}

/// While the panel is open: look for new notifications every two seconds.
fn ticks() -> impl cosmic::iced::futures::Stream<Item = Message> {
    cosmic::iced::futures::stream::unfold((), |()| async {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        Some((Message::Refresh, ()))
    })
}

#[derive(Debug, Clone)]
enum Message {
    Command(Command),
    Hide,
    Nothing,
    Refresh,
    Loaded(Vec<Notification>, Option<Why>),
    /// Clicked: its default action, then its app.
    Open(u32, String),
    /// One of its buttons.
    Act(u32, String, String),
    Dismiss(u32),
    ClearApp(String),
    ClearAll,
    Collapse(String),
    Dnd(bool),
}

struct App {
    core: Core,
    surface: Option<window::Id>,
    /// Newest first.
    items: Vec<Notification>,
    collapsed: HashSet<String>,
    dnd: Option<Why>,
}

/// The list and Do Not Disturb, read off the UI thread.
fn load() -> Task<Message> {
    cosmic::task::future(async {
        let (items, dnd) =
            tokio::task::spawn_blocking(|| (proto::recent(history::MAX), dnd::current())).await.unwrap_or_default();
        Message::Loaded(items, dnd)
    })
}

/// Ask the notification service, then read the list again.
fn ask(request: Request) -> Task<Message> {
    cosmic::task::future(async move {
        let _ = tokio::task::spawn_blocking(move || proto::call(&request)).await;
        Message::Refresh
    })
}

/// Switch to a window of `app`, if one is open: Search's service focuses it
/// (COSMIC's toplevel management, which also changes workspace).
fn focus_app(app: &str) {
    let Ok(out) = std::process::Command::new("lswt").arg("-j").output() else { return };
    let windows = iliada_types::toplevel::parse_lswt(&String::from_utf8_lossy(&out.stdout));
    let want = app.to_ascii_lowercase();
    let short = |s: &str| s.rsplit('.').next().unwrap_or(s).to_owned();
    let found = windows.iter().find(|w| {
        let id = w.app_id.to_ascii_lowercase();
        !id.is_empty() && (id == want || short(&id) == short(&want))
    });
    if let Some(w) = found
        && let Ok(mut c) = iliada_search::proto::Client::connect()
    {
        let _ = c.call(&iliada_search::proto::Request::FocusWindow { identifier: w.identifier.clone() });
    }
}

fn open(id: u32, action: Option<String>, app: String) -> Task<Message> {
    cosmic::task::future(async move {
        let _ = tokio::task::spawn_blocking(move || {
            let _ = proto::call(&Request::Activate { id, action });
            focus_app(&app);
        })
        .await;
        Message::Hide
    })
}

/// An icon by name, or by path for apps that send a file.
fn icon(name: &str, fallback: &str, size: u16) -> Element<'static, Message> {
    let name = name.trim();
    let path = name.strip_prefix("file://").unwrap_or(name);
    if path.starts_with('/') {
        return widget::icon(widget::icon::from_path(PathBuf::from(path))).size(size).into();
    }
    let name = if name.is_empty() { fallback } else { name };
    widget::icon::from_name(name.to_owned()).size(size).icon().into()
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

impl App {
    fn show(&mut self) -> Task<Message> {
        let id = window::Id::unique();
        self.surface = Some(id);
        let surface = get_layer_surface(SctkLayerSurfaceSettings {
            id,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            output: IcedOutput::Active,
            namespace: "iliada-notification-center".into(),
            size: Some((None, None)),
            // Below the top bar, not over it.
            exclusive_zone: 0,
            ..Default::default()
        });
        Task::batch([surface, load()])
    }

    fn hide(&mut self) -> Task<Message> {
        match self.surface.take() {
            Some(id) => destroy_layer_surface(id),
            None => Task::none(),
        }
    }

    fn notification(&self, n: &Notification) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut title = widget::row::with_capacity(3)
            .push(widget::text::heading(n.summary.clone()).width(Length::Fill))
            .push(widget::text::caption(i18n::ago(n.time)))
            .push(
                widget::button::icon(widget::icon::from_name("window-close-symbolic"))
                    .extra_small()
                    .tooltip(fl!("dismiss"))
                    .on_press(Message::Dismiss(n.id)),
            )
            .spacing(spacing.space_xxs)
            .align_y(Alignment::Center);
        if n.urgency == Urgency::Critical {
            title = title.push(widget::icon::from_name("dialog-warning-symbolic").size(16).icon());
        }
        let mut text = widget::column::with_capacity(2).push(title).spacing(spacing.space_xxxs);
        let body = n.plain_body();
        if !body.trim().is_empty() {
            text = text.push(widget::text::body(body));
        }
        let clickable = widget::mouse_area(text).on_press(Message::Open(n.id, n.app_key().to_owned()));
        let mut col = widget::column::with_capacity(2).push(clickable).spacing(spacing.space_xs);
        let buttons: Vec<_> = n.buttons().collect();
        if !buttons.is_empty() {
            let mut row = widget::row::with_capacity(buttons.len()).spacing(spacing.space_xs);
            for a in buttons {
                row = row.push(widget::button::standard(a.label.clone()).on_press(Message::Act(
                    n.id,
                    a.id.clone(),
                    n.app_key().to_owned(),
                )));
            }
            col = col.push(row);
        }
        widget::container(col)
            .padding(spacing.space_xs)
            .width(Length::Fill)
            .class(cosmic::theme::Container::Card)
            .into()
    }

    fn group(&self, g: &Group) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let collapsed = self.collapsed.contains(&g.app_id);
        let mut header = widget::row::with_capacity(5)
            .push(icon(&g.icon, &g.app_id, 20))
            .push(widget::text::heading(g.app_name.clone()).width(Length::Fill))
            .spacing(spacing.space_xs)
            .align_y(Alignment::Center);
        if g.items.len() > 1 {
            let (chevron, tip) = if collapsed {
                ("pan-end-symbolic", fl!("show-all", count = g.items.len()))
            } else {
                ("pan-down-symbolic", fl!("show-less"))
            };
            header = header.push(
                widget::button::icon(widget::icon::from_name(chevron))
                    .extra_small()
                    .tooltip(tip)
                    .on_press(Message::Collapse(g.app_id.clone())),
            );
        }
        header = header.push(widget::button::text(fl!("clear")).on_press(Message::ClearApp(g.app_id.clone())));
        let mut col = widget::column::with_capacity(g.items.len() + 2).push(header).spacing(spacing.space_xxs);
        let shown = if collapsed { 1 } else { g.items.len() };
        for n in g.items.iter().take(shown) {
            col = col.push(self.notification(n));
        }
        if collapsed && g.items.len() > 1 {
            let count = g.items.len() - 1;
            col = col.push(widget::text::caption(fl!("more", count = count)));
        }
        col.into()
    }

    fn panel(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut top = widget::row::with_capacity(2)
            .push(widget::text::title4(fl!("notifications")).width(Length::Fill))
            .align_y(Alignment::Center);
        if !self.items.is_empty() {
            top = top.push(widget::button::text(fl!("clear-all")).on_press(Message::ClearAll));
        }
        let moon = if self.dnd.is_some() { "weather-clear-night-symbolic" } else { "notification-symbolic" };
        let dnd = widget::row::with_capacity(3)
            .push(widget::icon::from_name(moon).size(20).icon())
            .push(
                widget::column::with_capacity(2)
                    .push(widget::text::body(fl!("do-not-disturb")))
                    .push(widget::text::caption(i18n::dnd_status(self.dnd)))
                    .width(Length::Fill),
            )
            .push(widget::toggler(self.dnd.is_some()).on_toggle(Message::Dnd))
            .spacing(spacing.space_s)
            .align_y(Alignment::Center);
        let dnd = widget::container(dnd).padding(spacing.space_xs).class(cosmic::theme::Container::Card);

        let mut list = widget::column::with_capacity(8).spacing(spacing.space_m);
        let groups = history::groups(self.items.iter());
        if groups.is_empty() {
            list = list.push(
                widget::container(widget::text::body(fl!("no-notifications")))
                    .center_x(Length::Fill)
                    .padding(spacing.space_l),
            );
        }
        for g in &groups {
            list = list.push(self.group(g));
        }
        let content = widget::column::with_capacity(4)
            .push(top)
            .push(dnd)
            .push(widget::divider::horizontal::light())
            .push(widget::scrollable(list).height(Length::Fill))
            .spacing(spacing.space_s)
            .padding(spacing.space_s);
        let panel = widget::container(content)
            .width(Length::Fixed(WIDTH))
            .height(Length::Fill)
            .class(cosmic::theme::Container::custom(frosted));
        // The panel along the right edge; a click anywhere else closes it.
        let page = widget::row::with_capacity(2)
            .push(
                widget::mouse_area(widget::space::horizontal().width(Length::Fill).height(Length::Fill))
                    .on_press(Message::Hide),
            )
            .push(widget::mouse_area(panel).on_press(Message::Nothing))
            .padding(spacing.space_xs)
            .width(Length::Fill)
            .height(Length::Fill);
        page.into()
    }
}

impl cosmic::Application for App {
    type Executor = executor::Default;
    type Flags = bool;
    type Message = Message;
    const APP_ID: &'static str = "org.iliada.NotificationCenter";

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, show: bool) -> (Self, Task<Message>) {
        let mut app = App { core, surface: None, items: Vec::new(), collapsed: HashSet::new(), dnd: None };
        let task = if show { app.show() } else { Task::none() };
        (app, task)
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = cosmic::iced::event::listen_with(|event, _status, _window| match event {
            cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }) => Some(Message::Hide),
            _ => None,
        });
        let mut subs = vec![Subscription::run(requests), keys];
        if self.surface.is_some() {
            subs.push(Subscription::run(ticks));
        }
        Subscription::batch(subs)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Command(c) => match (c, self.surface.is_some()) {
                (Command::Toggle | Command::Hide, true) => return self.hide(),
                (Command::Toggle | Command::Show, false) => return self.show(),
                _ => {}
            },
            Message::Hide => return self.hide(),
            Message::Nothing => {}
            Message::Refresh => {
                if self.surface.is_some() {
                    return load();
                }
            }
            Message::Loaded(items, dnd) => {
                self.items = items;
                self.dnd = dnd;
            }
            Message::Open(id, app) => return open(id, None, app),
            Message::Act(id, action, app) => return open(id, Some(action), app),
            Message::Dismiss(id) => {
                self.items.retain(|n| n.id != id);
                return ask(Request::Dismiss { id });
            }
            Message::ClearApp(app) => {
                self.items.retain(|n| n.app_key() != app);
                return ask(Request::ClearApp { app });
            }
            Message::ClearAll => {
                self.items.clear();
                return ask(Request::ClearAll);
            }
            Message::Collapse(app) => {
                if !self.collapsed.remove(&app) {
                    self.collapsed.insert(app);
                }
            }
            Message::Dnd(on) => {
                return cosmic::task::future(async move {
                    let _ = tokio::task::spawn_blocking(move || dnd::switch(on, None)).await;
                    Message::Refresh
                });
            }
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
