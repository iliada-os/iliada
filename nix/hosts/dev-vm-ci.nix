# The GitHub Actions VM: same as the mutable dev system, with RAM/CPU that
# fit a hosted runner (~7 GiB). memorySize here is both QEMU -m and the
# virtiofs memory-backend-memfd; they must stay in agreement (do not pass
# a second -m on the QEMU command line).
{ lib, ... }:
{
  imports = [ ./dev-vm.nix ];
  virtualisation.vmVariant.virtualisation = {
    memorySize = lib.mkForce 4096;
    cores = lib.mkForce 2;
    # Same GPU/display as the mutable VM, but VNC on loopback with no
    # password so CI can grab a picture without extra libraries. The
    # hosted runner is not reachable from the internet.
    qemu.options = lib.mkForce [
      "-vga none"
      "-device virtio-vga,xres=1600,yres=1000"
      "-display none"
      "-vnc 127.0.0.1:0"
    ];
  };
}
