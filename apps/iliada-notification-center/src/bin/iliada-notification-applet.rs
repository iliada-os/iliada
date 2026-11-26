//! The Notification Center's button in the top bar (a COSMIC panel
//! applet, `org.iliada.NotificationCenterApplet`): the time, and a bell,
//! or a moon while Do Not Disturb is on. Clicking it opens or closes the
//! Notification Center, as Super+N does.
//!
//! It replaces COSMIC's clock and notifications applets in Iliada's top
//! bar (nix/modules/desktop.nix), so the clock is where the Notification
//! Center opens, as on a Mac.

use std::time::Duration;

use cosmic::app::Task;
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::widget;
use cosmic::{Core, Element};
use iliada_notification_center::{fl, i18n, tell_panel};
use iliada_notify::dnd::{self, Why};

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

#[derive(Debug, Clone)]
enum Message {
    Toggle,
    Tick,
}

struct Applet {
    core: Core,
    time: String,
    dnd: Option<Why>,
}

impl Applet {
    fn refresh(&mut self) {
        self.time = chrono::Local::now().format("%H:%M").to_string();
        self.dnd = dnd::current();
    }
}

/// The clock changes each minute; Do Not Disturb can change any time
/// (Settings, Super+Shift+N, the panel, Homer).
fn ticks() -> impl cosmic::iced::futures::Stream<Item = Message> {
    cosmic::iced::futures::stream::unfold((), |()| async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        Some((Message::Tick, ()))
    })
}

impl cosmic::Application for Applet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "org.iliada.NotificationCenterApplet";

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Message>) {
        let mut a = Applet { core, time: String::new(), dnd: None };
        a.refresh();
        (a, Task::none())
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(ticks)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Toggle => tell_panel("toggle"),
            Message::Tick => self.refresh(),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let (icon, tip) = match self.dnd {
            Some(why) => ("weather-clear-night-symbolic", i18n::dnd_status(Some(why))),
            None => ("notification-symbolic", fl!("notifications")),
        };
        let size = self.core.applet.suggested_size(true);
        let pad = self.core.applet.suggested_padding(true);
        let content = widget::row::with_capacity(2)
            .push(widget::icon::from_name(icon).size(size.0).icon())
            .push(self.core.applet.text(self.time.clone()))
            .spacing(6)
            .align_y(Alignment::Center)
            .height(Length::Fixed(f32::from(size.1 + 2 * pad.1)));
        let button = widget::button::custom(content)
            .padding([0, pad.0])
            .on_press_down(Message::Toggle)
            .class(cosmic::theme::Button::AppletIcon);
        widget::tooltip(button, widget::text::body(tip), widget::tooltip::Position::Bottom).into()
    }
}
