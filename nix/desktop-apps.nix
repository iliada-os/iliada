# Iliada's desktop apps: agora, Settings, the Setup Assistant, the
# installer and the search overlay. One
# cargo workspace (apps/), built separately from the core tools because it
# depends on libcosmic (git dependencies, GPU rendering stack).
{ lib, rustPlatform, libcosmicAppHook, pkg-config, zstd, librsvg }:

rustPlatform.buildRustPackage {
  pname = "iliada-desktop-apps";
  version = lib.trim (builtins.readFile ../VERSION);

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../apps
    ];
  };
  cargoRoot = "apps";
  buildAndTestSubdir = "apps";
  cargoLock = {
    lockFile = ../apps/Cargo.lock;
    allowBuiltinFetchGit = true;
  };

  # libcosmic: a text field's right-click menu (Cut, Copy, Paste) opened off
  # by the page's scroll offset, often at the bottom of the screen. The patch
  # anchors it where the pointer is in the window.
  postPatch = ''
    lc=$(dirname "$(dirname "$(dirname "$(echo "$cargoDepsCopy"/*/src/widget/text_context_menu.rs)")")")
    patch -d "$lc" -p1 < ${./patches/libcosmic-context-menu-position.patch}
  '';

  nativeBuildInputs = [ libcosmicAppHook pkg-config librsvg ];
  # zstd: Iliada account backups are compressed before they are encrypted.
  buildInputs = [ zstd ];
  doCheck = false;

  postInstall = ''
    # Alt+Space: the assistant's quick panel.
    cat > $out/bin/iliada-ask <<EOS
    #!/bin/sh
    exec $out/bin/iliada-assistant-app --panel "\$@"
    EOS
    chmod +x $out/bin/iliada-ask
    install -Dm644 -t $out/share/applications \
      apps/iliada-store-gui/res/org.iliada.Agora.desktop \
      apps/iliada-settings/res/org.iliada.Settings.desktop \
      apps/iliada-settings/res/org.iliada.Setup.desktop \
      apps/iliada-settings/res/org.iliada.Assistant.desktop
    # Their icons (nix/branding/README.md): agora's stoa, Homer's coin,
    # the Settings gear, the Setup Assistant's wreath, the installer's ship.
    install -Dm644 -t $out/share/icons/hicolor/scalable/apps apps/*/res/org.iliada.*.svg
    # Docks from before the rename still look up the old icon name.
    ln -s org.iliada.Agora.svg $out/share/icons/hicolor/scalable/apps/org.iliada.Store.svg
    for svg in $out/share/icons/hicolor/scalable/apps/org.iliada.*.svg; do
      # Store.svg is the alias above; converting it would write Store.png
      # and the symlink below would then fail with "File exists".
      [ -L "$svg" ] && continue
      id=$(basename "$svg" .svg)
      for sz in 16 24 32 48 64 128 256 512; do
        mkdir -p $out/share/icons/hicolor/''${sz}x''${sz}/apps
        rsvg-convert -w $sz -h $sz "$svg" \
          -o $out/share/icons/hicolor/''${sz}x''${sz}/apps/$id.png
      done
    done
    for sz in 16 24 32 48 64 128 256 512; do
      ln -s org.iliada.Agora.png $out/share/icons/hicolor/''${sz}x''${sz}/apps/org.iliada.Store.png
    done
    # The installer's entries are only linked in on the live system.
    install -Dm644 -t $out/share/iliada/installer apps/iliada-installer/res/*.desktop
  '';

  meta = {
    description = "Iliada's desktop apps: agora, Settings, Setup Assistant";
    license = with lib.licenses; [ mit asl20 ];
    platforms = lib.platforms.linux;
    mainProgram = "iliada-store-gui";
  };
}
