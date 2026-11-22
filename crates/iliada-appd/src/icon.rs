//! Put a third-party app icon on Iliada's navy squircle tile.
//!
//! Iliada's own icons already include `id="iliada-tile"` and are left alone.
//! Everything else is drawn inside the same superellipse (n = 5) the
//! branding generator uses, so the dock and the app library look like one set.

use std::path::Path;

/// Marker in Iliada's own tiles. Presence means the icon is already wrapped.
pub const TILE_MARK: &str = "id=\"iliada-tile\"";

const TILE_R: f64 = 108.0;
const MAX_BYTES: usize = 2 * 1024 * 1024;

/// Wrap `src` in the navy tile. `None` when the file is already an Iliada
/// tile, missing, too large, or not a picture we can embed.
pub fn wrap_if_needed(src: &Path) -> Option<String> {
    let bytes = std::fs::read(src).ok()?;
    if bytes.len() > MAX_BYTES {
        return None;
    }
    if looks_tiled(&bytes) {
        return None;
    }
    let mime = mime_of(src, &bytes)?;
    Some(wrap_bytes(&bytes, mime))
}

pub fn looks_tiled(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|s| s.contains(TILE_MARK))
}

fn mime_of(path: &Path, bytes: &[u8]) -> Option<&'static str> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "ico" => Some("image/x-icon"),
        _ => sniff(bytes),
    }
}

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") {
        Some("image/svg+xml")
    } else {
        None
    }
}

fn wrap_bytes(bytes: &[u8], mime: &str) -> String {
    let sq = squircle_d(128.0, 128.0, TILE_R);
    let rim = squircle_d(128.0, 128.0, TILE_R - 0.75);
    let href = format!("data:{mime};base64,{}", base64(bytes));
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">
  <title>app icon</title>
  <defs>
    <radialGradient id="il-navy" cx="128" cy="72" r="190" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#22396A"/>
      <stop offset="0.6" stop-color="#14254A"/>
      <stop offset="1" stop-color="#0B1630"/>
    </radialGradient>
    <linearGradient id="il-rim" x1="0" y1="20" x2="0" y2="236" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.18"/>
      <stop offset="0.45" stop-color="#FFFFFF" stop-opacity="0"/>
      <stop offset="1" stop-color="#000000" stop-opacity="0.22"/>
    </linearGradient>
    <filter id="il-shadow" x="-25%" y="-10%" width="150%" height="155%">
      <feGaussianBlur in="SourceAlpha" stdDeviation="6"/>
      <feOffset dx="0" dy="6"/>
      <feComponentTransfer><feFuncA type="linear" slope="0.34"/></feComponentTransfer>
    </filter>
    <clipPath id="iliada-tile"><path d="{sq}"/></clipPath>
  </defs>
  <path fill="#0B1630" filter="url(#il-shadow)" d="{sq}"/>
  <path fill="url(#il-navy)" d="{sq}"/>
  <path fill="none" stroke="url(#il-rim)" stroke-width="1.5" d="{rim}"/>
  <g clip-path="url(#iliada-tile)">
    <image href="{href}" x="40" y="40" width="176" height="176" preserveAspectRatio="xMidYMid meet"/>
  </g>
</svg>
"##
    )
}

/// Superellipse n = 5, same geometry as `nix/branding/tools/generate.py`.
fn squircle_d(cx: f64, cy: f64, r: f64) -> String {
    const N: f64 = 5.0;
    const SEG: usize = 48;
    let mut pts = [(0.0, 0.0); SEG];
    for (i, p) in pts.iter_mut().enumerate() {
        let t = 2.0 * std::f64::consts::PI * i as f64 / SEG as f64;
        let c = t.cos();
        let s = t.sin();
        *p = (cx + r * c.abs().powf(2.0 / N).copysign(c), cy + r * s.abs().powf(2.0 / N).copysign(s));
    }
    let mut d = format!("M{:.2} {:.2} ", pts[0].0, pts[0].1);
    for i in 0..SEG {
        let p0 = pts[(i + SEG - 1) % SEG];
        let p1 = pts[i];
        let p2 = pts[(i + 1) % SEG];
        let p3 = pts[(i + 2) % SEG];
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        d.push_str(&format!("C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} ", c1.0, c1.1, c2.0, c2.1, p2.0, p2.1));
    }
    d.push('Z');
    d
}

fn base64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8) | u32::from(data[i + 2]);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    match data.len() - i {
        1 => {
            let n = u32::from(data[i]) << 16;
            out.push(T[((n >> 18) & 63) as usize] as char);
            out.push(T[((n >> 12) & 63) as usize] as char);
            out.push_str("==");
        }
        2 => {
            let n = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8);
            out.push(T[((n >> 18) & 63) as usize] as char);
            out.push(T[((n >> 12) & 63) as usize] as char);
            out.push(T[((n >> 6) & 63) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn wraps_a_plain_svg_and_skips_an_iliada_tile() {
        let d = tempfile::tempdir().unwrap();
        let plain = d.path().join("plain.svg");
        fs::write(&plain, "<svg xmlns='http://www.w3.org/2000/svg'><circle r='8'/></svg>").unwrap();
        let wrapped = wrap_if_needed(&plain).expect("plain icons get a tile");
        assert!(wrapped.contains(TILE_MARK), "clip path id is the skip marker");
        assert!(wrapped.contains("data:image/svg+xml;base64,"), "original picture is embedded");
        assert!(wrapped.contains("#14254A"), "navy tile");

        let tiled = d.path().join("tiled.svg");
        let mut f = fs::File::create(&tiled).unwrap();
        writeln!(f, "<svg><clipPath {TILE_MARK}></clipPath></svg>").unwrap();
        assert!(wrap_if_needed(&tiled).is_none(), "Iliada tiles are not wrapped again");
    }

    #[test]
    fn base64_matches_the_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
