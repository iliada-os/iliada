# The boot menu: GRUB in Iliada's colours. It lists Iliada, its earlier
# versions, and every other system on the computer that os-prober finds
# (Windows, other Linux distributions), even on another EFI system partition.
#
# When another system is found the menu shows at every start; when Iliada is
# alone it stays hidden and Iliada starts at once (press Esc during the first
# second to open it). The list of other systems is refreshed on every update.
{ config, lib, pkgs, ... }:
let
  cfg = config.iliada.bootMenu;
  grub = config.boot.loader.grub;
  distro = config.system.nixos.distroName;
  efiArch = config.nixpkgs.hostPlatform.efiArch;

  # The menu's words, in the system's language (language.nix).
  text = if lib.hasPrefix "el_" config.i18n.defaultLocale then {
    earlier = "Προηγούμενες εκδόσεις της Ιλιάδας";
    from = date: n: "${distro} της ${date} (#${n})";
    hint = "Επιλογή με τα βελάκια  ·  Enter για εκκίνηση";
    timeout = "Εκκίνηση σε %d δευτερόλεπτα";
  } else {
    earlier = "Earlier versions of ${distro}";
    from = date: n: "${distro} from ${date} (#${n})";
    hint = "Arrow keys to choose  ·  Enter to start";
    timeout = "Starting in %d seconds";
  };

  theme = pkgs.runCommand "iliada-boot-theme" {
    nativeBuildInputs = [ pkgs.librsvg pkgs.imagemagick pkgs.buildPackages.grub2 ];
  } ''
    mkdir -p $out/icons
    substitute ${../branding/boot/theme.txt} $out/theme.txt \
      --replace-fail @hint@ ${lib.escapeShellArg text.hint} \
      --replace-fail @timeout@ ${lib.escapeShellArg text.timeout}
    rsvg-convert -w 1920 -h 1080 ${../branding/boot/background.svg} -o $out/background.png
    rsvg-convert -w 370 -h 128 ${../branding/iliada-lockup-dark.svg} -o $out/logo.png

    # Icons by menu entry class: Iliada (and its earlier versions) wear the
    # helmet; the submenu of earlier versions a clock; anything os-prober
    # found (classes windows, gnu-linux, …, always also "os") a screen.
    rsvg-convert -w 32 -h 32 ${../branding/iliada.svg} -o $out/icons/nixos.png
    rsvg-convert -w 32 -h 32 ${../branding/boot/history.svg} -o $out/icons/submenu.png
    rsvg-convert -w 32 -h 32 ${../branding/boot/os.svg} -o $out/icons/os.png

    # The highlight, cut into the nine pieces GRUB stretches, plus the same
    # nine pieces, invisible, for the other entries: GRUB pads each entry by
    # its box's edges, so without them the list shifts as the highlight moves.
    # (PNG32: GRUB reads only 8-bit RGB(A), not palette images.)
    rsvg-convert -w 48 -h 48 ${../branding/boot/select.svg} -o select.png
    magick -size 48x48 xc:none item.png
    cut() {
      magick select.png -crop "$2" +repage PNG32:$out/select_$1.png
      magick item.png -crop "$2" +repage PNG32:$out/item_$1.png
    }
    cut nw 12x12+0+0;  cut n 24x12+12+0;  cut ne 12x12+36+0
    cut w  12x24+0+12; cut c 24x24+12+12; cut e  12x24+36+12
    cut sw 12x12+0+36; cut s 24x12+12+36; cut se 12x12+36+36

    # Open Sans (the desktop's font), Latin and Greek, under the names
    # theme.txt asks for: grub-mkfont adds the style (Regular, for both:
    # Open Sans 1.11 calls Semibold a family) and the size.
    fonts=${pkgs.open-sans}/share/fonts/truetype
    range=0x20-0x7E,0xA0-0x17F,0x370-0x3FF,0x1F00-0x1FFF,0x2010-0x2027
    mkfont() { grub-mkfont -r $range -s "$3" -n "$4" -o "$out/$1-$3.pf2" "$fonts/OpenSans-$2.ttf"; }
    mkfont regular Regular 16 Iliada
    mkfont regular Regular 24 Iliada
    mkfont semibold Semibold 24 "Iliada Semibold"
  '';

  # Runs after GRUB has written its menu (on every update): friendlier names,
  # the menu shown when another system was found, and a UEFI fallback copy.
  finish = ''
    # Iliada: finish the boot menu.
    for boot in ${lib.escapeShellArgs (map (b: b.path) grub.mirroredBoots)}; do
      conf="$boot/grub/grub.cfg"
      [ -f "$conf" ] || continue
      ${pkgs.gnused}/bin/sed \
        -e 's|^submenu "${distro} - All configurations"|submenu "${text.earlier}"|' \
        -e 's|^\([[:space:]]*\)menuentry "${distro} - Configuration \([0-9]*\) (\([0-9-]*\) - [^)]*)"|\1menuentry "${text.from "\\3" "\\2"}"|' \
        -e "/osprober-/ s# (on /dev/[^)]*)\(['\"]\)#\1#" \
        "$conf" > "$conf.iliada"
      ${lib.optionalString (cfg.show == "auto") ''
        if ${pkgs.gnugrep}/bin/grep -q "osprober-" "$conf.iliada"; then
          cat >> "$conf.iliada" <<'EOF'

        # Added by Iliada: another system is on this computer, so the menu shows.
        if [ -z "''${boot_once}" ]; then
          set timeout_style=menu
          set timeout=${toString cfg.timeout}
        fi
        EOF
        fi
      ''}
      ${pkgs.coreutils}/bin/mv "$conf.iliada" "$conf"
    done

    ${lib.concatMapStrings (b: let
      # Where GRUB installs itself (nixos/modules/system/boot/loader/grub).
      esp = if b.efiSysMountPoint == null then b.path else b.efiSysMountPoint;
      id = if b.efiBootloaderId == null then distro + lib.replaceStrings [ "/" ] [ "-" ] esp else b.efiBootloaderId;
    in ''
      b=${lib.escapeShellArg esp}
      # The firmware's fallback path, for firmware that forgets (or never
      # keeps) boot entries. Left alone when Windows shares this partition.
      src="$b/EFI/${id}/grub${efiArch}.efi"
      if [ -f "$src" ] && [ ! -d "$b/EFI/Microsoft" ]; then
        ${pkgs.coreutils}/bin/install -D -m 0644 "$src" "$b/EFI/BOOT/BOOT${lib.toUpper efiArch}.EFI"
      fi
      # Computers installed before Iliada used GRUB booted with systemd-boot:
      # its files (and its copies of every kernel) only take space now.
      if [ -f "$src" ] && [ -d "$b/loader/entries" ]; then
        ${pkgs.coreutils}/bin/rm -rf "$b/loader" "$b/EFI/systemd" "$b/EFI/nixos"
      fi
    '') grub.mirroredBoots}
  '';
in
{
  options.iliada.bootMenu = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = config.iliada.workstation.enable;
      defaultText = lib.literalExpression "config.iliada.workstation.enable";
      description = ''
        Boot with GRUB in Iliada's look, listing other systems on the
        computer. Has no effect where another boot loader is chosen (the
        virtual-machine images use systemd-boot).
      '';
    };
    show = lib.mkOption {
      type = lib.types.enum [ "auto" "always" "hidden" ];
      default = "auto";
      description = ''
        When the menu shows: "auto" when another system is on the computer,
        "always", or "hidden" (open it with Esc during the first second).
      '';
    };
    timeout = lib.mkOption {
      type = lib.types.ints.unsigned;
      default = 5;
      description = "Seconds the menu waits before starting the first entry.";
    };
    theme = lib.mkOption {
      type = lib.types.package;
      default = theme;
      defaultText = lib.literalMD "Iliada's theme (nix/branding/boot)";
      description = "The GRUB theme (also used by the installer's own boot menu).";
    };
  };

  config = lib.mkIf (cfg.enable && grub.enable) {
    boot.loader.grub = {
      efiSupport = lib.mkDefault true;
      device = lib.mkDefault "nodev";
      useOSProber = lib.mkDefault true;
      theme = lib.mkDefault cfg.theme;
      # (The theme draws the background; NixOS sets its own picture by default.)
      splashImage = lib.mkOverride 900 null;
      timeoutStyle = lib.mkDefault (if cfg.show == "always" then "menu" else "hidden");
      gfxmodeEfi = lib.mkDefault "1920x1080,auto";
      # The menu lists ten earlier versions; it also keeps the kernels GRUB
      # copies to the EFI system partition from filling it.
      configurationLimit = lib.mkDefault 10;
      extraInstallCommands = finish;
    };
    boot.loader.timeout = lib.mkDefault (if cfg.show == "always" then cfg.timeout else 1);
  };
}
