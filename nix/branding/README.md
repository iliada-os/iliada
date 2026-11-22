# Iliada branding

The Iliada mark is a left-facing Corinthian helmet (περικεφαλαία) in bronze,
with a red horsehair crest seated on the dome. Every file below is built from
the same hand-authored helmet and crest paths, so they always match.

## Files

| File | What it is |
| --- | --- |
| `iliada.svg` | Main color mark, transparent background, 256×256. Installed as the `iliada` icon by `nix/modules/branding.nix`. |
| `iliada-icon.svg` | App / launcher icon: the mark on a deep navy squircle (superellipse n=5, inset for a soft shadow). |
| `iliada-mono.svg` | Single-color mark using `currentColor`. Legible down to 16 px. |
| `iliada-lockup.svg` | Mark plus "Iliada" wordmark, for light backgrounds. |
| `iliada-lockup-dark.svg` | Same lockup, for dark backgrounds. |
| `apps/iliada-store-gui/res/org.iliada.Agora.svg` | The agora icon: a stoa of the marketplace (pediment, meander, columns, amphorae) on the navy squircle. |
| `apps/iliada-settings/res/org.iliada.Assistant.svg` | Homer (the assistant): his bearded profile with a red fillet on a bronze coin, eye closed (Homer was blind). |
| `apps/iliada-settings/res/org.iliada.Settings.svg` | Settings: a bronze gear with a meander ring and a red boss. |
| `apps/iliada-settings/res/org.iliada.Setup.svg` | The Setup Assistant: a laurel wreath around a red alpha (the beginning). |
| `apps/iliada-installer/res/org.iliada.Installer.svg` | The installer: a Greek ship with a red-striped sail setting out. |
| `wallpaper/wine-dark-sea.svg` | The default wallpaper: the wine-dark sea at dusk, Troy on its hill (left), the Achaean ships (right). Rendered to a 3840 px PNG at build time (nix/modules/desktop.nix). |
| `logo.txt` | fastfetch ASCII art. `$1` is the crest color, `$2` the helmet color. |
| `fastfetch.jsonc` | fastfetch config that uses `logo.txt`. |
| `png/` | Raster exports: icon at 16 to 1024 px, mark and mono mark at 512 px, lockups at 1600 px wide. PNG sizes for the icon theme are built at install (`rsvg-convert`). |
| `glyphs/` | The picture for each app icon. `tools/app_icons.py` sizes each one and adds the drop shadow. |
| `tools/generate.py` | Regenerates all SVGs, PNGs and `logo.txt` (needs `cairosvg` and `pillow`). |
| `tools/app_icons.py` | Freestanding icon layout (size balancing, drop shadow) plus the app and COSMIC icon list. |

The wordmark is drawn as geometric outlines, so no font is needed.

## Palette

| Role | Hex |
| --- | --- |
| Bronze light | `#F6D38E` |
| Bronze mid | `#DDA34E` |
| Bronze shadow | `#A5671F` |
| Bronze sheen | `#FFF1CC` (35% opacity) |
| Crest light | `#EE5442` |
| Crest mid | `#CF3228` |
| Crest shadow | `#9E1F18` |
| Navy glow | `#22396A` |
| Navy | `#14254A` |
| Navy deep | `#0B1630` |
| Ink (wordmark on light) | `#18213A` |
| Paper (wordmark on dark) | `#F4EFE6` |

## Usage

* Use `iliada.svg` wherever the OS logo appears on its own; use `iliada-icon.svg`
  where a launcher or agora expects a filled tile. The named `iliada` icon in
  the icon theme is the tile (About, the greeter, the launcher).
* Every Iliada app icon (agora, Settings, Homer, Setup Assistant, the
  installer, Files, Text Editor, Terminal, Media Player, Photos, Document
  Reader, Screenshot, Search, System Monitor, the app library, Workspaces,
  updates) is a freestanding bronze and red shape with a soft drop
  shadow, no tile behind it, readable from 16 to 512 px on light and dark
  backgrounds. Apps installed as `.app` bundles keep their own icon; only a
  plain square picture gets a soft rounded tile when `iliada-appd` exports
  it.
* Use `iliada-mono.svg` for tiny sizes, the boot splash, panel / tray icons and
  anywhere only one color is allowed. Set the color with CSS `color` (or the
  `color` attribute on the `<svg>`).
* Keep clear space around the mark of at least the crest's thickness on every side.
* Don't recolor the crest and helmet independently, stretch the mark, or add
  outlines or drop shadows to `iliada.svg`.
* After editing the geometry in `tools/generate.py`, run
  `python3 nix/branding/tools/generate.py` and commit the regenerated files.
