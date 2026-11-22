# Iliada app and system icons: one navy squircle tile, bronze-and-red glyphs.
# Called from generate.py. Glyphs live in nix/branding/glyphs/.
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
    cells = []
    for row in range(3):
        for col in range(3):
            cx = 78 + col * 50
            cy = 78 + row * 50
            d = squircle(cx, cy, 18, n=5, seg=24)
            fill = "url(#il-crest)" if col == 0 else "url(#il-bronze)"
            cells.append((d, fill, cx, cy))
    shadow = "\n    ".join(f'<path d="{d}"/>' for d, _, _, _ in cells)
    tiles = "\n  ".join(f'<path fill="{fill}" d="{d}"/>' for d, fill, _, _ in cells)
    return (
        f'<g opacity="0.3" fill="#040A1A" transform="translate(0 4)">\n    {shadow}\n  </g>\n  {tiles}'
    )


def workspaces_glyph(squircle):
    cells = [(78, 90), (178, 90), (78, 166), (178, 166)]
    parts = ['<g fill="#040A1A" opacity="0.3" transform="translate(0 4)">']
    ds = []
    for cx, cy in cells:
        d = squircle(cx, cy, 32, n=5, seg=24)
        ds.append(d)
        parts.append(f'<path d="{d}"/>')
    parts.append("</g>")
    parts.append(f'<path fill="url(#il-bronze)" d="{ds[0]}"/>')
    for d in ds[1:]:
        parts.append(f'<path fill="#22396A" stroke="#DDA34E" stroke-width="3" d="{d}"/>')
    parts.append('<rect x="62" y="78" width="32" height="8" rx="3" fill="#9E1F18"/>')
    return "\n  ".join(parts)


def generate(branding, repo, squircle, svg, tile_icon):
    """Write every app / system SVG. Returns [(svg_path, icon_id), ...]."""
    glyphs_dir = Path(branding) / "glyphs"
    written = []

    def glyph(name):
        if name == "apps":
            return apps_glyph(squircle), "Applications", ""
        if name == "workspaces":
            return workspaces_glyph(squircle), "Workspaces", ""
        title, extra = GLYPHS[name]
        body = (glyphs_dir / f"{name}.svg.part").read_text()
        extra_defs = EXTRA.get(extra or "", "")
        return body, title, extra_defs

    for name, rel, icon_id in TARGETS:
        body, title, extra_defs = glyph(name)
        out = Path(repo) / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(tile_icon(title, body, extra_defs))
        written.append((str(out), icon_id))
    return written
