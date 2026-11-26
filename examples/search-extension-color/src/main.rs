//! An example Search extension: type `color` and a space, then `#ff0000`,
//! `rgb(255, 0, 0)` or a color name, and choose the hex, rgb or hsl value
//! to copy it.
//!
//! It speaks the extension protocol (docs/search-extensions.md): one JSON
//! request per line on stdin, `{"query":"…","id":N}`, and one reply per
//! line on stdout, `{"id":N,"results":[…]}`. It needs no internet and keeps
//! nothing.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { return };
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = req.get("id").and_then(Value::as_u64) else { continue };
        let query = req.get("query").and_then(Value::as_str).unwrap_or_default();
        let reply = json!({ "id": id, "results": results(query) });
        if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
            return;
        }
    }
}

/// The rows for a query: hex, rgb and hsl, each copied when chosen.
fn results(query: &str) -> Vec<Value> {
    let Some((r, g, b)) = parse(query) else { return Vec::new() };
    let (h, s, l) = hsl(r, g, b);
    [
        (format!("#{r:02x}{g:02x}{b:02x}"), "Hex"),
        (format!("rgb({r}, {g}, {b})"), "RGB"),
        (format!("hsl({h}, {s}%, {l}%)"), "HSL"),
    ]
    .into_iter()
    .map(|(value, kind)| json!({ "title": value, "subtitle": kind, "action": { "copy": value } }))
    .collect()
}

/// `#rrggbb`, `#rgb` (the `#` may be left out), `rgb(r, g, b)` or a name.
fn parse(query: &str) -> Option<(u8, u8, u8)> {
    let q = query.trim().to_lowercase();
    if let Some(inner) = q.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<u8> = inner
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|p| !p.is_empty())
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        return match parts[..] {
            [r, g, b] => Some((r, g, b)),
            _ => None,
        };
    }
    let compact = q.replace(' ', "");
    if let Some(rgb) = NAMES.iter().find(|(n, _)| *n == compact).map(|(_, c)| *c) {
        return Some(rgb);
    }
    let hex = q.strip_prefix('#').unwrap_or(&q);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        6 => Some((byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?)),
        3 => {
            let d = |i: usize| byte(&hex[i..=i]).map(|v| v * 17);
            Some((d(0)?, d(1)?, d(2)?))
        }
        _ => None,
    }
}

/// Hue in degrees, saturation and lightness in percent, rounded.
fn hsl(r: u8, g: u8, b: u8) -> (u16, u8, u8) {
    let (r, g, b) = (f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0, 0, (l * 100.0).round() as u8);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    ((h.round() as u16) % 360, (s * 100.0).round() as u8, (l * 100.0).round() as u8)
}

/// Common CSS color names.
const NAMES: [(&str, (u8, u8, u8)); 32] = [
    ("black", (0, 0, 0)),
    ("white", (255, 255, 255)),
    ("red", (255, 0, 0)),
    ("green", (0, 128, 0)),
    ("lime", (0, 255, 0)),
    ("blue", (0, 0, 255)),
    ("yellow", (255, 255, 0)),
    ("cyan", (0, 255, 255)),
    ("aqua", (0, 255, 255)),
    ("magenta", (255, 0, 255)),
    ("fuchsia", (255, 0, 255)),
    ("gray", (128, 128, 128)),
    ("grey", (128, 128, 128)),
    ("silver", (192, 192, 192)),
    ("maroon", (128, 0, 0)),
    ("olive", (128, 128, 0)),
    ("navy", (0, 0, 128)),
    ("purple", (128, 0, 128)),
    ("teal", (0, 128, 128)),
    ("orange", (255, 165, 0)),
    ("pink", (255, 192, 203)),
    ("brown", (165, 42, 42)),
    ("gold", (255, 215, 0)),
    ("indigo", (75, 0, 130)),
    ("violet", (238, 130, 238)),
    ("coral", (255, 127, 80)),
    ("salmon", (250, 128, 114)),
    ("turquoise", (64, 224, 208)),
    ("beige", (245, 245, 220)),
    ("crimson", (220, 20, 60)),
    ("rebeccapurple", (102, 51, 153)),
    ("skyblue", (135, 206, 235)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hex_rgb_and_names() {
        assert_eq!(parse("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse("FF8000"), Some((255, 128, 0)));
        assert_eq!(parse("#0f0"), Some((0, 255, 0)));
        assert_eq!(parse("rgb(255, 0, 0)"), Some((255, 0, 0)));
        assert_eq!(parse("rgb(1 2 3)"), Some((1, 2, 3)));
        assert_eq!(parse("Rebecca Purple"), Some((102, 51, 153)));
        for bad in ["", "#ff00", "rgb(256, 0, 0)", "rgb(1, 2)", "not a color", "#gg0000"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn hsl_values() {
        assert_eq!(hsl(255, 0, 0), (0, 100, 50));
        assert_eq!(hsl(0, 128, 0), (120, 100, 25));
        assert_eq!(hsl(128, 128, 128), (0, 0, 50));
        assert_eq!(hsl(102, 51, 153), (270, 50, 40));
    }

    #[test]
    fn replies_are_three_copy_results() {
        let r = results("red");
        let titles: Vec<&str> = r.iter().map(|v| v["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["#ff0000", "rgb(255, 0, 0)", "hsl(0, 100%, 50%)"]);
        assert!(r.iter().all(|v| v["action"]["copy"] == v["title"]));
        assert!(results("nothing").is_empty());
    }

    #[test]
    fn the_bundle_is_a_valid_extension() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../bundles/color.app/Contents");
        let text = std::fs::read_to_string(dir.join("Info.toml")).unwrap();
        let info: iliada_bundle::Info = toml::from_str(&text).unwrap();
        info.validate().unwrap();
        let search = info.search.unwrap();
        assert_eq!((search.keyword.as_str(), search.title.as_str()), ("color", "Color"));
        assert!(!info.network, "it needs no internet");
        assert_eq!(info.entry.as_deref(), Some("usr/bin/iliada-search-color"));
    }
}
