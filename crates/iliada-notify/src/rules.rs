//! What happens to a notification that arrives: whether it pops up as a
//! banner, plays a sound, and is kept in the Notification Center.
//!
//! * An app that is not allowed: nothing (dropped).
//! * Banners and sound follow the app's switches, and Do Not Disturb holds
//!   both back, except for critical notifications when that is allowed.
//!   Low-urgency notifications never play a sound.
//! * Never kept: notifications from private apps (the app's "Show in
//!   history" is off), from the windows nobody but the person may see
//!   (permission prompts, the portal's consent, Homer's banner: the same
//!   protected ids Homer's app control and Search skip), and ones the app
//!   marked `transient`.

use iliada_types::toplevel::is_protected;

use crate::dnd::{self, Clock};
use crate::settings::Settings;
use crate::{Notification, Urgency};

/// What the app asked for, beyond the notification itself (hints).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hints {
    /// `transient`: not to be kept.
    pub transient: bool,
    /// `suppress-sound`.
    pub suppress_sound: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// The app may send notifications at all.
    pub allowed: bool,
    pub banner: bool,
    pub sound: bool,
    /// Written to the history (the Notification Center).
    pub keep: bool,
}

impl Decision {
    const DROPPED: Decision = Decision { allowed: false, banner: false, sound: false, keep: false };
}

/// Whether notifications from this app are never kept.
pub fn private(settings: &Settings, n: &Notification) -> bool {
    is_protected(&n.app_id) || is_protected(&n.app_name) || !settings.rule(n.app_key()).history
}

pub fn decide(settings: &Settings, clock: &Clock, n: &Notification, hints: Hints) -> Decision {
    let rule = settings.rule(n.app_key());
    if !rule.allow {
        return Decision::DROPPED;
    }
    let dnd = &settings.do_not_disturb;
    let quiet = dnd::status(dnd, clock).is_some() && !dnd::bypasses(dnd, n.urgency);
    Decision {
        allowed: true,
        banner: rule.banners && !quiet,
        sound: rule.sound && !quiet && !hints.suppress_sound && n.urgency != Urgency::Low,
        keep: !hints.transient && !private(settings, n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::sample;
    use crate::settings::AppRule;

    fn noon() -> Clock {
        Clock { now: 1_000_000, local_secs: 12 * 3600 }
    }

    #[test]
    fn everything_by_default() {
        let d = decide(&Settings::default(), &noon(), &sample(1, "mail", 0), Hints::default());
        assert_eq!(d, Decision { allowed: true, banner: true, sound: true, keep: true });
    }

    #[test]
    fn per_app_switches() {
        let mut s = Settings::default();
        let n = sample(1, "chat", 0);
        s.apps.insert("chat".into(), AppRule { banners: false, ..AppRule::default() });
        let d = decide(&s, &noon(), &n, Hints::default());
        assert!(!d.banner && d.sound && d.keep);
        s.apps.insert("chat".into(), AppRule { sound: false, ..AppRule::default() });
        let d = decide(&s, &noon(), &n, Hints::default());
        assert!(d.banner && !d.sound && d.keep);
        s.apps.insert("chat".into(), AppRule { allow: false, ..AppRule::default() });
        assert_eq!(decide(&s, &noon(), &n, Hints::default()), Decision::DROPPED);
        // Other apps are not affected.
        assert!(decide(&s, &noon(), &sample(2, "mail", 0), Hints::default()).banner);
    }

    #[test]
    fn private_apps_are_never_kept() {
        let mut s = Settings::default();
        s.apps.insert("org.keepassxc.KeePassXC".into(), AppRule { history: false, ..AppRule::default() });
        let d = decide(&s, &noon(), &sample(1, "org.keepassxc.KeePassXC", 0), Hints::default());
        assert!(d.banner && !d.keep);
        // Protected windows: permission prompts, the portal, Homer's banner.
        for app in ["org.iliada.Prompt", "xdg-desktop-portal-cosmic", "org.iliada.HomerBanner"] {
            assert!(!decide(&s, &noon(), &sample(1, app, 0), Hints::default()).keep, "{app}");
        }
        let mut by_name = sample(1, "", 0);
        by_name.app_name = "iliada-prompt".into();
        assert!(!decide(&s, &noon(), &by_name, Hints::default()).keep);
        // Transient ones, from any app.
        let transient = Hints { transient: true, ..Hints::default() };
        assert!(!decide(&s, &noon(), &sample(1, "mail", 0), transient).keep);
    }

    #[test]
    fn do_not_disturb_holds_back_banners_and_sounds() {
        let mut s = Settings::default();
        dnd::turn_on(&mut s.do_not_disturb, None);
        let d = decide(&s, &noon(), &sample(1, "mail", 0), Hints::default());
        assert_eq!(d, Decision { allowed: true, banner: false, sound: false, keep: true });
        // Critical ones get through.
        let mut critical = sample(2, "power", 0);
        critical.urgency = Urgency::Critical;
        let d = decide(&s, &noon(), &critical, Hints::default());
        assert!(d.banner && d.sound && d.keep);
        // Unless the person turned that off.
        s.do_not_disturb.allow_critical = false;
        let d = decide(&s, &noon(), &critical, Hints::default());
        assert!(!d.banner && !d.sound && d.keep);
    }

    #[test]
    fn sounds() {
        let s = Settings::default();
        let mut low = sample(1, "mail", 0);
        low.urgency = Urgency::Low;
        assert!(!decide(&s, &noon(), &low, Hints::default()).sound);
        let quiet = Hints { suppress_sound: true, ..Hints::default() };
        assert!(!decide(&s, &noon(), &sample(1, "mail", 0), quiet).sound);
    }
}
