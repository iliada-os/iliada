//! Layer 2: seeing the screen, and the keyboard.
//!
//! Screenshots come from the compositor's capture protocol
//! (ext-image-copy-capture, which cosmic-comp offers only to clients outside
//! a sandbox) through `grim`; window lists from ext-foreign-toplevel-list
//! through `lswt`. Typing uses the virtual-keyboard protocol (`wtype`), also
//! refused to sandboxed clients. The mouse is in `input.rs`.

use std::process::Stdio;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};
use tokio::process::Command;

/// Screenshots are scaled down to at most this width for the model.
pub const MAX_WIDTH: u32 = 1366;

/// A window on the screen (from the compositor, so also windows that have
/// no accessibility).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toplevel {
    pub title: String,
    pub app_id: String,
    pub identifier: String,
}

/// `lswt -j`: tolerant of the field names lswt's versions use.
pub fn parse_lswt(json_text: &str) -> Vec<Toplevel> {
    let v: Value = serde_json::from_str(json_text).unwrap_or(Value::Null);
    let list = v.get("toplevels").cloned().unwrap_or(v);
    list.as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let get = |keys: &[&str]| {
                keys.iter().find_map(|k| t.get(*k).and_then(Value::as_str)).unwrap_or_default().to_owned()
            };
            Toplevel {
                title: get(&["title"]),
                app_id: get(&["app-id", "app_id", "appid"]),
                identifier: get(&["identifier", "id"]),
            }
        })
        .collect()
}

pub async fn toplevels() -> Vec<Toplevel> {
    match Command::new("lswt").arg("-j").stdin(Stdio::null()).output().await {
        Ok(o) if o.status.success() => parse_lswt(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// Windows Homer must never be shown or clicked through the screen: the
/// prompts where the person decides (permissions, the portal's consent,
/// passwords) and Homer's own banner.
pub const PROTECTED_APP_IDS: [&str; 6] = [
    "org.iliada.Prompt",
    "iliada-prompt",
    "org.iliada.HomerBanner",
    "xdg-desktop-portal-cosmic",
    "com.system76.CosmicOsd",
    "cosmic-osd",
];

pub fn protected_open(tops: &[Toplevel]) -> Option<&Toplevel> {
    tops.iter().find(|t| PROTECTED_APP_IDS.iter().any(|p| t.app_id.eq_ignore_ascii_case(p)))
}

/// A screenshot for the model: PNG, scaled down; `scale` turns its pixels
/// back into the screen's logical pixels.
#[derive(Debug, Clone)]
pub struct Shot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

impl Shot {
    pub fn to_json(&self) -> Value {
        json!({ "media_type": "image/png", "width": self.width, "height": self.height,
                "data": base64::engine::general_purpose::STANDARD.encode(&self.png) })
    }
}

/// The whole screen (`window` empty) or one window, by its title or app id.
pub async fn screenshot(window: Option<&str>) -> Result<Shot> {
    let mut cmd = Command::new("grim");
    // One image pixel = one logical pixel, so clicks map back exactly.
    cmd.args(["-s", "1", "-t", "png"]);
    if let Some(w) = window.filter(|w| !w.trim().is_empty()) {
        let tops = toplevels().await;
        let needle = w.to_lowercase();
        let t = tops
            .iter()
            .find(|t| t.title.to_lowercase().contains(&needle) || t.app_id.to_lowercase() == needle)
            .with_context(|| format!("no window called {w:?} is open"))?;
        if PROTECTED_APP_IDS.iter().any(|p| t.app_id.eq_ignore_ascii_case(p)) {
            bail!("that window is one only the person may look at");
        }
        cmd.args(["-T", &t.identifier]);
    }
    cmd.arg("-");
    let out = cmd.stdin(Stdio::null()).output().await.context("taking a screenshot (grim)")?;
    if !out.status.success() {
        bail!("the screenshot failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    tokio::task::spawn_blocking(move || downscale(&out.stdout, MAX_WIDTH)).await?
}

/// Scales a PNG down to at most `max_width` wide.
pub fn downscale(png: &[u8], max_width: u32) -> Result<Shot> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).context("reading the screenshot")?;
    let (w, h) = (img.width(), img.height());
    if w <= max_width {
        return Ok(Shot { png: png.to_vec(), width: w, height: h, scale: 1.0 });
    }
    let nh = ((h as f64) * (max_width as f64) / (w as f64)).round().max(1.0) as u32;
    let small = img.resize_exact(max_width, nh, image::imageops::FilterType::Triangle);
    let mut out = Vec::new();
    small.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(Shot { png: out, width: max_width, height: nh, scale: w as f64 / max_width as f64 })
}

/// Types text with the virtual keyboard (any language: wtype sends a keymap
/// with exactly the characters it needs).
pub async fn type_text(text: &str) -> Result<()> {
    anyhow::ensure!(!text.is_empty(), "no text to type");
    run_wtype(&["--", text]).await
}

/// "ctrl+shift+t", "Enter", "alt+F4": modifiers and one key.
pub fn key_args(combo: &str) -> Result<Vec<String>> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let (key, mods) = parts.split_last().context("no key given")?;
    let mut args = Vec::new();
    let mut names = Vec::new();
    for m in mods {
        let name = match m.to_lowercase().as_str() {
            "ctrl" | "control" => "ctrl",
            "shift" => "shift",
            "alt" => "alt",
            "super" | "logo" | "meta" | "win" | "cmd" => "logo",
            "altgr" => "altgr",
            other => bail!("{other:?} is not a modifier key (ctrl, shift, alt, super)"),
        };
        names.push(name);
        args.extend(["-M".to_owned(), name.to_owned()]);
    }
    let k = match key.to_lowercase().as_str() {
        "enter" | "return" => "Return".to_owned(),
        "esc" | "escape" => "Escape".to_owned(),
        "tab" => "Tab".to_owned(),
        "space" => "space".to_owned(),
        "backspace" => "BackSpace".to_owned(),
        "delete" | "del" => "Delete".to_owned(),
        "up" => "Up".to_owned(),
        "down" => "Down".to_owned(),
        "left" => "Left".to_owned(),
        "right" => "Right".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        "pageup" => "Prior".to_owned(),
        "pagedown" => "Next".to_owned(),
        k if k.len() > 1 && k.starts_with('f') && k[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) => {
            k.to_uppercase()
        }
        k if k.chars().count() == 1 => k.to_owned(),
        _ => key.to_string(),
    };
    args.extend(["-k".to_owned(), k]);
    for n in names.iter().rev() {
        args.extend(["-m".to_owned(), (*n).to_owned()]);
    }
    Ok(args)
}

/// Super+Escape is the person's stop key: Homer may not press it, nor the
/// keys that lock the screen or log out.
pub fn forbidden_combo(combo: &str) -> bool {
    let c = combo.to_lowercase().replace(' ', "");
    let has_super = ["super", "logo", "meta", "win", "cmd"].iter().any(|m| c.contains(m));
    (has_super && (c.ends_with("escape") || c.ends_with("esc") || c.ends_with("+l")))
        || (c.contains("ctrl") && c.contains("alt") && (c.ends_with("delete") || c.ends_with("del")))
}

pub async fn key_combo(combo: &str) -> Result<()> {
    anyhow::ensure!(!forbidden_combo(combo), "Homer may not press {combo}");
    let args = key_args(combo)?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_wtype(&refs).await
}

async fn run_wtype(args: &[&str]) -> Result<()> {
    let out = Command::new("wtype").args(args).stdin(Stdio::null()).output().await.context("typing (wtype)")?;
    if !out.status.success() {
        bail!("typing failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_combos_become_wtype_arguments() {
        assert_eq!(
            key_args("ctrl+shift+t").unwrap(),
            ["-M", "ctrl", "-M", "shift", "-k", "t", "-m", "shift", "-m", "ctrl"]
        );
        assert_eq!(key_args("Enter").unwrap(), ["-k", "Return"]);
        assert_eq!(key_args("alt+F4").unwrap(), ["-M", "alt", "-k", "F4", "-m", "alt"]);
        assert!(key_args("hyper+x").is_err());
        assert!(forbidden_combo("Super+Escape") && forbidden_combo("super+l") && forbidden_combo("ctrl+alt+del"));
        assert!(!forbidden_combo("ctrl+l") && !forbidden_combo("Escape"));
    }

    #[test]
    fn windows_are_read_from_lswt_and_prompts_are_protected() {
        let t = parse_lswt(
            r#"{"toplevels":[{"title":"Inbox","app-id":"google-chrome","identifier":"a1"},
                             {"title":"Allow?","app-id":"org.iliada.Prompt","identifier":"b2"}]}"#,
        );
        assert_eq!(t[0], Toplevel { title: "Inbox".into(), app_id: "google-chrome".into(), identifier: "a1".into() });
        assert_eq!(protected_open(&t).map(|t| t.identifier.as_str()), Some("b2"));
        assert!(protected_open(&t[..1]).is_none());
    }

    #[test]
    fn big_screenshots_are_scaled_down() {
        let img = image::RgbImage::from_pixel(2732, 100, image::Rgb([10, 20, 30]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let s = downscale(&png, MAX_WIDTH).unwrap();
        assert_eq!((s.width, s.height), (1366, 50));
        assert!((s.scale - 2.0).abs() < 1e-9);
        assert_eq!(s.to_json()["media_type"], "image/png");
    }
}
