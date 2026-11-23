# The GitHub Actions VM: same as the mutable dev system, with RAM/CPU that
# fit a hosted runner (~7 GiB). memorySize here is both QEMU -m and the
# virtiofs memory-backend-memfd; they must stay in agreement (do not pass
# a second -m on the QEMU command line).
#
# Do not mkForce qemu.options: NixOS puts -kernel/-initrd, virtiofs and
# the memfd there. Replacing the list left SeaBIOS with an empty disk
# ("No bootable device"). Screenshots go through QMP; VNC keeps the
# mutable VM's password (iliada).
{ lib, ... }:
{
  imports = [ ./dev-vm.nix ];
  virtualisation.vmVariant.virtualisation = {
    memorySize = lib.mkForce 4096;
    cores = lib.mkForce 2;
  };
}
