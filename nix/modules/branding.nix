# Branding: "Iliada" in /etc/os-release, the boot menu and the greeter, and
# its logo, a Greek helmet (περικεφαλαία) with a horsehair crest: the
# `iliada` icon, and the picture `fastfetch` / `neofetch` show.
# The distribution ID stays "nixos" so NixOS tooling keeps working.
{ config, pkgs, ... }:
let
  branding = pkgs.runCommand "iliada-branding" {
    nativeBuildInputs = [ pkgs.librsvg ];
  } ''
    # The named `iliada` icon is the navy tile (greeter, About, launcher).
    # The unpadded mark stays in pixmaps for places that want the helmet alone.
    install -Dm644 ${../branding/iliada-icon.svg} $out/share/icons/hicolor/scalable/apps/iliada.svg
    install -Dm644 ${../branding/iliada.svg} $out/share/pixmaps/iliada.svg
    for sz in 16 24 32 48 64 128 256 512; do
      mkdir -p $out/share/icons/hicolor/''${sz}x''${sz}/apps
      rsvg-convert -w $sz -h $sz ${../branding/iliada-icon.svg} \
        -o $out/share/icons/hicolor/''${sz}x''${sz}/apps/iliada.png
    done
    install -Dm644 ${../branding/logo.txt} $out/share/iliada/logo.txt
    substitute ${../branding/fastfetch.jsonc} $out/share/iliada/fastfetch.jsonc \
      --replace-fail @logo@ $out/share/iliada/logo.txt
  '';
  # neofetch is no longer maintained; fastfetch is its successor.
  neofetch = pkgs.writeShellScriptBin "neofetch" ''exec ${pkgs.fastfetch}/bin/fastfetch "$@"'';
in
{
  system.nixos.distroName = "Iliada";
  # What people see (fastfetch, Settings, the report): Iliada's own version.
  # VERSION_ID stays the NixOS release, which NixOS tooling reads.
  system.nixos.extraOSReleaseArgs = {
    LOGO = "iliada";
    VERSION = config.iliada.version;
    PRETTY_NAME = "Iliada ${config.iliada.version}";
    HOME_URL = "https://iliada-os.com";
    BUG_REPORT_URL = "https://github.com/iliada-os/iliada/issues";
  };
  environment.systemPackages = [ branding pkgs.fastfetch neofetch ];
  environment.etc."xdg/fastfetch/config.jsonc".source = "${branding}/share/iliada/fastfetch.jsonc";
}
