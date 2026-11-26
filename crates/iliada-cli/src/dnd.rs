//! `iliada dnd on|off|toggle|status [--until HH:MM | --for 1h]`: Do Not
//! Disturb (no banners and no sounds; notifications still go to the
//! Notification Center, Super+N). The same switch as the panel,
//! Super+Shift+N and Settings → Notifications.

use anyhow::Result;
use clap::ValueEnum;
use iliada_notify::dnd::{self, Why};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Switch {
    On,
    Off,
    Toggle,
    Status,
}

#[derive(Debug, clap::Args)]
pub struct Args {
    /// on, off, toggle or status (default: status).
    #[arg(value_enum, default_value = "status")]
    switch: Switch,
    /// With on: until this time (24-hour, e.g. 07:30; tomorrow if it has passed today).
    #[arg(long, value_name = "HH:MM")]
    until: Option<String>,
    /// With on: for this long (1h, 30m, 1h30m).
    #[arg(long = "for", value_name = "LENGTH")]
    length: Option<String>,
}

/// One line saying whether it is on.
pub fn describe(status: Option<Why>) -> String {
    match status {
        None => "Do Not Disturb is off.".into(),
        Some(Why::Manual) => "Do Not Disturb is on, until you turn it off.".into(),
        Some(Why::Until { until }) => format!("Do Not Disturb is on until {}.", dnd::clock_time(until)),
        Some(Why::Schedule { until }) => {
            format!("Do Not Disturb is on until {} (your schedule).", dnd::clock_time(until))
        }
    }
}

pub fn run(args: Args) -> Result<()> {
    let ends = dnd::end_from(args.until.as_deref(), args.length.as_deref())?;
    if ends.is_some() && args.switch != Switch::On {
        anyhow::bail!("--until and --for go with `iliada dnd on`");
    }
    let status = match args.switch {
        Switch::On => dnd::switch(true, ends)?,
        Switch::Off => dnd::switch(false, None)?,
        Switch::Toggle => dnd::toggle()?,
        Switch::Status => dnd::current(),
    };
    println!("{}", describe(status));
    Ok(())
}
