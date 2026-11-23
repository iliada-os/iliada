//! The computer's limits (`/etc/iliada/permissions.toml`, ADR 0007) as the
//! broker's system policy: an administrator's `deny` is never asked about.
//!
//! The broker is one system service for everyone, so it cannot read or
//! write a person's own `~/.config/iliada/permissions.toml`; its "Always"
//! grants, for apps that ask at run time over the broker socket, stay in
//! its own store (ADR 0007 says why). The file is read at every decision,
//! so edits apply at once. A file that cannot be read denies.

use std::path::Path;

use iliada_bundle::policy::{self, Key, Layer, Value};
use iliada_types::Capability;

/// Whether the computer's limits let `app` have `cap` at all.
pub fn allows(path: &Path, app: &str, cap: &Capability) -> bool {
    let loaded = match std::fs::read_to_string(path) {
        Ok(t) => policy::parse(&t, &path.display().to_string(), Layer::System),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    if loaded.broken {
        return false;
    }
    let Some(key) = Key::from_capability(cap) else { return true };
    let limit = |a: &str| loaded.apps.get(a).and_then(|m| m.get(&key)).copied();
    // The broker hands out whole folders: read-only is a "no" here.
    !matches!(limit(app), Some(Value::Deny | Value::ReadOnly))
        && !matches!(limit("*"), Some(Value::Deny | Value::ReadOnly))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iliada_types::{FileScope, KnownDir};

    #[test]
    fn limits_deny_and_a_broken_file_denies_everything() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("permissions.toml");
        assert!(allows(&p, "org.a", &Capability::Camera), "no file, no limits");
        std::fs::write(&p, "[all]\ncamera = \"deny\"\n[apps.\"org.a\"]\n\"files.pictures\" = \"read-only\"\n").unwrap();
        assert!(!allows(&p, "org.b", &Capability::Camera));
        assert!(allows(&p, "org.b", &Capability::Microphone));
        assert!(!allows(&p, "org.a", &Capability::Files(FileScope::KnownDir(KnownDir::Pictures))));
        std::fs::write(&p, "[all\n").unwrap();
        assert!(!allows(&p, "org.b", &Capability::Microphone));
    }
}
