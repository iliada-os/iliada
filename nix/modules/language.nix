# Languages: Iliada comes in English and Greek (Ελληνικά).
#
# Which one a computer uses is `[language] locale` in settings.toml (changed
# in Settings → Time & Language → Language, or with `iliada language el`);
# settings.nix makes it the system locale. Iliada's own apps and COSMIC's
# follow it after the next login.
{ lib, pkgs, ... }:
{
  # Both languages are built in, so switching downloads nothing.
  i18n.extraLocales = [ "en_US.UTF-8/UTF-8" "el_GR.UTF-8/UTF-8" ];

  # Greek letters, polytonic ones too (Homer's "Μῆνιν ἄειδε, θεά"): Noto
  # Sans and Open Sans (COSMIC's interface font) both cover Greek Extended.
  fonts.packages = [ pkgs.noto-fonts pkgs.open-sans ];

  # Keyboard layouts: English (US) and Greek, Super+Space switches between
  # them (COSMIC's shortcut). A system default: a person's own choice in
  # Settings → Keyboard replaces it.
  environment.systemPackages = [
    (lib.hiPrio (pkgs.runCommand "iliada-keyboard-defaults" { } ''
      d=$out/share/cosmic/com.system76.CosmicComp/v1
      mkdir -p $d
      cat > $d/xkb_config <<'RON'
      (
          rules: "",
          model: "",
          layout: "us,gr",
          variant: ",",
          options: None,
          repeat_delay: 600,
          repeat_rate: 25,
      )
      RON
    ''))
  ];
}
