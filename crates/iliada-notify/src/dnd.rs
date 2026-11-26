//! Do Not Disturb: whether it is on now, and switching it.
//!
//! It is on when switched on by hand (until a time, or until switched off)
//! or, with a schedule, every day between two times (across midnight when
//! the end is earlier than the start). Switching it off while the schedule
//! has it on keeps it off until that stretch ends.
//!
//! While it is on, banners and sounds are held back; notifications still
//! go to the Notification Center. Critical ones get through unless the
//! person turned that off ([`bypasses`]).

use serde::{Deserialize, Serialize};

use crate::Urgency;
use crate::settings::DndSettings;

const DAY: u64 = 24 * 60 * 60;

/// The time, as Do Not Disturb needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// Unix seconds.
    pub now: u64,
    /// Seconds since local midnight.
    pub local_secs: u32,
}

impl Clock {
    pub fn now() -> Clock {
        use chrono::Timelike;
        let t = chrono::Local::now();
        Clock { now: u64::try_from(t.timestamp()).unwrap_or(0), local_secs: t.num_seconds_from_midnight() }
    }
}

/// Why Do Not Disturb is on, and until when (Unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum Why {
    /// Switched on, until switched off.
    Manual,
    /// Switched on until a time.
    Until { until: u64 },
    /// The schedule.
    Schedule { until: u64 },
}

impl Why {
    pub fn until(self) -> Option<u64> {
        match self {
            Why::Manual => None,
            Why::Until { until } | Why::Schedule { until } => Some(until),
        }
    }
}

/// "23:00" → minutes since midnight.
pub fn parse_time(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// "1h", "30m", "1h30m", "90" (minutes), "2 hours" → seconds.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase().replace(' ', "");
    if s.is_empty() {
        return None;
    }
    if let Ok(minutes) = s.parse::<u64>() {
        return (minutes > 0).then_some(minutes * 60);
    }
    let mut total = 0u64;
    let mut num = String::new();
    let mut rest = s.as_str();
    while !rest.is_empty() {
        let c = rest.chars().next()?;
        if c.is_ascii_digit() {
            num.push(c);
            rest = &rest[1..];
            continue;
        }
        let unit_len = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());
        let unit = &rest[..unit_len];
        rest = &rest[unit_len..];
        let n: u64 = num.parse().ok()?;
        num.clear();
        total += n * match unit {
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
            "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "d" | "day" | "days" => DAY,
            _ => return None,
        };
    }
    (num.is_empty() && total > 0).then_some(total)
}

/// The next time the clock reads `minutes` (today, or tomorrow when that
/// has passed), in Unix seconds.
pub fn next_time(c: &Clock, minutes: u32) -> u64 {
    let target = u64::from(minutes) * 60;
    let now = u64::from(c.local_secs);
    let ahead = (target + DAY - now) % DAY;
    c.now + if ahead == 0 { DAY } else { ahead }
}

/// When the schedule has it on now, when that ends.
pub fn in_schedule(s: &DndSettings, c: &Clock) -> Option<u64> {
    if !s.scheduled {
        return None;
    }
    let (from, to) = (parse_time(&s.from)?, parse_time(&s.to)?);
    if from == to {
        return None;
    }
    let m = c.local_secs / 60;
    let on = if from < to { (from..to).contains(&m) } else { m >= from || m < to };
    on.then(|| next_time(c, to))
}

/// Whether Do Not Disturb is on now, and why.
pub fn status(s: &DndSettings, c: &Clock) -> Option<Why> {
    if s.on {
        match s.until {
            None => return Some(Why::Manual),
            Some(until) if c.now < until => return Some(Why::Until { until }),
            // Ended: only the schedule can have it on now.
            Some(_) => {}
        }
    }
    let end = in_schedule(s, c)?;
    if s.paused_until.is_some_and(|p| c.now < p) {
        return None;
    }
    Some(Why::Schedule { until: end })
}

/// Switch it on, until a time or until switched off.
pub fn turn_on(s: &mut DndSettings, until: Option<u64>) {
    s.on = true;
    s.until = until;
    s.paused_until = None;
}

/// Switch it off; if the schedule has it on now, off until that ends.
pub fn turn_off(s: &mut DndSettings, c: &Clock) {
    s.on = false;
    s.until = None;
    s.paused_until = in_schedule(s, c);
}

/// Whether a notification gets through while Do Not Disturb is on.
pub fn bypasses(s: &DndSettings, urgency: Urgency) -> bool {
    urgency == Urgency::Critical && s.allow_critical
}

/// Whether it is on now for the person, and why.
pub fn current() -> Option<Why> {
    status(&crate::settings::Settings::load_user().do_not_disturb, &Clock::now())
}

/// Switch it on (until a time, or until switched off) or off, for the
/// person: what the panel, Super+Shift+N, Settings, `iliada dnd` and Homer
/// do. The notification service reads the file for every notification.
pub fn switch(on: bool, until: Option<u64>) -> anyhow::Result<Option<Why>> {
    let clock = Clock::now();
    let s = crate::settings::Settings::update(|s| {
        if on {
            turn_on(&mut s.do_not_disturb, until);
        } else {
            turn_off(&mut s.do_not_disturb, &clock);
        }
    })?;
    Ok(status(&s.do_not_disturb, &clock))
}

/// Switch it the other way (Super+Shift+N, the panel's button).
pub fn toggle() -> anyhow::Result<Option<Why>> {
    switch(current().is_none(), None)
}

/// When it should end, from `--until HH:MM` or `--for 1h` (at most one).
pub fn end_from(until: Option<&str>, length: Option<&str>) -> anyhow::Result<Option<u64>> {
    let clock = Clock::now();
    match (until, length) {
        (Some(_), Some(_)) => anyhow::bail!("give --until or --for, not both"),
        (Some(t), None) => {
            let m = parse_time(t).ok_or_else(|| anyhow::anyhow!("{t:?} is not a time like 07:30"))?;
            Ok(Some(next_time(&clock, m)))
        }
        (None, Some(d)) => {
            let secs = parse_duration(d).ok_or_else(|| anyhow::anyhow!("{d:?} is not a length like 1h or 30m"))?;
            Ok(Some(clock.now + secs))
        }
        (None, None) => Ok(None),
    }
}

/// A Unix time as the local clock shows it ("07:00").
pub fn clock_time(unix: u64) -> String {
    use chrono::TimeZone;
    i64::try_from(unix)
        .ok()
        .and_then(|t| chrono::Local.timestamp_opt(t, 0).single())
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock at `hh:mm` local, on a day starting at Unix second 1,000,000.
    fn at(hh: u32, mm: u32) -> Clock {
        let local_secs = (hh * 60 + mm) * 60;
        Clock { now: 1_000_000 + u64::from(local_secs), local_secs }
    }

    fn scheduled(from: &str, to: &str) -> DndSettings {
        DndSettings { scheduled: true, from: from.into(), to: to.into(), ..DndSettings::default() }
    }

    #[test]
    fn times_and_durations() {
        assert_eq!(parse_time("23:00"), Some(23 * 60));
        assert_eq!(parse_time("7:05"), Some(7 * 60 + 5));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("noon"), None);
        assert_eq!(parse_duration("1h"), Some(3600));
        assert_eq!(parse_duration("1h30m"), Some(5400));
        assert_eq!(parse_duration("45m"), Some(2700));
        assert_eq!(parse_duration("90"), Some(5400));
        assert_eq!(parse_duration("2 hours"), Some(7200));
        assert_eq!(parse_duration("1d"), Some(DAY));
        for bad in ["", "0", "h", "1x", "1h30", "soon"] {
            assert_eq!(parse_duration(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn schedule_across_midnight() {
        let s = scheduled("23:00", "07:00");
        assert_eq!(status(&s, &at(22, 59)), None);
        // 23:30: on until 07:00 tomorrow.
        let late = at(23, 30);
        assert_eq!(status(&s, &late), Some(Why::Schedule { until: late.now + (7 * 60 + 30) * 60 }));
        // 03:00: on until 07:00 today.
        let night = at(3, 0);
        assert_eq!(status(&s, &night), Some(Why::Schedule { until: night.now + 4 * 3600 }));
        assert_eq!(status(&s, &at(7, 0)), None);
        assert_eq!(status(&s, &at(12, 0)), None);
        // Within one day.
        let day = scheduled("09:00", "17:00");
        assert!(status(&day, &at(9, 0)).is_some() && status(&day, &at(16, 59)).is_some());
        assert!(status(&day, &at(8, 59)).is_none() && status(&day, &at(17, 0)).is_none());
        // Off, or the same start and end: never.
        assert_eq!(status(&DndSettings { scheduled: false, ..s.clone() }, &at(23, 30)), None);
        assert_eq!(status(&scheduled("08:00", "08:00"), &at(8, 0)), None);
    }

    #[test]
    fn until_a_time_and_for_a_while() {
        let c = at(14, 0);
        let mut s = DndSettings::default();
        // --until 15:30: today.
        turn_on(&mut s, Some(next_time(&c, parse_time("15:30").unwrap())));
        assert_eq!(s.until, Some(c.now + 90 * 60));
        assert_eq!(status(&s, &c), Some(Why::Until { until: c.now + 90 * 60 }));
        // --until 09:00: tomorrow.
        assert_eq!(next_time(&c, 9 * 60), c.now + 19 * 3600);
        // --for 1h: on, then off by itself.
        turn_on(&mut s, Some(c.now + parse_duration("1h").unwrap()));
        assert!(status(&s, &Clock { now: c.now + 3599, local_secs: c.local_secs + 3599 }).is_some());
        assert_eq!(status(&s, &Clock { now: c.now + 3600, local_secs: c.local_secs + 3600 }), None);
        // On until switched off.
        turn_on(&mut s, None);
        assert_eq!(status(&s, &c), Some(Why::Manual));
        turn_off(&mut s, &c);
        assert_eq!(status(&s, &c), None);
    }

    #[test]
    fn switching_off_during_the_schedule_lasts_until_it_ends() {
        let mut s = scheduled("23:00", "07:00");
        let night = at(1, 0);
        assert!(status(&s, &night).is_some());
        turn_off(&mut s, &night);
        assert_eq!(status(&s, &night), None);
        // Still off at 06:00, the same night.
        assert_eq!(status(&s, &Clock { now: night.now + 5 * 3600, local_secs: 6 * 3600 }), None);
        // The next night it is on again.
        let next = Clock { now: night.now + DAY, local_secs: night.local_secs };
        assert!(status(&s, &next).is_some());
        // Switching on by hand clears the pause.
        turn_on(&mut s, None);
        assert_eq!(s.paused_until, None);
    }

    #[test]
    fn critical_notifications_get_through_unless_turned_off() {
        let mut s = DndSettings::default();
        assert!(bypasses(&s, Urgency::Critical));
        assert!(!bypasses(&s, Urgency::Normal) && !bypasses(&s, Urgency::Low));
        s.allow_critical = false;
        assert!(!bypasses(&s, Urgency::Critical));
    }
}
