# linux-firmware trimmed to what a live session on a PC actually needs:
# Intel/AMD GPUs, common Wi-Fi and Bluetooth. The full tree is most of a
# gigabyte of phones, servers and NVIDIA. Installed systems keep the full
# redistributable set (workstation.nix).
#
# Files are copied (not hardlinked) so this derivation does not keep a
# reference to the full linux-firmware store path.
{
  lib,
  linux-firmware,
  rsync,
  stdenvNoCC,
}:
let
  dirs = [
    # GPUs (Intel, AMD; older Radeon)
    "amd"
    "amdgpu"
    "i915"
    "radeon"
    "xe"
    # Intel: Wi-Fi bits plus Bluetooth (ibt-*)
    "intel"
    # Wi-Fi / Bluetooth
    "ath10k"
    "ath11k"
    "ath12k"
    "ath6k"
    "ath9k_htc"
    "brcm"
    "cypress"
    "iwlwifi"
    "libertas"
    "mediatek"
    "mrvl"
    "mt7601u"
    "mwlwifi"
    "qca"
    "rsi"
    "rtw88"
    "rtw89"
    "rtl_bt"
    "rtl_nic"
    "rtlwifi"
    "ti-connectivity"
  ];
in
stdenvNoCC.mkDerivation {
  name = "iliada-linux-firmware";
  src = linux-firmware;
  dontUnpack = true;
  dontPatch = true;
  dontConfigure = true;
  dontBuild = true;
  # Firmware blobs are not ELF; stdenv's patchelf fixup would fail.
  dontFixup = true;
  dontPatchELF = true;
  nativeBuildInputs = [ rsync ];
  installPhase = ''
    runHook preInstall
    mkdir -p $out/lib/firmware
    src=$src/lib/firmware
    if [ ! -d "$src" ]; then
      echo "linux-firmware layout changed: $src missing" >&2
      ls -la $src/.. >&2
      exit 1
    fi
    copy() {
      local rel="$1"
      if [ -e "$src/$rel" ]; then
        rsync -a --copy-links --copy-dirlinks "$src/$rel" $out/lib/firmware/
      fi
    }
    ${lib.concatMapStrings (d: "copy ${lib.escapeShellArg d}\n") dirs}
    # Intel Wi-Fi lives at the top of the tree as iwlwifi-*.ucode / .pnvm.
    find "$src" -maxdepth 1 \( -name 'iwlwifi-*' -o -name 'WHENCE' -o -name 'LICEN*' -o -name 'GPL*' \) \
      -exec rsync -a --copy-links {} $out/lib/firmware/ \;
    runHook postInstall
  '';
  # Do not leak a reference to the full tree.
  allowedReferences = [ ];
  meta = {
    description = "linux-firmware for common PC Wi-Fi, Bluetooth and Intel/AMD GPUs";
    license = lib.licenses.unfreeRedistributableFirmware;
  };
}
