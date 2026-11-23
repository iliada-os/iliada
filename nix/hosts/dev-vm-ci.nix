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
  };
}
