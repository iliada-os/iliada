//! Asking for the administrator's password before a sensitive change, as a
//! Mac does: polkit checks the program that asked, and the desktop's
//! password window (or `pkttyagent` in a terminal) asks the person. Being
//! in `wheel` is not enough on its own: an app or Homer acting for you
//! can't type the password.

use anyhow::{Context, Result, bail};

/// polkit's checker. The service's PATH has polkit (nix/modules/system.nix).
const PKCHECK: &str = "pkcheck";

/// When the process started, in clock ticks since boot (field 22 of
/// `/proc/<pid>/stat`): polkit identifies a process by pid *and* start time,
/// so a reused pid can't borrow someone's answer.
pub fn start_time(stat: &str) -> Option<u64> {
    // The name (field 2) is in parentheses and may contain spaces.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Waits until the person gives the administrator password for `action`
/// (polkit remembers it for a few minutes), or fails if they don't.
pub async fn authorize(action: &str, pid: u32, uid: u32) -> Result<()> {
    if uid == 0 {
        return Ok(());
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).context("the program that asked has stopped")?;
    let start = start_time(&stat).context("can't read when the program that asked started")?;
    let status = tokio::process::Command::new(PKCHECK)
        .args(["--action-id", action, "--process", &format!("{pid},{start},{uid}"), "--allow-user-interaction"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .context("can't ask for the administrator password (polkit is missing)")?;
    match status.code() {
        Some(0) => Ok(()),
        Some(3) => bail!("Cancelled: the administrator password is needed for this."),
        Some(1) | Some(2) => bail!("Not allowed: this needs the administrator password."),
        _ => bail!("Couldn't ask for the administrator password."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_time_is_field_22_even_with_spaces_in_the_name() {
        let stat = "4242 (my (odd) app) S 1 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 987654 1234 56";
        assert_eq!(start_time(stat), Some(987654));
        assert_eq!(start_time(&std::fs::read_to_string("/proc/self/stat").unwrap()).map(|t| t > 0), Some(true));
        assert_eq!(start_time("garbage"), None);
    }
}
