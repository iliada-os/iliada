# The Iliada desktop: COSMIC (Rust, Wayland, Smithay-based compositor).
# Iliada-specific shell pieces (trusted prompts, Applications browser,
# Privacy settings) are layered on top in Phase 2.
{ config, lib, pkgs, ... }:
let
  cfg = config.iliada.desktop;
  desktopApps = pkgs.callPackage ../desktop-apps.nix { };
  # What COSMIC Settings needs of Iliada's source for its pages.
  iliadaSource = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [ ../../Cargo.toml ../../Cargo.lock ../../crates ../../apps ];
  };
  # The default wallpaper: Homer's wine-dark sea at dusk, Troy on its hill,
  # the Achaean ships on the horizon (nix/branding/wallpaper).
  wallpaper = pkgs.runCommand "iliada-wallpaper" { nativeBuildInputs = [ pkgs.librsvg ]; } ''
    mkdir -p $out/share/backgrounds/iliada
    rsvg-convert -w 3840 ${../branding/wallpaper/wine-dark-sea.svg} -o $out/share/backgrounds/iliada/wine-dark-sea.png
  '';
  themeIds = "com.system76.CosmicTheme.Dark com.system76.CosmicTheme.Light com.system76.CosmicTheme.Dark.Builder com.system76.CosmicTheme.Light.Builder";
  # COSMIC's apps under plain names, in English and Greek (left out when a
  # system excludes the app).
  renamedApps = lib.filter (a: !(lib.elem a.pkg config.environment.cosmic.excludePackages)) [
    { pkg = pkgs.cosmic-settings; id = "com.system76.CosmicSettings"; name = "Settings"; el = "Ρυθμίσεις"; }
    { pkg = pkgs.cosmic-files; id = "com.system76.CosmicFiles"; name = "Files"; el = "Αρχεία"; }
    { pkg = pkgs.cosmic-edit; id = "com.system76.CosmicEdit"; name = "Text Editor"; el = "Επεξεργασία κειμένου"; }
    { pkg = pkgs.cosmic-term; id = "com.system76.CosmicTerm"; name = "Terminal"; el = "Τερματικό"; }
    { pkg = pkgs.cosmic-player; id = "com.system76.CosmicPlayer"; name = "Media Player"; el = "Αναπαραγωγή πολυμέσων"; }
    { pkg = pkgs.cosmic-viewer; id = "com.system76.CosmicViewer"; name = "Photos"; el = "Φωτογραφίες"; }
    { pkg = pkgs.cosmic-monitor; id = "com.system76.CosmicMonitor"; name = "System Monitor"; el = "Εποπτεία συστήματος"; }
    { pkg = pkgs.cosmic-reader; id = "com.system76.CosmicReader"; name = "Document Reader"; el = "Ανάγνωση εγγράφων"; }
    { pkg = pkgs.cosmic-screenshot; id = "com.system76.CosmicScreenshot"; name = "Screenshot"; el = "Στιγμιότυπο οθόνης"; }
  ];
in
{
  options.iliada.desktop = {
    enable = lib.mkEnableOption "the Iliada desktop (COSMIC)";
    autoLoginUser = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "iliada";
      description = "Log this user in automatically (kiosk / test VMs).";
    };
    setupAssistant = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Open the Setup Assistant (restore a backup, turn backups on) at each user's first login.";
    };
  };

  config = lib.mkIf cfg.enable {
    # COSMIC Files shows Iliada .app bundles as apps (their own icon, name
    # without ".app"), launches them sandboxed on open, and never treats them
    # as drop targets. See nix/patches/cosmic-files-app-bundles.patch.
    nixpkgs.overlays = [
      (final: prev: {
        cosmic-files = prev.cosmic-files.overrideAttrs (old: {
          patches = (old.patches or [ ]) ++ [ ../patches/cosmic-files-app-bundles.patch ];
        });
        # One Settings app: COSMIC Settings shows Iliada's pages (account,
        # privacy, Homer, updates, backup…) among its own. Iliada's source
        # goes next to it (the pages are iliada_settings::panel), and the
        # lock file covers both.
        cosmic-settings = prev.cosmic-settings.overrideAttrs (old: {
          patches = (old.patches or [ ]) ++ [ ../patches/cosmic-settings-iliada-pages.patch ];
          postPatch = (old.postPatch or "") + ''
            cp -r ${iliadaSource} iliada
            chmod -R u+w iliada
            cp ${../patches/cosmic-settings-iliada.Cargo.lock} Cargo.lock
          '';
          cargoDeps = final.rustPlatform.importCargoLock {
            lockFile = ../patches/cosmic-settings-iliada.Cargo.lock;
            allowBuiltinFetchGit = true;
          };
          # The Iliada account's backups are compressed with zstd.
          buildInputs = (old.buildInputs or [ ]) ++ [ final.zstd ];
        });
        # Every COSMIC app is built with this hook, which points at COSMIC
        # Settings' theme files. Give it the stock COSMIC Settings (same
        # theme files) so the other apps stay byte-identical to nixpkgs and
        # download from its cache, instead of all compiling here.
        libcosmicAppHook = prev.libcosmicAppHook.override {
          targetPackages = prev.targetPackages // { inherit (prev) cosmic-settings; };
        };
        # Document Reader OCR: English, Greek, and a few common languages
        # (every language together is ~1 GB). More can be added later by
        # widening this list. The live ISO leaves the reader out entirely.
        cosmic-reader = prev.cosmic-reader.override {
          tesseract = final.tesseract.override {
            enableLanguages = [
              "eng"
              "osd"
              "ell"
              "fra"
              "deu"
              "spa"
            ];
          };
        };
        # The login screen shows a picture of its own until someone has signed
        # in once (it shows each person's wallpaper after that): the Iliad
        # wallpaper instead of COSMIC's nebula, so a new install starts in
        # the Iliad look.
        cosmic-greeter = prev.cosmic-greeter.overrideAttrs (old: {
          postPatch = (old.postPatch or "") + ''
            # res/background.jpg in 1.11, .png upstream; the image is read by
            # its contents, so a PNG serves under either name.
            ${final.librsvg}/bin/rsvg-convert -w 2560 ${../branding/wallpaper/wine-dark-sea.svg} -o iliada-login.png
            for f in res/background.*; do cp iliada-login.png "$f"; done
            rm iliada-login.png
          '';
        });
      })
    ];
    services.desktopManager.cosmic.enable = true;
    # Apps come only as .app bundles: agora replaces COSMIC Store
    # (which installs Flatpaks and system packages). Iliada's Setup Assistant
    # replaces COSMIC's first-login wizard.
    environment.cosmic.excludePackages = [ pkgs.cosmic-store ]
      ++ lib.optional (pkgs ? cosmic-initial-setup) pkgs.cosmic-initial-setup;
    environment.etc."xdg/autostart/org.iliada.Setup.desktop" = lib.mkIf cfg.setupAssistant {
      source = ../../apps/iliada-settings/res/org.iliada.Setup-autostart.desktop;
    };
    # The search overlay waits, hidden, from login, so Super only has to
    # show it (iliada-search starts it if it is not running).
    environment.etc."xdg/autostart/org.iliada.SearchOverlay.desktop".source =
      ../../apps/iliada-search-overlay/res/org.iliada.SearchOverlay-autostart.desktop;
    # The search service: the home folder's index, in memory, kept current.
    systemd.user.services.iliada-searchd = {
      description = "Iliada search: the index of apps, files and settings";
      wantedBy = [ "default.target" ];
      # Desktop entries are found as in the session (systemd's user
      # manager has no XDG_DATA_DIRS of its own).
      environment.XDG_DATA_DIRS = "%h/.local/share:%h/.nix-profile/share:/etc/profiles/per-user/%u/share:/run/current-system/sw/share:/var/lib/iliada-desktop";
      path = [ pkgs.xdg-utils ];
      serviceConfig = {
        ExecStart = "${config.iliada.package}/bin/iliada-searchd";
        Restart = "on-failure";
        RestartSec = 2;
        # Indexing never competes with what you do for the disk.
        IOSchedulingClass = "idle";
      };
    };
    # Homer's control of other apps: a private socket, a banner, a stop key.
    systemd.user.services.iliada-control = {
      description = "Homer's control of other apps";
      wantedBy = [ "default.target" ];
      # Accessibility, Chrome and the desktop portal all need the session's
      # usual tools and sockets.
      path = [ pkgs.xdg-dbus-proxy pkgs.xdg-utils pkgs.libnotify ];
      serviceConfig = {
        ExecStart = "${config.iliada.package}/bin/iliada-control serve";
        Restart = "on-failure";
        RestartSec = 2;
      };
    };
    environment.systemPackages = [
      # Notifications from scripts and the Setup Assistant's background installs.
      pkgs.libnotify
      # Filtered accessibility bus for sandboxed apps (Homer's app control).
      pkgs.xdg-dbus-proxy
      # agora, Iliada Settings and the Setup Assistant.
      desktopApps
      wallpaper
      # Iliada's desktop defaults (users can still change them): Applications
      # in the Files sidebar; agora in the dock; the Iliad look.
      # hiPrio: they replace COSMIC's own defaults for the same settings.
      (lib.hiPrio (pkgs.runCommand "iliada-cosmic-defaults" {
        nativeBuildInputs = [ pkgs.librsvg ];
      } ''
        # The Iliad look: bronze on navy (dark), deep bronze on parchment
        # (light), computed by COSMIC's own theme code (iliada-theme).
        # Dark mode is the default (Mode/v1/is_dark).
        ${desktopApps}/bin/iliada-theme $out/share
        bg=$out/share/cosmic/com.system76.CosmicBackground/v1
        mkdir -p $bg
        cat > $bg/all <<EOF
        (
            output: "all",
            source: Path("${wallpaper}/share/backgrounds/iliada/wine-dark-sea.png"),
            filter_by_theme: false,
            rotation_frequency: 3600,
            filter_method: Lanczos,
            scaling_mode: Zoom,
            sampling_method: Alphanumeric,
        )
        EOF
        files=$out/share/cosmic/com.system76.CosmicFiles/v1
        dock=$out/share/cosmic/com.system76.CosmicAppList/v1
        mkdir -p $files $dock
        cat > $files/favorites <<'EOF'
        [
            Home,
            Path("/Applications"),
            Documents,
            Downloads,
            Music,
            Pictures,
            Videos,
        ]
        EOF
        apps=$out/share/applications
        mkdir -p $apps
        # COSMIC's apps by plain names ("Files", not "COSMIC Files"), and
        # Iliada's icons for them (nix/branding/cosmic-icons). COSMIC's apps
        # ship their icon as an SVG in every hicolor size folder
        # (16x16/apps/com.system76.CosmicFiles.svg ...), and an SVG there is
        # found before a PNG of the same name, so Iliada's icon goes in under
        # the very same file names (hiPrio wins the clash), plus a PNG for
        # toolkits that want one. One Settings app: COSMIC Settings, which
        # shows Iliada's pages among its own, is "Settings".
        rename() {
          [ -f "$1" ] || return 0
          # The app's own name in English ($2) and Greek ($3); its actions
          # (New Window…) keep theirs.
          awk -v name="$2" -v el="$3" '
            /^\[/ { main = ($0 == "[Desktop Entry]") }
            main && /^Name\[/ { next }
            main && /^Name=/ { print "Name=" name; print "Name[el]=" el; next }
            { print }
          ' "$1" > $apps/$(basename "$1")
        }
        ${lib.concatMapStrings ({ pkg, id, name, el }: ''
          rename ${pkg}/share/applications/${id}.desktop "${name}" "${el}"
        '') renamedApps}
        for icon in ${../branding/cosmic-icons}/*.svg; do
          id=$(basename "$icon" .svg)
          install -Dm644 "$icon" $out/share/icons/hicolor/scalable/apps/$id.svg
          for sz in 16 24 32 48 64 128 256 512; do
            mkdir -p $out/share/icons/hicolor/''${sz}x''${sz}/apps
            install -m644 "$icon" $out/share/icons/hicolor/''${sz}x''${sz}/apps/$id.svg
            rsvg-convert -w $sz -h $sz "$icon" \
              -o $out/share/icons/hicolor/''${sz}x''${sz}/apps/$id.png
          done
        done
        # Tools for the terminal and for Linux experts stay out of the app
        # library (they still run from a terminal).
        for id in htop btop nixos-manual qt5ct qt6ct gvim vim nm-connection-editor; do
          printf '[Desktop Entry]\nType=Application\nName=%s\nExec=true\nNoDisplay=true\n' $id > $apps/$id.desktop
        done
        cat > $dock/favorites <<'EOF'
        [
            "com.system76.CosmicFiles",
            "org.iliada.Agora",
            "org.iliada.Assistant",
            "com.system76.CosmicSettings",
            "com.system76.CosmicEdit",
            "com.system76.CosmicTerm",
        ]
        EOF
        # Iliada's search (Spotlight for Iliada) in place of COSMIC's
        # launcher: Super shows it, Super+O asks Homer, Super+V is the
        # clipboard, Super+Escape stops Homer in an app. Tiling's "toggle orientation" moves from Super+O to
        # Super+Shift+O. Alt+Space still opens Homer's quick panel
        # (Super+Space stays the keyboard-layout switch). The dock's Search
        # button runs the launcher's desktop entry, replaced below.
        keys=$out/share/cosmic/com.system76.CosmicSettings.Shortcuts/v1
        mkdir -p $keys
        cat > $keys/custom <<'EOF'
        {
            (modifiers: [Super]): Spawn("iliada-search"),
            (modifiers: [Super], key: "slash"): Spawn("iliada-search"),
            (modifiers: [Super], key: "o"): Spawn("iliada-search homer"),
            (modifiers: [Super, Shift], key: "o"): ToggleOrientation,
            (modifiers: [Super], key: "v"): Spawn("iliada-search clipboard"),
            (modifiers: [Super], key: "Escape"): Spawn("iliada-control stop"),
            (modifiers: [Alt], key: "space"): Spawn("iliada-ask"),
        }
        EOF
        install -m644 ${../../apps/iliada-search-overlay/res/com.system76.CosmicLauncher.desktop} \
          $apps/com.system76.CosmicLauncher.desktop
        # Frosted glass: windows, the panel and dock, their menus and the
        # launcher are see-through with the desktop blurred behind them
        # (COSMIC Settings → Appearance turns it off). Set for the theme and
        # for the builder COSMIC regenerates the theme from, light and dark.
        for theme in Dark Light Dark.Builder Light.Builder; do
          for v in v1 v2; do
            d=$out/share/cosmic/com.system76.CosmicTheme.$theme/$v
            mkdir -p $d
            for key in frosted_windows frosted_panel frosted_applets frosted_system_interface; do
              echo true > $d/$key
            done
          done
        done
      ''))
    ];
    # Dark mode by default for GTK apps that do not follow COSMIC yet.
    # These files apply only when the person has no settings of their own.
    # Someone who picks Light in Appearance: COSMIC sets
    # org.gnome.desktop.interface color-scheme to prefer-light, which GTK 4,
    # libadwaita and the portal follow.
    environment.etc."xdg/gtk-3.0/settings.ini".text = ''
      [Settings]
      gtk-application-prefer-dark-theme=true
    '';
    environment.etc."xdg/gtk-4.0/settings.ini".text = ''
      [Settings]
      gtk-application-prefer-dark-theme=true
    '';
    programs.dconf.enable = true;
    programs.dconf.profiles.user.databases = [
      {
        settings."org/gnome/desktop/interface" = {
          color-scheme = "prefer-dark";
        };
      }
    ];
    # COSMIC's apps read the built-in fallback theme before the system
    # defaults above, so each person gets Iliada's look (colours and frosted
    # glass) in their own settings at login. A theme the person changed in
    # Appearance is theirs and stays; one still as Iliada left it follows
    # Iliada's (a newer Iliada can refresh its look). Dark mode is copied
    # only when they have no Mode setting yet, so a saved choice stays.
    systemd.user.services.iliada-look = {
      description = "Iliada's look: the Iliad theme and frosted glass";
      wantedBy = [ "default.target" ];
      path = [ pkgs.diffutils ];
      serviceConfig.Type = "oneshot";
      script = ''
        base=''${XDG_CONFIG_HOME:-$HOME/.config}/cosmic
        sys=/run/current-system/sw/share/cosmic
        saved=''${XDG_STATE_HOME:-$HOME/.local/state}/iliada/look
        for id in ${themeIds}; do
          for v in v1 v2; do
            src=$sys/$id/$v
            [ -d "$src" ] || continue
            dst=$base/$id/$v
            was=$saved/$id/$v
            mkdir -p "$dst"
            if [ -z "$(ls -A "$dst")" ] || { [ -d "$was" ] && diff -rq "$dst" "$was" >/dev/null; }; then
              rm -rf "$dst" "$was"
              mkdir -p "$dst" "$was"
              cp -L "$src"/* "$dst"/
              cp -L "$src"/* "$was"/
              chmod -R u+w "$dst" "$was"
            fi
          done
        done
        # Dark mode for a new person. Leave a choice they already made.
        mode=$base/com.system76.CosmicTheme.Mode/v1
        sys_mode=$sys/com.system76.CosmicTheme.Mode/v1
        if [ ! -e "$mode/is_dark" ] && [ -f "$sys_mode/is_dark" ]; then
          mkdir -p "$mode"
          cp -L "$sys_mode/is_dark" "$mode"/
          if [ -f "$sys_mode/auto_switch" ] && [ ! -e "$mode/auto_switch" ]; then
            cp -L "$sys_mode/auto_switch" "$mode"/
          fi
          chmod -R u+w "$mode"
        fi
        # Dock ids that changed with an Iliada update. Same replacements as
        # migrate_dock (iliada-cli): Settings since 0.2.6, agora after that.
        fav=$base/com.system76.CosmicAppList/v1/favorites
        if [ -f "$fav" ]; then
          if grep -q '"org.iliada.Settings"' "$fav"; then
            if grep -q '"com.system76.CosmicSettings"' "$fav"; then
              grep -v '"org.iliada.Settings"' "$fav" > "$fav.new" && mv "$fav.new" "$fav"
            else
              sed -i 's/"org.iliada.Settings"/"com.system76.CosmicSettings"/' "$fav"
            fi
          fi
          if grep -q '"org.iliada.Store"' "$fav"; then
            if grep -q '"org.iliada.Agora"' "$fav"; then
              grep -v '"org.iliada.Store"' "$fav" > "$fav.new" && mv "$fav.new" "$fav"
            else
              sed -i 's/"org.iliada.Store"/"org.iliada.Agora"/' "$fav"
            fi
          fi
        fi
      '';
    };
    services.displayManager.cosmic-greeter.enable = true;
    # Its helper reads each person's theme; for someone who never signed in
    # that is the system's defaults (the Iliad theme), which it finds only
    # through XDG_DATA_DIRS (a system service has none).
    systemd.services.cosmic-greeter-daemon.environment.XDG_DATA_DIRS = "/run/current-system/sw/share";
    services.displayManager.autoLogin = lib.mkIf (cfg.autoLoginUser != null) {
      enable = true;
      user = cfg.autoLoginUser;
    };
    hardware.graphics.enable = true;
    # Desktop apps reach portals; the Iliada broker will back these in Phase 2.
    xdg.portal.enable = true;
  };
}
