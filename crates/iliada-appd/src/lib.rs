//! The Iliada application registry.
//!
//! Desktops find apps through freedesktop `.desktop` entries in
//! `$XDG_DATA_DIRS/applications`. `iliada-appd` keeps one exported entry per
//! GUI bundle in `/Applications`, whose `Exec` line starts the app through
//! `iliada-run`, so a click in the launcher always lands in a sandbox.
//!
//! Exported entries are named `iliada-<app-id>.desktop`; anything else in
//! the export directory is left alone. It also keeps the app list
//! ([`applist`]) equal to what is installed, so installing or trashing an app
//! is recorded without anyone editing a file. Phase 2 adds signature checks
//! and MIME registration here.

pub mod applist;
pub mod icon;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use iliada_bundle::Bundle;

const PREFIX: &str = "iliada-";

/// Quote one argument for a desktop-entry `Exec` key (Desktop Entry spec,
/// "The Exec key"): wrap in double quotes, backslash-escape `" ` $ \`, then
/// escape backslashes again for the string-value level.
pub fn exec_quote(arg: &str) -> String {
    let mut q = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            q.push('\\');
        }
        q.push(c);
    }
    q.push('"');
    q.replace('\\', "\\\\")
}

/// Strip characters that would break the key=value line format.
fn value(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Fields copied from the app's own desktop entry (shipped in `Resources/`).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Upstream {
    pub comment: Option<String>,
    pub generic_name: Option<String>,
    pub keywords: Option<String>,
    /// The Wayland app id / X11 class its windows use. Lets the desktop
    /// connect a running window to this entry (icon, dock pinning).
    pub wm_class: Option<String>,
}

pub fn upstream_entry(b: &Bundle) -> Upstream {
    let Some(file) = fs::read_dir(b.layout.resources())
        .ok()
        .and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "desktop")))
    else {
        return Upstream::default();
    };
    let mut u = Upstream {
        // GTK/Qt apps use the desktop file id as their app id by convention.
        wm_class: file.file_stem().map(|s| s.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let text = fs::read_to_string(&file).unwrap_or_default();
    let mut main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
            continue;
        }
        let Some((k, v)) = line.split_once('=').filter(|_| main) else { continue };
        let v = Some(value(v.trim()));
        match k.trim() {
            "Comment" => u.comment = v,
            "GenericName" => u.generic_name = v,
            "Keywords" => u.keywords = v,
            "StartupWMClass" => u.wm_class = v,
            _ => {}
        }
    }
    u
}

/// Is this a GUI app that belongs in the launcher? Command-line tools
/// (no desktop entry, no categories) are not exported.
pub fn is_gui(b: &Bundle) -> bool {
    let has_entry = fs::read_dir(b.layout.resources())
        .map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "desktop")))
        .unwrap_or(false);
    has_entry || !b.info.categories.is_empty()
}

pub fn desktop_entry(b: &Bundle, iliada_run: &Path, icon: Option<&Path>) -> String {
    let mut out = String::from("[Desktop Entry]\nType=Application\n");
    out += &format!("Name={}\n", value(&b.info.name));
    out += &format!(
        // %F: files opened with the app; iliada-run gives it exactly those.
        "Exec={} {} %F\n",
        exec_quote(&iliada_run.display().to_string()),
        exec_quote(&b.layout.root().display().to_string())
    );
    if let Some(p) = icon.filter(|p| p.is_file()) {
        out += &format!("Icon={}\n", value(&p.display().to_string()));
    }
    let up = upstream_entry(b);
    for (key, v) in [
        ("Comment", &up.comment),
        ("GenericName", &up.generic_name),
        ("Keywords", &up.keywords),
        ("StartupWMClass", &up.wm_class),
    ] {
        if let Some(v) = v {
            out += &format!("{key}={v}\n");
        }
    }
    if !b.info.categories.is_empty() {
        out += &format!("Categories={};\n", value(&b.info.categories.join(";")));
    }
    if !b.info.mime_types.is_empty() {
        out += &format!("MimeType={};\n", value(&b.info.mime_types.join(";")));
    }
    out +=
        &format!("X-Iliada-AppId={}\nX-Iliada-Bundle={}\n", b.info.id, value(&b.layout.root().display().to_string()));
    out
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub written: Vec<String>,
    pub removed: Vec<String>,
}

/// Make `<export>/applications` mirror the GUI bundles in `apps`.
/// A third-party icon that is a plain square picture gets the fallback tile
/// under `<export>/icons/`; icons with their own shape are used as they are.
pub fn sync(apps: &Path, export: &Path, iliada_run: &Path) -> Result<SyncReport> {
    let dir = export.join("applications");
    let theme_icons = export.join("icons/hicolor/scalable/apps");
    let bundle_icons = export.join("icons/app-bundles");
    fs::create_dir_all(&dir)?;
    fs::create_dir_all(&theme_icons)?;
    fs::create_dir_all(&bundle_icons)?;
    let mut report = SyncReport::default();
    let mut keep = BTreeSet::new();
    let mut keep_theme = BTreeSet::new();
    let mut keep_bundle = BTreeSet::new();
    let mut bundles: Vec<PathBuf> = fs::read_dir(apps)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "app")).collect())
        .unwrap_or_default();
    bundles.sort();
    for path in bundles {
        let Ok(b) = Bundle::open(&path) else { continue };
        if !is_gui(&b) {
            continue;
        }
        let icon = export_icon(&b, &path, &theme_icons, &bundle_icons, &mut keep_theme, &mut keep_bundle)?;
        let file = format!("{PREFIX}{}.desktop", b.info.id);
        let text = desktop_entry(&b, iliada_run, icon.as_deref());
        let target = dir.join(&file);
        if fs::read_to_string(&target).ok().as_deref() != Some(text.as_str()) {
            let tmp = dir.join(format!(".{file}.tmp"));
            fs::write(&tmp, &text)?;
            fs::rename(&tmp, &target)?;
            report.written.push(file.clone());
        }
        keep.insert(file);
    }
    for e in fs::read_dir(&dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(PREFIX) && name.ends_with(".desktop") && !keep.contains(&name) {
            fs::remove_file(e.path())?;
            report.removed.push(name);
        }
    }
    sweep_icons(&theme_icons, &keep_theme)?;
    sweep_icons(&bundle_icons, &keep_bundle)?;
    Ok(report)
}

fn export_icon(
    b: &Bundle,
    bundle: &Path,
    theme_icons: &Path,
    bundle_icons: &Path,
    keep_theme: &mut BTreeSet<String>,
    keep_bundle: &mut BTreeSet<String>,
) -> Result<Option<PathBuf>> {
    let Some(src) = b.info.icon.as_ref().map(|i| b.layout.contents().join(i)).filter(|p| p.is_file()) else {
        return Ok(None);
    };
    let Some(bundle_name) = bundle.file_name().map(|n| format!("{}.svg", n.to_string_lossy())) else {
        return Ok(Some(src));
    };
    let theme_name = format!("{PREFIX}{}.svg", b.info.id);
    let theme_dest = theme_icons.join(&theme_name);
    let bundle_dest = bundle_icons.join(&bundle_name);
    match icon::wrap_if_needed(&src) {
        Some(svg) => {
            keep_theme.insert(theme_name);
            keep_bundle.insert(bundle_name);
            write_if_changed(&theme_dest, &svg)?;
            write_if_changed(&bundle_dest, &svg)?;
            Ok(Some(theme_dest))
        }
        None => Ok(Some(src)),
    }
}

fn write_if_changed(path: &Path, text: &str) -> Result<()> {
    if fs::read_to_string(path).ok().as_deref() != Some(text) {
        fs::write(path, text)?;
    }
    Ok(())
}

fn sweep_icons(dir: &Path, keep: &BTreeSet<String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for e in fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".svg") && !keep.contains(&name) {
            fs::remove_file(e.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iliada_bundle::{BundleLayout, Info, IsolationTier, Permissions};
    use iliada_types::AppId;

    fn make(apps: &Path, name: &str, id: &str, gui: bool) -> PathBuf {
        let l = BundleLayout::in_dir(apps, name);
        l.create_dirs().unwrap();
        fs::write(l.executables().join("x"), "#!/bin/sh\n").unwrap();
        fs::write(l.resources().join("icon.svg"), "<svg/>").unwrap();
        if gui {
            fs::write(
                l.resources().join("org.gnome.Calculator.desktop"),
                "[Desktop Entry]\nName=Calculator\nComment=Perform arithmetic\nKeywords=math;sum;\n[Desktop Action x]\nComment=ignored\n",
            )
            .unwrap();
        }
        let info = Info {
            id: AppId::new(id).unwrap(),
            name: name.into(),
            version: "1".into(),
            executable: "x".into(),
            entry: Some("usr/bin/x".into()),
            args: vec![],
            env: Default::default(),
            icon: Some("Resources/icon.svg".into()),
            runtime: "iliada-base-1".into(),
            isolation: IsolationTier::MicroVm,
            network: false,
            categories: if gui { vec!["Utility".into()] } else { vec![] },
            mime_types: vec![],
            search: None,
        };
        Bundle::write_manifests(&l, &info, &Permissions::default(), None).unwrap();
        l.root().to_owned()
    }

    #[test]
    fn exports_gui_apps_only_and_follows_removal() {
        let d = tempfile::tempdir().unwrap();
        let (apps, export) = (d.path().join("Applications"), d.path().join("export"));
        fs::create_dir_all(&apps).unwrap();
        let calc = make(&apps, "Calculator", "org.debian.gnome-calculator", true);
        make(&apps, "jq", "org.debian.jq", false);
        let run = Path::new("/run/current-system/sw/bin/iliada-run");

        let r = sync(&apps, &export, run).unwrap();
        assert_eq!(r.written, ["iliada-org.debian.gnome-calculator.desktop"]);
        let text = fs::read_to_string(export.join("applications/iliada-org.debian.gnome-calculator.desktop")).unwrap();
        assert!(text.contains(&format!("Exec=\"/run/current-system/sw/bin/iliada-run\" \"{}\"", calc.display())));
        assert!(text.contains("Categories=Utility;"));
        assert!(text.contains("Comment=Perform arithmetic\n"), "only [Desktop Entry] keys are copied");
        assert!(text.contains("Keywords=math;sum;"));
        assert!(text.contains("StartupWMClass=org.gnome.Calculator"), "window ↔ entry matching");
        assert!(
            text.contains(&format!("Icon={}", calc.join("Contents/Resources/icon.svg").display())),
            "an icon with its own shape is used as it is: {text}"
        );
        assert!(!export.join("icons/hicolor/scalable/apps/iliada-org.debian.gnome-calculator.svg").exists());
        assert!(!export.join("icons/app-bundles/Calculator.app.svg").exists());

        // Idempotent.
        assert_eq!(sync(&apps, &export, run).unwrap(), SyncReport::default());

        // Uninstall removes the entry; unrelated files are untouched.
        fs::write(export.join("applications/other.desktop"), "x").unwrap();
        fs::remove_dir_all(&calc).unwrap();
        let r = sync(&apps, &export, run).unwrap();
        assert_eq!(r.removed, ["iliada-org.debian.gnome-calculator.desktop"]);
        assert!(export.join("applications/other.desktop").exists());
    }

    #[test]
    fn exec_quoting_follows_the_spec() {
        assert_eq!(exec_quote("/Applications/Hello World.app"), "\"/Applications/Hello World.app\"");
        assert_eq!(exec_quote("a\"b$c"), "\"a\\\\\"b\\\\$c\"");
    }

    #[test]
    fn square_icons_get_the_fallback_tile_until_they_have_a_shape() {
        let d = tempfile::tempdir().unwrap();
        let (apps, export) = (d.path().join("Applications"), d.path().join("export"));
        fs::create_dir_all(&apps).unwrap();
        make(&apps, "Calculator", "org.debian.gnome-calculator", true);
        let icon_path = BundleLayout::in_dir(&apps, "Calculator").resources().join("icon.svg");
        fs::write(
            &icon_path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="#333"/></svg>"##,
        )
        .unwrap();
        let run = Path::new("/run/current-system/sw/bin/iliada-run");
        sync(&apps, &export, run).unwrap();
        let tile = export.join("icons/hicolor/scalable/apps/iliada-org.debian.gnome-calculator.svg");
        let svg = fs::read_to_string(&tile).unwrap();
        assert!(svg.contains(icon::TILE_MARK), "a square picture sits on the fallback tile");
        assert!(export.join("icons/app-bundles/Calculator.app.svg").is_file(), "Files finds the same tile");
        let text = fs::read_to_string(export.join("applications/iliada-org.debian.gnome-calculator.desktop")).unwrap();
        assert!(text.contains(&format!("Icon={}", tile.display())));

        // An update that ships a shaped icon drops the tile again.
        fs::write(&icon_path, r#"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="30"/></svg>"#).unwrap();
        sync(&apps, &export, run).unwrap();
        assert!(!tile.exists());
        assert!(!export.join("icons/app-bundles/Calculator.app.svg").exists());
    }
}
