//! Settings pages and quick actions, found by name or by what people call
//! them ("wifi", "night", "volume").

use crate::{Action, Hit, Kind};

/// A Settings page: its title, other words for it, its icon and the
/// `cosmic-settings` arguments that open it.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub title: &'static str,
    pub words: &'static str,
    pub icon: &'static str,
    pub args: &'static [&'static str],
}

/// COSMIC's pages and Iliada's (one Settings app shows both).
pub const PAGES: &[Page] = &[
    Page {
        title: "Wi-Fi",
        words: "wifi wireless network internet",
        icon: "network-wireless-symbolic",
        args: &["wireless"],
    },
    Page {
        title: "Network",
        words: "ethernet wired internet proxy",
        icon: "network-wired-symbolic",
        args: &["network"],
    },
    Page { title: "VPN", words: "vpn wireguard openvpn", icon: "network-vpn-symbolic", args: &["vpn"] },
    Page {
        title: "Bluetooth",
        words: "bluetooth headphones devices",
        icon: "bluetooth-symbolic",
        args: &["bluetooth"],
    },
    Page {
        title: "Appearance",
        words: "theme dark light mode colors accent",
        icon: "preferences-desktop-appearance-symbolic",
        args: &["appearance"],
    },
    Page {
        title: "Wallpaper",
        words: "background picture desktop",
        icon: "preferences-desktop-wallpaper-symbolic",
        args: &["wallpaper"],
    },
    Page { title: "Dock", words: "dock taskbar", icon: "preferences-dock-symbolic", args: &["dock"] },
    Page { title: "Panel", words: "top bar panel", icon: "preferences-panel-symbolic", args: &["panel"] },
    Page {
        title: "Workspaces",
        words: "workspaces virtual desktops",
        icon: "preferences-workspaces-symbolic",
        args: &["workspaces"],
    },
    Page {
        title: "Window Management",
        words: "windows tiling focus",
        icon: "preferences-window-management-symbolic",
        args: &["window-management"],
    },
    Page {
        title: "Displays",
        words: "monitor screen resolution scale night light brightness",
        icon: "preferences-desktop-display-symbolic",
        args: &["displays"],
    },
    Page {
        title: "Sound",
        words: "audio volume speakers microphone output input",
        icon: "preferences-sound-symbolic",
        args: &["sound"],
    },
    Page {
        title: "Power & Battery",
        words: "battery power sleep suspend energy",
        icon: "preferences-power-and-battery-symbolic",
        args: &["power"],
    },
    Page {
        title: "Keyboard",
        words: "keyboard layout input language typing",
        icon: "preferences-desktop-keyboard-symbolic",
        args: &["keyboard"],
    },
    Page {
        title: "Keyboard Shortcuts",
        words: "shortcuts hotkeys keybindings keys",
        icon: "preferences-desktop-keyboard-shortcuts-symbolic",
        args: &["keyboard", "shortcuts"],
    },
    Page { title: "Mouse", words: "mouse pointer cursor speed", icon: "input-mouse-symbolic", args: &["mouse"] },
    Page {
        title: "Touchpad",
        words: "touchpad trackpad gestures scroll",
        icon: "input-touchpad-symbolic",
        args: &["touchpad"],
    },
    Page { title: "Users", words: "accounts users password login", icon: "system-users-symbolic", args: &["users"] },
    Page {
        title: "Date & Time",
        words: "clock time zone timezone date",
        icon: "preferences-system-time-symbolic",
        args: &["date-time"],
    },
    Page {
        title: "Region & Language",
        words: "language locale region formats",
        icon: "preferences-desktop-locale-symbolic",
        args: &["region-language"],
    },
    Page {
        title: "Default Apps",
        words: "default applications browser open with",
        icon: "preferences-desktop-default-applications-symbolic",
        args: &["default-apps"],
    },
    Page {
        title: "Startup Apps",
        words: "startup login autostart",
        icon: "preferences-desktop-startup-applications-symbolic",
        args: &["startup-apps"],
    },
    Page {
        title: "Accessibility",
        words: "accessibility magnifier screen reader contrast",
        icon: "preferences-desktop-accessibility-symbolic",
        args: &["accessibility"],
    },
    Page {
        title: "Iliada Account",
        words: "account sign in sync recovery key",
        icon: "avatar-default-symbolic",
        args: &["iliada", "account"],
    },
    Page {
        title: "Privacy",
        words: "privacy permissions camera microphone files location isolation",
        icon: "preferences-system-privacy-symbolic",
        args: &["iliada", "privacy"],
    },
    Page {
        title: "Homer",
        words: "homer assistant ai model api key",
        icon: "org.iliada.Assistant",
        args: &["iliada", "homer"],
    },
    Page {
        title: "Updates",
        words: "updates upgrade version software update",
        icon: "software-update-available-symbolic",
        args: &["iliada", "updates"],
    },
    Page {
        title: "Backup",
        words: "backup restore github",
        icon: "drive-harddisk-symbolic",
        args: &["iliada", "backup"],
    },
    Page {
        title: "Services",
        words: "services ssh server daemons",
        icon: "network-server-symbolic",
        args: &["iliada", "services"],
    },
    Page {
        title: "Command-Line Tools",
        words: "tools terminal packages cli nix",
        icon: "utilities-terminal-symbolic",
        args: &["iliada", "tools"],
    },
    Page {
        title: "About This Computer",
        words: "about system version hardware memory disk",
        icon: "help-about-symbolic",
        args: &["iliada", "about"],
    },
];

impl Page {
    pub fn hit(&self) -> Hit {
        let mut argv = vec!["cosmic-settings".to_owned()];
        argv.extend(self.args.iter().map(|a| (*a).to_owned()));
        Hit::new(Kind::Setting, self.title, Action::Run(argv))
            .subtitle("Settings")
            .icon_name(self.icon)
            .key(format!("settings:{}", self.args.join("/")))
    }
}

/// A quick action.
#[derive(Debug, Clone, Copy)]
pub struct QuickAction {
    pub title: &'static str,
    pub words: &'static str,
    pub icon: &'static str,
    pub what: Quick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quick {
    Dark,
    Light,
    Command(&'static [&'static str]),
}

/// Restart, Shut Down and Log Out open COSMIC's own confirmation, so a
/// stray Enter never ends the session.
pub const ACTIONS: &[QuickAction] = &[
    QuickAction {
        title: "Dark Mode",
        words: "dark mode night theme appearance",
        icon: "weather-clear-night-symbolic",
        what: Quick::Dark,
    },
    QuickAction {
        title: "Light Mode",
        words: "light mode day theme appearance",
        icon: "weather-clear-symbolic",
        what: Quick::Light,
    },
    QuickAction {
        title: "Lock Screen",
        words: "lock screen away",
        icon: "system-lock-screen-symbolic",
        what: Quick::Command(&["loginctl", "lock-session"]),
    },
    QuickAction {
        title: "Sleep",
        words: "sleep suspend",
        icon: "weather-clear-night-symbolic",
        what: Quick::Command(&["systemctl", "suspend"]),
    },
    QuickAction {
        title: "Restart…",
        words: "restart reboot",
        icon: "system-reboot-symbolic",
        what: Quick::Command(&["cosmic-osd", "restart"]),
    },
    QuickAction {
        title: "Shut Down…",
        words: "shut down shutdown power off turn off",
        icon: "system-shutdown-symbolic",
        what: Quick::Command(&["cosmic-osd", "shutdown"]),
    },
    QuickAction {
        title: "Log Out…",
        words: "log out logout sign out",
        icon: "system-log-out-symbolic",
        what: Quick::Command(&["cosmic-osd", "log-out"]),
    },
    QuickAction {
        title: "Mute / Unmute",
        words: "mute unmute sound silence volume",
        icon: "audio-volume-muted-symbolic",
        what: Quick::Command(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
    },
    QuickAction {
        title: "Turn Wi-Fi Off",
        words: "wifi off disable wireless airplane",
        icon: "network-wireless-disabled-symbolic",
        what: Quick::Command(&["nmcli", "radio", "wifi", "off"]),
    },
    QuickAction {
        title: "Turn Wi-Fi On",
        words: "wifi on enable wireless",
        icon: "network-wireless-symbolic",
        what: Quick::Command(&["nmcli", "radio", "wifi", "on"]),
    },
    QuickAction {
        title: "Take a Screenshot",
        words: "screenshot screen capture print",
        icon: "applets-screenshooter-symbolic",
        what: Quick::Command(&["cosmic-screenshot", "--interactive"]),
    },
    QuickAction {
        title: "Check for Updates",
        words: "update upgrade check",
        icon: "software-update-available-symbolic",
        what: Quick::Command(&["cosmic-settings", "iliada", "updates"]),
    },
];

impl QuickAction {
    pub fn hit(&self) -> Hit {
        let action = match self.what {
            Quick::Dark => Action::Appearance(true),
            Quick::Light => Action::Appearance(false),
            Quick::Command(argv) => Action::Run(argv.iter().map(|a| (*a).to_owned()).collect()),
        };
        Hit::new(Kind::Action, self.title, action)
            .subtitle("Action")
            .icon_name(self.icon)
            .key(format!("action:{}", self.title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_open_settings() {
        let h = PAGES.iter().find(|p| p.title == "Homer").unwrap().hit();
        assert_eq!(h.action, Action::Run(vec!["cosmic-settings".into(), "iliada".into(), "homer".into()]));
        assert_eq!(h.key, "settings:iliada/homer");
    }

    #[test]
    fn actions() {
        assert_eq!(ACTIONS[0].hit().action, Action::Appearance(true));
        assert!(ACTIONS.iter().all(|a| !a.words.is_empty()));
    }
}
