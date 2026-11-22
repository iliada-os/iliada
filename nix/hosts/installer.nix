# The Iliada live/installer ISO: a COSMIC live session plus `iliada-install`.
#   nix build .#installer-iso
{ config, lib, pkgs, modulesPath, iliada, ... }:
let
  desktopApps = pkgs.callPackage ../desktop-apps.nix { };
  iliada-install = pkgs.writeShellApplication {
    name = "iliada-install";
    runtimeInputs = with pkgs; [
      coreutils
      util-linux
      gptfdisk
      dosfstools
      e2fsprogs
      gnused
      gnugrep
      nixos-install-tools
      config.nix.package
      git
      config.iliada.package
    ];
    text = builtins.readFile ../installer/iliada-install.sh;
  };
in
{
  imports = [ "${modulesPath}/installer/cd-dvd/installation-cd-graphical-base.nix" ];

  networking.hostName = "iliada-installer";
  image.baseName = lib.mkForce "iliada-${config.iliada.version}-${pkgs.stdenv.hostPlatform.system}";
  isoImage.volumeID = lib.mkForce "ILIADA_${lib.replaceStrings [ "." ] [ "_" ] config.iliada.version}";
  iliada = {
    desktop = {
      enable = true;
      autoLoginUser = "nixos";
      setupAssistant = false;
    };
    applications.ingest.enable = false;
  };
  # A smaller download. The installed system keeps the document reader,
  # a few OCR languages, the scheduler, mbrola voices, and full firmware.
  environment.cosmic.excludePackages = [ pkgs.cosmic-reader ];
  services.system76-scheduler.enable = lib.mkForce false;
  # The stock graphical installer image ships Firefox for the NixOS manual.
  # Iliada's installer does not need a browser, and Firefox pulls espeak's
  # mbrola voices (~0.6 GB) along with it.
  environment.defaultPackages = lib.mkForce (with pkgs; [
    rsync
    gparted
    vim
    nano
  ]);
  # Screen reader still works (espeak-ng); the extra mbrola voice bank does
  # not. Overlay is ISO-only — installed systems are unchanged.
  nixpkgs.overlays = [
    (final: prev: {
      espeak-ng = prev.espeak-ng.override { mbrolaSupport = false; };
    })
  ];
  # Common PC hardware (Wi-Fi, Intel/AMD GPUs, laptop sound). The full
  # linux-firmware tree is phones, servers and NVIDIA. Intel/AMD microcode
  # stays on (workstation.nix); only the firmware blobs are trimmed.
  hardware.firmware = lib.mkForce (
    with pkgs;
    [
      (callPackage ../firmware-common.nix { })
      sof-firmware
      alsa-firmware
      rtl8761b-firmware
      wireless-regdb
    ]
  );
  # sudo-rs here too (the stock installer profile enables C sudo); the live
  # user needs no password, as on the stock NixOS installer.
  security.sudo.enable = lib.mkForce false;
  security.sudo-rs.wheelNeedsPassword = false;
  # The ISO has its own boot loader.
  boot.loader.systemd-boot.enable = lib.mkForce false;
  boot.loader.efi.canTouchEfiVariables = lib.mkForce false;
  # Also log in on the serial console (used by the automated install test).
  boot.kernelParams = [ "console=tty0" "console=ttyS0,115200" ];

  # The Iliada source the installed system is built from.
  environment.etc."iliada/source".source = iliada;
  environment.systemPackages = [
    iliada-install
    # "Install Iliada" in the launcher and the dock.
    (pkgs.runCommand "iliada-installer-entry" { } ''
      install -Dm644 ${desktopApps}/share/iliada/installer/org.iliada.Installer.desktop \
        $out/share/applications/org.iliada.Installer.desktop
    '')
  ];
  # The graphical installer opens when the live session starts.
  environment.etc."xdg/autostart/org.iliada.Installer.desktop".source =
    "${desktopApps}/share/iliada/installer/org.iliada.Installer-autostart.desktop";

  services.getty.helpLine = lib.mkForce ''

    Welcome to the Iliada installer. Use "Install Iliada" on the desktop, or:
    sudo iliada-install --disk /dev/<disk> --user <name>     (disks: lsblk)
  '';
}
