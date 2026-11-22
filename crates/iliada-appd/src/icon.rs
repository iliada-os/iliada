//! Give a square third-party app icon a soft rounded tile.
//!
//! Iliada's own icons are freestanding shapes with a drop shadow, and most
//! third-party icons already have a shape of their own (a transparent margin
//! around a circle, a squircle, a glyph). Those are shown as they are. Only
//! a plain square or rectangular picture with nothing transparent at its
//! corners gets the fallback tile, so it does not sit in the dock as a hard
//! square. The tile uses the same superellipse (n = 5) as the branding
//! generator.

use std::io::Read;
use std::path::Path;

/// Marker in the fallback tile. Presence means the icon is already wrapped.
pub const TILE_MARK: &str = "id=\"iliada-tile\"";

const TILE_R: f64 = 102.0;
const MAX_BYTES: usize = 2 * 1024 * 1024;

/// The tiled icon for `src`, or `None` when the icon already has its own
/// shape, is already tiled, is missing or too large, or is not a picture we
/// can read.
pub fn wrap_if_needed(src: &Path) -> Option<String> {
    let bytes = std::fs::read(src).ok()?;
    if bytes.len() > MAX_BYTES || looks_tiled(&bytes) {
        return None;
    }
    let mime = mime_of(src, &bytes)?;
    full_bleed(&bytes, mime).then(|| wrap_bytes(&bytes, mime))
}

pub fn looks_tiled(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|s| s.contains(TILE_MARK))
}

/// True when the picture fills its whole box: nothing transparent at the
/// corners. Formats we cannot check are treated as already shaped.
pub fn full_bleed(bytes: &[u8], mime: &str) -> bool {
    match mime {
        "image/jpeg" => true,
        "image/png" => png_corners_opaque(bytes).unwrap_or(false),
        "image/svg+xml" => std::str::from_utf8(bytes).is_ok_and(svg_has_backdrop),
        _ => false,
    }
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

/// Are the four corner pixels of a PNG opaque? `None` for PNGs we do not
/// decode (16-bit, palette with transparency, interlaced).
fn png_corners_opaque(bytes: &[u8]) -> Option<bool> {
    let mut chunks = Vec::new();
    let mut pos = 8;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data = bytes.get(pos + 8..pos + 8 + len)?;
        chunks.push((kind, data));
        pos += 12 + len;
    }
    let (_, ihdr) = chunks.iter().find(|(k, _)| *k == b"IHDR")?;
    let width = u32::from_be_bytes(ihdr.get(0..4)?.try_into().ok()?) as usize;
    let height = u32::from_be_bytes(ihdr.get(4..8)?.try_into().ok()?) as usize;
    let (depth, color, interlace) = (*ihdr.get(8)?, *ihdr.get(9)?, *ihdr.get(12)?);
    let has_trns = chunks.iter().any(|(k, _)| *k == b"tRNS");
    // Grey, RGB or palette without a transparency chunk: opaque everywhere.
    if matches!(color, 0 | 2 | 3) && !has_trns {
        return Some(true);
    }
    // Only 8-bit grey+alpha and RGBA are decoded; the rest count as shaped.
    let bpp = match (color, depth, interlace) {
        (4, 8, 0) => 2,
        (6, 8, 0) => 4,
        _ => return None,
    };
    if width == 0 || height == 0 || width.checked_mul(height)? > 4096 * 4096 {
        return None;
    }
    let idat: Vec<u8> = chunks.iter().filter(|(k, _)| *k == b"IDAT").flat_map(|(_, d)| d.iter().copied()).collect();
    let stride = width * bpp;
    let mut raw = Vec::with_capacity((stride + 1) * height);
    flate2::read::ZlibDecoder::new(&idat[..]).take(((stride + 1) * height) as u64).read_to_end(&mut raw).ok()?;
    if raw.len() < (stride + 1) * height {
        return None;
    }
    let mut prev = vec![0u8; stride];
    let mut row = vec![0u8; stride];
    let alpha_at = |row: &[u8], x: usize| row[x * bpp + bpp - 1];
    let mut opaque = true;
    for y in 0..height {
        let line = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        unfilter(line[0], &line[1..], &prev, &mut row, bpp)?;
        if y == 0 || y == height - 1 {
            opaque &= alpha_at(&row, 0) > 250 && alpha_at(&row, width - 1) > 250;
        }
        std::mem::swap(&mut prev, &mut row);
    }
    Some(opaque)
}

fn unfilter(kind: u8, line: &[u8], prev: &[u8], out: &mut [u8], bpp: usize) -> Option<()> {
    for i in 0..line.len() {
        let a = if i >= bpp { out[i - bpp] } else { 0 };
        let b = prev[i];
        let c = if i >= bpp { prev[i - bpp] } else { 0 };
        let pred = match kind {
            0 => 0,
            1 => a,
            2 => b,
            3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
            4 => {
                let p = i16::from(a) + i16::from(b) - i16::from(c);
                let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
                if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                }
            }
            _ => return None,
        };
        out[i] = line[i].wrapping_add(pred);
    }
    Some(())
}

/// Does the SVG paint a backdrop over its whole box first: a square-cornered
/// `<rect>` the size of the view box?
fn svg_has_backdrop(svg: &str) -> bool {
    let body = match (svg.find("<defs"), svg.find("</defs>")) {
        (Some(a), Some(b)) if b > a => format!("{}{}", &svg[..a], &svg[b..]),
        _ => svg.to_string(),
    };
    let Some(root) = body.find("<svg").and_then(|i| body[i..].find('>').map(|j| &body[i..i + j])) else {
        return false;
    };
    let (vw, vh) = match attr(root, "viewBox") {
        Some(vb) => {
            let n: Vec<f64> = vb.split([' ', ',']).filter_map(|v| v.parse().ok()).collect();
            if n.len() != 4 {
                return false;
            }
            (n[2], n[3])
        }
        None => match (num(attr(root, "width")), num(attr(root, "height"))) {
            (Some(w), Some(h)) => (w, h),
            _ => return false,
        },
    };
    let shapes = ["<rect", "<path", "<circle", "<ellipse", "<polygon", "<polyline", "<line", "<image", "<use", "<text"];
    let Some((at, tag)) = shapes.iter().filter_map(|t| body.find(t).map(|i| (i, *t))).min_by_key(|(i, _)| *i) else {
        return false;
    };
    if tag != "<rect" {
        return false;
    }
    let el = &body[at..body[at..].find('>').map_or(body.len(), |j| at + j)];
    let covers = |v: Option<&str>, full: f64| v == Some("100%") || num(v).is_some_and(|n| (n - full).abs() < 0.5);
    let at_origin = |v: Option<&str>| v.is_none() || num(v) == Some(0.0);
    let square = num(attr(el, "rx")).unwrap_or(0.0) < 1.0 && num(attr(el, "ry")).unwrap_or(0.0) < 1.0;
    square
        && at_origin(attr(el, "x"))
        && at_origin(attr(el, "y"))
        && covers(attr(el, "width"), vw)
        && covers(attr(el, "height"), vh)
}

fn attr<'a>(el: &'a str, name: &str) -> Option<&'a str> {
    for q in ['"', '\''] {
        let key = format!(" {name}={q}");
        if let Some(i) = el.find(&key) {
            let rest = &el[i + key.len()..];
            return rest.find(q).map(|j| &rest[..j]);
        }
    }
    None
}

fn num(v: Option<&str>) -> Option<f64> {
    v?.trim().trim_end_matches("px").parse().ok()
}

fn wrap_bytes(bytes: &[u8], mime: &str) -> String {
    let sq = squircle_d(128.0, 124.0, TILE_R);
    let rim = squircle_d(128.0, 124.0, TILE_R - 0.75);
    let href = format!("data:{mime};base64,{}", base64(bytes));
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">
  <title>app icon</title>
  <defs>
    <linearGradient id="il-paper" x1="0" y1="22" x2="0" y2="226" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FBF7EF"/>
      <stop offset="1" stop-color="#E9E0CF"/>
    </linearGradient>
    <filter id="il-drop" filterUnits="userSpaceOnUse" x="-20" y="-20" width="296" height="300">
      <feGaussianBlur in="SourceAlpha" stdDeviation="7" result="b1"/>
      <feOffset in="b1" dy="8" result="o1"/>
      <feComponentTransfer in="o1" result="s1"><feFuncA type="linear" slope="0.42"/></feComponentTransfer>
      <feGaussianBlur in="SourceAlpha" stdDeviation="1.6" result="b2"/>
      <feOffset in="b2" dy="2" result="o2"/>
      <feComponentTransfer in="o2" result="s2"><feFuncA type="linear" slope="0.35"/></feComponentTransfer>
      <feMerge><feMergeNode in="s1"/><feMergeNode in="s2"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
    <clipPath id="iliada-tile"><path d="{sq}"/></clipPath>
  </defs>
  <path fill="url(#il-paper)" filter="url(#il-drop)" d="{sq}"/>
  <g clip-path="url(#iliada-tile)">
    <image href="{href}" x="26" y="22" width="204" height="204" preserveAspectRatio="xMidYMid meet"/>
  </g>
  <path fill="none" stroke="#A5671F" stroke-opacity="0.35" stroke-width="1.5" d="{rim}"/>
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

    /// A minimal 8-bit RGBA PNG, `w` by `h`, every pixel `px` except the
    /// corners, which are `corner`.
    pub(crate) fn rgba_png(w: u32, h: u32, px: [u8; 4], corner: [u8; 4]) -> Vec<u8> {
        use std::io::Write;
        let mut raw = Vec::new();
        for y in 0..h {
            raw.push(0);
            for x in 0..w {
                let edge = (x == 0 || x == w - 1) && (y == 0 || y == h - 1);
                raw.extend_from_slice(if edge { &corner } else { &px });
            }
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&raw).unwrap();
        let idat = z.finish().unwrap();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        for (kind, data) in [(&b"IHDR"[..], &ihdr[..]), (b"IDAT", &idat[..]), (b"IEND", &[][..])] {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0, 0, 0, 0]); // CRC is not checked
        }
        out
    }

    #[test]
    fn square_pictures_get_the_tile_and_shaped_icons_do_not() {
        let d = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let p = d.path().join(name);
            fs::write(&p, bytes).unwrap();
            p
        };
        let square = write("square.png", &rgba_png(16, 16, [200, 40, 40, 255], [200, 40, 40, 255]));
        let wrapped = wrap_if_needed(&square).expect("an opaque square PNG gets the tile");
        assert!(wrapped.contains(TILE_MARK), "clip path id is the skip marker");
        assert!(wrapped.contains("data:image/png;base64,"), "original picture is embedded");
        assert!(!wrapped.contains("#14254A"), "the fallback tile is not the old navy");

        let round = write("round.png", &rgba_png(16, 16, [200, 40, 40, 255], [0, 0, 0, 0]));
        assert!(wrap_if_needed(&round).is_none(), "transparent corners mean the icon has its own shape");

        let backdrop = write("backdrop.svg", br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><rect width="48" height="48" fill="#06c"/><circle cx="24" cy="24" r="9"/></svg>"##);
        assert!(wrap_if_needed(&backdrop).is_some(), "an SVG painted edge to edge gets the tile");

        let rounded = write("rounded.svg", br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><rect width="48" height="48" rx="10" fill="#06c"/></svg>"##);
        assert!(wrap_if_needed(&rounded).is_none(), "a rounded backdrop is already a shape");

        let glyph = write(
            "glyph.svg",
            b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'><circle cx='8' cy='8' r='6'/></svg>",
        );
        assert!(wrap_if_needed(&glyph).is_none(), "a freestanding glyph is shown as it is");

        let tiled = write("tiled.svg", format!("<svg><clipPath {TILE_MARK}></clipPath></svg>").as_bytes());
        assert!(wrap_if_needed(&tiled).is_none(), "tiles are not wrapped again");
    }

    #[test]
    fn png_filters_are_undone_before_reading_corners() {
        // One row each of Sub, Up, Average and Paeth on a 2x4 RGBA image;
        // every pixel decodes to opaque.
        let row = |f: u8, px: [u8; 8]| [&[f][..], &px[..]].concat();
        let raw = [
            row(1, [9, 9, 9, 255, 0, 0, 0, 0]),
            row(2, [0, 0, 0, 0, 0, 0, 0, 0]),
            row(3, [5, 5, 5, 128, 0, 0, 0, 0]),
            row(4, [0, 0, 0, 0, 0, 0, 0, 0]),
        ]
        .concat();
        let mut out = vec![0u8; 8];
        let mut prev = vec![0u8; 8];
        let mut alphas = Vec::new();
        for line in raw.chunks(9) {
            unfilter(line[0], &line[1..], &prev, &mut out, 4).unwrap();
            alphas.push((out[3], out[7]));
            prev.copy_from_slice(&out);
        }
        assert_eq!(alphas, [(255, 255), (255, 255), (255, 255), (255, 255)]);
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
