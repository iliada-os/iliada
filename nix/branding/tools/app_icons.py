# Iliada app and system icons: freestanding bronze-and-red shapes with a soft
# drop shadow, no tile behind them. Called from generate.py. Glyphs live in
# nix/branding/glyphs/.
from pathlib import Path

EXTRA = {
    "agora": """
    <linearGradient id="col" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#A5671F"/>
      <stop offset="0.35" stop-color="#F6D38E"/>
      <stop offset="0.7" stop-color="#DDA34E"/>
      <stop offset="1" stop-color="#8A5418"/>
    </linearGradient>""",
    "homer": """
    <linearGradient id="hm-face" x1="104" y1="70" x2="152" y2="132" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFF1CC"/>
      <stop offset="0.45" stop-color="#F6D38E"/>
      <stop offset="1" stop-color="#D49A45"/>
    </linearGradient>
    <linearGradient id="hm-hair" x1="96" y1="50" x2="160" y2="150" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#C58A3A"/>
      <stop offset="1" stop-color="#7A4612"/>
    </linearGradient>
    <linearGradient id="hm-neck" x1="0" y1="120" x2="0" y2="156" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#A5671F"/>
      <stop offset="1" stop-color="#DDA34E"/>
    </linearGradient>
    <linearGradient id="hm-drape" x1="70" y1="150" x2="190" y2="200" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#F6D38E"/>
      <stop offset="0.5" stop-color="#DDA34E"/>
      <stop offset="1" stop-color="#A5671F"/>
    </linearGradient>
    <linearGradient id="hm-base" x1="0" y1="196" x2="0" y2="206" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#F6D38E"/>
      <stop offset="1" stop-color="#A5671F"/>
    </linearGradient>""",
    "installer": """
    <linearGradient id="sail" x1="0" y1="64" x2="0" y2="140" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFF8EA"/><stop offset="1" stop-color="#E9DCC2"/>
    </linearGradient>""",
}

# glyph name -> (title, extra-defs key or None)
GLYPHS = {
    "agora": ("agora", "agora"),
    "settings": ("Iliada Settings", None),
    "homer": ("Homer, the Iliada assistant", "homer"),
    "setup": ("Iliada Setup Assistant", None),
    "installer": ("Install Iliada", "installer"),
    "files": ("Files", None),
    "edit": ("Text Editor", None),
    "term": ("Terminal", None),
    "player": ("Media Player", None),
    "photos": ("Photos", None),
    "reader": ("Document Reader", None),
    "screenshot": ("Screenshot", None),
    "search": ("Search", None),
    "monitor": ("System Monitor", None),
    "updates": ("Updates", None),
}

# Where each named icon is installed (repo-relative). Same glyph can land in two files.
TARGETS = [
    ("agora", "apps/iliada-store-gui/res/org.iliada.Agora.svg", "org.iliada.Agora"),
    ("settings", "apps/iliada-settings/res/org.iliada.Settings.svg", "org.iliada.Settings"),
    ("settings", "nix/branding/cosmic-icons/com.system76.CosmicSettings.svg", "com.system76.CosmicSettings"),
    ("homer", "apps/iliada-settings/res/org.iliada.Assistant.svg", "org.iliada.Assistant"),
    ("setup", "apps/iliada-settings/res/org.iliada.Setup.svg", "org.iliada.Setup"),
    ("installer", "apps/iliada-installer/res/org.iliada.Installer.svg", "org.iliada.Installer"),
    ("files", "nix/branding/cosmic-icons/com.system76.CosmicFiles.svg", "com.system76.CosmicFiles"),
    ("edit", "nix/branding/cosmic-icons/com.system76.CosmicEdit.svg", "com.system76.CosmicEdit"),
    ("term", "nix/branding/cosmic-icons/com.system76.CosmicTerm.svg", "com.system76.CosmicTerm"),
    ("player", "nix/branding/cosmic-icons/com.system76.CosmicPlayer.svg", "com.system76.CosmicPlayer"),
    ("photos", "nix/branding/cosmic-icons/com.system76.CosmicViewer.svg", "com.system76.CosmicViewer"),
    ("reader", "nix/branding/cosmic-icons/com.system76.CosmicReader.svg", "com.system76.CosmicReader"),
    ("screenshot", "nix/branding/cosmic-icons/com.system76.CosmicScreenshot.svg", "com.system76.CosmicScreenshot"),
    ("search", "nix/branding/cosmic-icons/com.system76.CosmicLauncher.svg", "com.system76.CosmicLauncher"),
    ("monitor", "nix/branding/cosmic-icons/com.system76.CosmicMonitor.svg", "com.system76.CosmicMonitor"),
    ("apps", "nix/branding/cosmic-icons/com.system76.CosmicAppLibrary.svg", "com.system76.CosmicAppLibrary"),
    ("workspaces", "nix/branding/cosmic-icons/com.system76.CosmicWorkspaces.svg", "com.system76.CosmicWorkspaces"),
    ("updates", "nix/branding/cosmic-icons/software-update-available.svg", "software-update-available"),
]


def apps_glyph(squircle):
    tiles = []
    for row in range(3):
        for col in range(3):
            d = squircle(78 + col * 50, 78 + row * 50, 18, n=5, seg=24)
            fill = "url(#il-crest)" if col == 0 else "url(#il-bronze)"
            tiles.append(f'<path fill="{fill}" d="{d}"/>')
    return "\n  ".join(tiles)


def workspaces_glyph(squircle):
    ds = [squircle(cx, cy, 32, n=5, seg=24) for cx, cy in [(78, 90), (178, 90), (78, 166), (178, 166)]]
    parts = [f'<path fill="url(#il-bronze)" d="{ds[0]}"/>']
    # mid blue, not navy, so the empty workspaces read on light and dark
    for d in ds[1:]:
        parts.append(f'<path fill="#3B5C94" stroke="#DDA34E" stroke-width="4" d="{d}"/>')
    parts.append('<rect x="62" y="78" width="32" height="8" rx="3" fill="#9E1F18"/>')
    return "\n  ".join(parts)


# ---- freestanding layout ----
# A few glyphs only read because the old navy tile sat behind them. They get
# their own body here, drawn under the glyph.
UNDERLAY = {
    # temple: a dark cella wall behind the columns
    "agora": '<rect x="52" y="110" width="152" height="80" fill="#14254A"/>',
    # screenshot: a screen inside the corner brackets
    "screenshot": '<rect x="72" y="80" width="112" height="96" rx="12" fill="#14254A"/>',
}

# Every glyph is scaled so its longest side is TARGET units of the 256 box,
# times this factor: wide rectangles and tall shapes look heavier than round
# ones at the same size.
OPTICAL = {
    "settings": 0.95, "updates": 0.95, "search": 0.95,
    "term": 0.92, "photos": 0.92, "monitor": 0.92, "files": 0.93,
    "agora": 0.95, "edit": 0.96, "installer": 0.97, "screenshot": 0.94,
    "player": 0.88, "apps": 0.86, "workspaces": 0.88,
}
TARGET = 204
CENTER = (128, 124)  # a little high, the shadow sits underneath

DROP_DEFS = """
    <filter id="il-drop" filterUnits="userSpaceOnUse" x="-20" y="-20" width="296" height="300">
      <feGaussianBlur in="SourceAlpha" stdDeviation="7" result="b1"/>
      <feOffset in="b1" dy="8" result="o1"/>
      <feComponentTransfer in="o1" result="s1"><feFuncA type="linear" slope="0.42"/></feComponentTransfer>
      <feGaussianBlur in="SourceAlpha" stdDeviation="1.6" result="b2"/>
      <feOffset in="b2" dy="2" result="o2"/>
      <feComponentTransfer in="o2" result="s2"><feFuncA type="linear" slope="0.35"/></feComponentTransfer>
      <feMerge><feMergeNode in="s1"/><feMergeNode in="s2"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>"""


def ink_box(svg_text):
    """Bounding box of everything drawn, in 256-unit icon space."""
    import io
    import cairosvg
    from PIL import Image

    png = cairosvg.svg2png(bytestring=svg_text.encode(), output_width=512, output_height=512)
    alpha = Image.open(io.BytesIO(png)).getchannel("A").point(lambda a: 255 if a > 24 else 0)
    x0, y0, x1, y1 = alpha.getbbox()
    return x0 / 2, y0 / 2, x1 / 2, y1 / 2


def freestanding(name, title, body, extra_defs, svg, icon_defs):
    """The glyph alone, sized to the icon box, with a soft drop shadow."""
    if name in UNDERLAY:
        body = f"  {UNDERLAY[name]}\n  {body}"
    defs = icon_defs + extra_defs
    x0, y0, x1, y1 = ink_box(svg((0, 0, 256, 256), body, defs))
    s = TARGET * OPTICAL.get(name, 1.0) / max(x1 - x0, y1 - y0)
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    tf = f"translate({CENTER[0]} {CENTER[1]}) scale({s:.4f}) translate({-cx:.2f} {-cy:.2f})"
    out = f'  <g filter="url(#il-drop)">\n    <g transform="{tf}">\n{body}\n    </g>\n  </g>'
    return svg((0, 0, 256, 256), out, defs + DROP_DEFS, title)


def glyph(name, glyphs_dir, squircle):
    """(body, title, extra_defs) for one named glyph."""
    if name == "apps":
        return apps_glyph(squircle), "Applications", ""
    if name == "workspaces":
        return workspaces_glyph(squircle), "Workspaces", ""
    title, extra = GLYPHS[name]
    body = (Path(glyphs_dir) / f"{name}.svg.part").read_text()
    return body, title, EXTRA.get(extra or "", "")


def icon(name, branding, squircle, svg, icon_defs):
    body, title, extra_defs = glyph(name, Path(branding) / "glyphs", squircle)
    return freestanding(name, title, body, extra_defs, svg, icon_defs)


def generate(branding, repo, squircle, svg, icon_defs):
    """Write every app / system SVG. Returns [(svg_path, icon_id), ...]."""
    written = []
    cache = {}
    for name, rel, icon_id in TARGETS:
        if name not in cache:
            cache[name] = icon(name, branding, squircle, svg, icon_defs)
        out = Path(repo) / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(cache[name])
        written.append((str(out), icon_id))
    return written
