//! The permission prompt: drawn by Iliada, never by the app asking.
//!
//!   iliada-prompt --app GIMP --wants "access your Pictures folder" \
//!                 --purpose "…" [--icon /Applications/GIMP.app/Contents/Resources/gimp.png]
//!
//!   iliada-prompt --app Firefox --notice "Firefox is made for …"
//!
//! Exit status: 0 allowed, 1 not allowed, 2 closed without answering. With
//! `--notice` it only tells the user something (one OK button; exit 0).
//! `iliada-run` runs it before starting an app and remembers the answer.
//!
//! It is an overlay (a layer-shell surface, like COSMIC's own password
//! dialog), not a window: it stays above every app, so it can't end up hidden
//! behind another window, and no app can cover it.

use std::path::PathBuf;

use cosmic::app::{Settings, Task};
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::{IcedOutput, SctkLayerSurfaceSettings};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, Layer, get_layer_surface,
};
use cosmic::iced::{Alignment, Length, window};
use cosmic::widget;
use cosmic::{Core, Element, executor};
use iliada_settings::fl;
use iliada_settings::i18n::lookup;

#[derive(Debug, Clone, Default)]
struct Ask {
    app: String,
    wants: String,
    purpose: String,
    icon: Option<PathBuf>,
    /// A message instead of a question.
    notice: String,
}

/// Untrusted text (it comes from the app's bundle): no control characters,
/// bounded length.
fn clean(s: &str, max: usize) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

fn parse() -> Option<Ask> {
    let mut a = Ask::default();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let v = it.next()?;
        match flag.as_str() {
            "--app" => a.app = clean(&v, 60),
            "--wants" => a.wants = clean(&v, 120),
            "--purpose" => a.purpose = clean(&v, 300),
            "--icon" => a.icon = Some(PathBuf::from(v)),
            "--notice" => a.notice = clean(&v, 400),
            _ => return None,
        }
    }
    (!a.app.is_empty() && (!a.wants.is_empty() || !a.notice.is_empty())).then_some(a)
}

const WIDTH: f32 = 480.0;

/// What the app wants, in the user's language. `iliada-run` passes the
/// capability's English words; anything else is shown as it came.
fn wants(english: &str) -> String {
    translate(english).unwrap_or_else(|| english.to_owned())
}

fn translate(english: &str) -> Option<String> {
    let key = match english {
        "use your camera" => "wants-camera",
        "use your microphone" => "wants-microphone",
        "know your approximate location" => "wants-location-coarse",
        "know your precise location" => "wants-location-precise",
        "read your clipboard" => "wants-clipboard-read",
        "write to your clipboard" => "wants-clipboard-write",
        "open a file you choose" => "wants-file",
        "show notifications" => "wants-notifications",
        "record your screen" => "wants-screen",
        "play sound" => "wants-sound",
        _ => {
            let folder = english.strip_prefix("access your ")?.strip_suffix(" folder")?;
            let name = lookup(&format!("folder-{}", folder.to_lowercase()))?;
            return Some(fl!("wants-folder", folder = name));
        }
    };
    lookup(key)
}

/// The question, as a sentence.
fn title(a: &Ask) -> String {
    fl!("prompt-title", app = a.app.as_str(), wants = wants(&a.wants))
}

/// Tall enough for the question and the app's reason: wrapped lines are
/// estimated from the text (the content also scrolls, so nothing is cut).
fn height(a: &Ask) -> f32 {
    let lines = |text: &str, per_line: usize| text.chars().count().div_ceil(per_line).max(1) as f32;
    if !a.notice.is_empty() {
        return (200.0 + 22.0 * lines(&a.notice, 50)).clamp(280.0, 560.0);
    }
    let title = lines(&title(a), 34);
    let purpose = if a.purpose.is_empty() { 0.0 } else { lines(&a.purpose, 62) };
    (250.0 + 30.0 * title + 20.0 * purpose).clamp(320.0, 640.0)
}

fn main() {
    let Some(ask) = parse() else {
        eprintln!("usage: iliada-prompt --app NAME (--wants TEXT [--purpose TEXT] | --notice TEXT) [--icon PATH]");
        std::process::exit(2);
    };
    let settings = Settings::default().no_main_window(true).exit_on_close(false);
    let _ = cosmic::app::run::<App>(settings, ask);
    // Closed without an answer.
    std::process::exit(2);
}

#[derive(Debug, Clone)]
enum Message {
    Answer(bool),
}

struct App {
    core: Core,
    ask: Ask,
    surface: window::Id,
}

impl cosmic::Application for App {
    type Executor = executor::Default;
    type Flags = Ask;
    type Message = Message;
    const APP_ID: &'static str = "org.iliada.Prompt";

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, ask: Ask) -> (Self, Task<Message>) {
        let surface = window::Id::unique();
        let size = (WIDTH as u32, height(&ask) as u32);
        let overlay = get_layer_surface(SctkLayerSurfaceSettings {
            id: surface,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            // No anchor: centred on the screen in use.
            anchor: Anchor::empty(),
            output: IcedOutput::Active,
            namespace: "iliada-prompt".into(),
            size: Some((Some(size.0), Some(size.1))),
            ..Default::default()
        });
        (App { core, ask, surface }, overlay)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Answer(allow) => std::process::exit(if allow { 0 } else { 1 }),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        // There is no main window; the prompt is the overlay.
        widget::text("").into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        let content = if id == self.surface { self.prompt() } else { widget::text("").into() };
        widget::container(content)
            .class(cosmic::theme::Container::Dialog(true))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

impl App {
    fn prompt(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let icon: Element<_> = match &self.ask.icon {
            Some(p) if p.is_file() => widget::icon::icon(widget::icon::from_path(p.clone())).size(64).into(),
            _ => widget::icon::from_name("dialog-question").size(64).icon().into(),
        };
        if !self.ask.notice.is_empty() {
            let col = widget::column::with_capacity(3)
                .push(icon)
                .push(widget::text::body(self.ask.notice.as_str()).width(Length::Fill).align_x(Alignment::Center))
                .push(widget::button::suggested(fl!("ok")).on_press(Message::Answer(true)))
                .spacing(spacing.space_s)
                .align_x(Alignment::Center);
            return widget::scrollable(widget::container(col).center_x(Length::Fill).padding(spacing.space_m))
                .height(Length::Fill)
                .into();
        }
        let mut col = widget::column::with_capacity(5)
            .push(icon)
            .push(widget::text::title4(title(&self.ask)).width(Length::Fill).align_x(Alignment::Center))
            .spacing(spacing.space_s)
            .align_x(Alignment::Center);
        if !self.ask.purpose.is_empty() {
            col = col
                .push(widget::text::caption(self.ask.purpose.as_str()).width(Length::Fill).align_x(Alignment::Center));
        }
        col = col.push(widget::text::caption(fl!("prompt-later")).width(Length::Fill).align_x(Alignment::Center));
        let buttons = widget::row::with_capacity(2)
            .push(widget::button::standard(fl!("dont-allow")).on_press(Message::Answer(false)))
            .push(widget::button::suggested(fl!("allow")).on_press(Message::Answer(true)))
            .spacing(spacing.space_s);
        col = col.push(buttons);
        widget::scrollable(widget::container(col).center_x(Length::Fill).padding(spacing.space_m))
            .height(Length::Fill)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iliada_types::{Capability, ClipboardAccess, FileScope, KnownDir, LocationPrecision};

    #[test]
    fn every_capability_has_words_in_each_language() {
        let mut caps = vec![
            Capability::Camera,
            Capability::Microphone,
            Capability::Location(LocationPrecision::Coarse),
            Capability::Location(LocationPrecision::Precise),
            Capability::Clipboard(ClipboardAccess::Read),
            Capability::Clipboard(ClipboardAccess::Write),
            Capability::Files(FileScope::UserPicked),
            Capability::Notifications,
            Capability::ScreenCapture,
            Capability::Sound,
        ];
        for d in [KnownDir::Documents, KnownDir::Downloads, KnownDir::Music, KnownDir::Pictures, KnownDir::Videos] {
            caps.push(Capability::Files(FileScope::KnownDir(d)));
        }
        for cap in caps {
            let english = cap.to_string();
            assert!(translate(&english).is_some(), "no message for {english:?}");
        }
        assert_eq!(wants("juggle"), "juggle");
    }
}
