#!/usr/bin/env python3
"""End-to-end installer test.

1. Boot the Iliada ISO in a UEFI KVM machine with a blank disk.
2. On the serial console (the ISO auto-logs in there), run iliada-install.
3. Power off, boot the installed disk, and capture screenshots over VNC:
   one every two seconds while it starts (the boot menu), and one at the end.

With --next-to-windows, a stand-in for Windows goes on the disk first (an EFI
system partition with a Windows Boot Manager, which is really the live
system's kernel) and Iliada is installed next to it: the boot menu should
show at start-up and list "Windows Boot Manager"; choosing it starts that
kernel, which stops with a kernel panic (it has no root file system).

    python3 tests/installer/run.py --iso result-iso/iso/*.iso \
        --qemu $(which qemu-system-x86_64) --ovmf <OVMF.fd> --workdir /tmp/iliada-install-test

Only the Python standard library is used. Exit status is non-zero on failure.
"""

import argparse
import base64
import os
import re
import socket
import subprocess
import sys
import time


class Serial:
    def __init__(self, path: str, log):
        self.log = log
        self.buf = ""
        deadline = time.time() + 60
        while True:
            try:
                self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.s.connect(path)
                break
            except OSError:
                if time.time() > deadline:
                    raise
                time.sleep(0.5)
        self.s.settimeout(1.0)

    def read_until(self, pattern: str, timeout: float) -> str:
        rx = re.compile(pattern)
        deadline = time.time() + timeout
        while time.time() < deadline:
            m = rx.search(self.buf)
            if m:
                out, self.buf = self.buf[: m.end()], self.buf[m.end():]
                return out
            try:
                chunk = self.s.recv(65536).decode("utf-8", "replace")
            except socket.timeout:
                continue
            if not chunk:
                raise EOFError("serial console closed")
            self.log.write(chunk)
            self.log.flush()
            self.buf += chunk
        raise TimeoutError(f"timed out after {timeout}s waiting for {pattern!r}")

    def send(self, text: str):
        self.s.sendall(text.encode())


def run_as_root(con, script: str, tag: str, timeout: float) -> int:
    """Run a bash script as root on the live system; return its exit status.
    It travels base64-encoded, so neither fish nor quoting gets in the way."""
    b64 = base64.b64encode(script.encode()).decode()
    con.send(f"bash -c 'echo {b64} | base64 -d | sudo bash; echo {tag}-EXIT=$?'\n")
    out = con.read_until(tag + r"-EXIT=\d+", timeout)
    con.read_until(r"(> |[$#] )", 60)
    return int(re.search(tag + r"-EXIT=(\d+)", out).group(1))


STAND_IN_WINDOWS = """set -e
sgdisk -n1:1M:+260M -t1:EF00 -c1:"EFI system partition" /dev/vda
udevadm settle
mkfs.fat -F 32 /dev/vda1
mkdir -p /tmp/win
mount /dev/vda1 /tmp/win
mkdir -p /tmp/win/EFI/Microsoft/Boot
cp "$(readlink -f /run/current-system/kernel)" /tmp/win/EFI/Microsoft/Boot/bootmgfw.efi
echo stand-in > /tmp/win/EFI/Microsoft/Boot/BCD
umount /tmp/win
"""


def qemu(args, disk, sock, cdrom=None):
    cmd = [
        args.qemu, "-enable-kvm", "-machine", "q35", "-cpu", "host", "-m", "4096", "-smp", "4",
        "-bios", args.ovmf,
        "-drive", f"file={disk},if=virtio,format=qcow2",
        "-nic", "user,model=virtio-net-pci",
        "-serial", f"unix:{sock},server=on,wait=off",
        "-vga", "none", "-device", "virtio-vga,xres=1600,yres=1000",
        "-display", "none", "-vnc", f"127.0.0.1:{args.vnc}",
    ]
    if cdrom:
        cmd += ["-cdrom", cdrom, "-boot", "d"]
    return subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--iso", required=True)
    ap.add_argument("--qemu", required=True)
    ap.add_argument("--ovmf", required=True)
    ap.add_argument("--workdir", required=True)
    ap.add_argument("--vnc", type=int, default=1, help="VNC display number (port 5900+N)")
    ap.add_argument("--vncdo", help="vncdo binary, to capture a screenshot of the installed system")
    ap.add_argument("--boot-only", action="store_true", help="skip installation; boot the existing disk")
    ap.add_argument("--next-to-windows", action="store_true",
                    help="put a stand-in for Windows on the disk first, and install next to it")
    ap.add_argument("--substituter", help="an extra binary cache for the installation, e.g. "
                    "http://10.0.2.2:5000 (the host, seen from the VM); its paths are not signature-checked")
    args = ap.parse_args()

    os.makedirs(args.workdir, exist_ok=True)
    disk = os.path.join(args.workdir, "disk.qcow2")
    sock = os.path.join(args.workdir, "serial.sock")
    if os.path.exists(sock):
        os.remove(sock)
    log = open(os.path.join(args.workdir, "serial.log"), "a" if args.boot_only else "w")
    if args.boot_only:
        return boot_installed(args, disk, sock)
    if os.path.exists(disk):
        os.remove(disk)
    subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", disk, "40G"], check=True)

    print("[1/3] booting the installer ISO", flush=True)
    vm = qemu(args, disk, sock, cdrom=args.iso)
    try:
        con = Serial(sock, log)
        # The live user's shell is fish (Iliada's default); accept fish or sh prompts.
        con.read_until(r"nixos\S*@\S*iliada-installer.*?(> |[$#] )", 600)
        time.sleep(2)
        if args.next_to_windows:
            print("[2/3] putting a stand-in for Windows on the disk", flush=True)
            if run_as_root(con, STAND_IN_WINDOWS, "WINDOWS", 120) != 0:
                print(f"could not make the stand-in for Windows; see {log.name}", file=sys.stderr)
                return 1
        print("[2/3] running iliada-install", flush=True)
        env = ""
        if args.substituter:
            conf = f"extra-substituters = {args.substituter}\nrequire-sigs = false\n"
            if run_as_root(con, f"printf '{conf}' > /tmp/nix-extra.conf", "NIXCONF", 30) != 0:
                return 1
            env = "env NIX_USER_CONF_FILES=/tmp/nix-extra.conf "
        free = " --free-space" if args.next_to_windows else ""
        # Run under bash explicitly so the exit-status syntax is shell-independent.
        con.send(
            f"bash -c 'sudo {env}iliada-install --disk /dev/vda{free} --user alex --fullname \"Alex Test\" "
            "--hostname iliada-test --timezone Europe/Athens --password test --yes; "
            "echo INSTALL-EXIT=$?'\n"
        )
        out = con.read_until(r"INSTALL-EXIT=\d+", 5400)
        code = int(re.search(r"INSTALL-EXIT=(\d+)", out).group(1))
        if code != 0:
            print(f"iliada-install failed with {code}; see {log.name}", file=sys.stderr)
            return 1
        # Flush everything to the virtual disk, then stop the VM. (Shutting the
        # live session down cleanly is slow and irrelevant to the result.)
        con.read_until(r"(> |[$#] )", 60)
        con.send("bash -c 'sync; echo FLUSHED-$((40+2))'\n")
        con.read_until(r"FLUSHED-42", 120)
        time.sleep(2)
    finally:
        if vm.poll() is None:
            vm.kill()

    return boot_installed(args, disk, sock)


def boot_installed(args, disk, sock):
    print("[3/3] booting the installed system", flush=True)
    vm = qemu(args, disk, sock)
    try:
        start = time.time()
        if args.vncdo:
            # The first seconds: the boot menu (or, when it stays hidden, none).
            for i in range(12):
                time.sleep(max(0.0, start + 2 * (i + 1) - time.time()))
                shot = os.path.join(args.workdir, f"start-{2 * (i + 1):02d}s.png")
                subprocess.run([args.vncdo, "-s", f"127.0.0.1::{5900 + args.vnc}", "capture", shot],
                               check=False, timeout=10)
        time.sleep(max(0.0, start + 90 - time.time()))
        if vm.poll() is not None:
            print("installed system's VM exited early", file=sys.stderr)
            return 1
        if args.vncdo:
            shot = os.path.join(args.workdir, "installed.png")
            subprocess.run([args.vncdo, "-s", f"127.0.0.1::{5900 + args.vnc}", "capture", shot], check=True)
            print(f"screenshot: {shot}", flush=True)
    finally:
        vm.kill()
    print("OK", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
