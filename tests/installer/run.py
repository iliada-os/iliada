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
import zlib


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


class Monitor:
    """QEMU's human monitor: screenshots and key presses (more dependable
    than VNC while the firmware and the boot menu change the screen mode)."""

    def __init__(self, path: str):
        deadline = time.time() + 30
        while True:
            try:
                self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.s.connect(path)
                break
            except OSError:
                if time.time() > deadline:
                    raise
                time.sleep(0.2)
        self.s.settimeout(5)
        self._read()

    def _read(self):
        out = b""
        while not out.endswith(b"(qemu) "):
            out += self.s.recv(65536)
        return out

    def command(self, line: str):
        self.s.sendall(line.encode() + b"\n")
        self._read()

    def screen(self, tmp: str):
        """The screen now, as (width, height, RGB bytes)."""
        self.command(f"screendump {tmp}")
        image = read_ppm(tmp)
        os.remove(tmp)
        return image

    def screenshot(self, png: str):
        write_png(png, *self.screen(png + ".ppm"))


def boot_menu_on_screen(image) -> bool:
    """Iliada's boot menu: the navy background (the firmware and the
    kernel's console are black, the greeter is a picture)."""
    width, height, pixels = image
    i = ((height // 20) * width + width // 2) * 3
    r, g, b = pixels[i], pixels[i + 1], pixels[i + 2]
    return r < 70 and b > 60 and b > r + 25


def read_ppm(ppm: str):
    """A binary PPM (what screendump writes): (width, height, RGB bytes)."""
    data = open(ppm, "rb").read()
    fields, pos = [], 0
    while len(fields) < 4:  # P6, width, height, maxval
        while data[pos:pos + 1].isspace():
            pos += 1
        start = pos
        while not data[pos:pos + 1].isspace():
            pos += 1
        fields.append(data[start:pos])
    width, height = int(fields[1]), int(fields[2])
    return width, height, data[pos + 1:]


def write_png(png: str, width: int, height: int, pixels: bytes):
    """RGB bytes as a PNG, with the standard library."""
    rows = b"".join(b"\0" + pixels[y * width * 3:(y + 1) * width * 3] for y in range(height))

    def chunk(kind, body):
        return (len(body).to_bytes(4, "big") + kind + body
                + zlib.crc32(kind + body).to_bytes(4, "big"))

    with open(png, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", width.to_bytes(4, "big") + height.to_bytes(4, "big") + bytes([8, 2, 0, 0, 0])))
        f.write(chunk(b"IDAT", zlib.compress(rows, 6)))
        f.write(chunk(b"IEND", b""))


def qemu(args, disk, sock, cdrom=None):
    cmd = [
        args.qemu, "-enable-kvm", "-machine", "q35", "-cpu", "host", "-m", "4096", "-smp", "4",
        "-bios", args.ovmf,
        "-drive", f"file={disk},if=virtio,format=qcow2",
        "-nic", "user,model=virtio-net-pci",
        "-serial", f"unix:{sock},server=on,wait=off",
        "-vga", "none", "-device", "virtio-vga,xres=1600,yres=1000",
        "-display", "none", "-vnc", f"127.0.0.1:{args.vnc}",
        "-monitor", f"unix:{os.path.join(args.workdir, 'monitor.sock')},server=on,wait=off",
    ]
    if cdrom:
        # First in the boot order even when the disk can start a system.
        cmd += ["-drive", f"file={cdrom},if=none,id=cd,media=cdrom,readonly=on",
                "-device", "ide-cd,drive=cd,bootindex=0"]
    return subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--iso", required=True)
    ap.add_argument("--qemu", required=True)
    ap.add_argument("--ovmf", required=True)
    ap.add_argument("--workdir", required=True)
    ap.add_argument("--vnc", type=int, default=1, help="VNC display number (port 5900+N)")
    ap.add_argument("--vncdo", help="unused (screenshots now come from QEMU's monitor); still accepted")
    ap.add_argument("--keys", help="keys to press once the boot menu is on screen, "
                    "e.g. 'down down ret' (QEMU sendkey names)")
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
    mon_sock = os.path.join(args.workdir, "monitor.sock")
    if os.path.exists(mon_sock):
        os.remove(mon_sock)
    vm = qemu(args, disk, sock)
    try:
        start = time.time()
        mon = Monitor(mon_sock)
        # The first seconds: a screenshot every two, and whether (and when)
        # the boot menu shows; --keys are pressed as soon as it does.
        keys = args.keys.split() if args.keys else []
        tmp = os.path.join(args.workdir, "screen.ppm")
        menu_seen, next_shot = False, 2.0
        while time.time() - start < 24:
            image = mon.screen(tmp)
            now = time.time() - start
            if now >= next_shot:
                write_png(os.path.join(args.workdir, f"start-{int(next_shot):02d}s.png"), *image)
                next_shot += 2
            if not menu_seen and boot_menu_on_screen(image):
                menu_seen = True
                print(f"boot menu on screen after {now:.1f} s", flush=True)
                write_png(os.path.join(args.workdir, "menu.png"), *image)
                for key in keys:
                    mon.command(f"sendkey {key}")
                    time.sleep(0.3)
                if keys:
                    mon.screenshot(os.path.join(args.workdir, "menu-after-keys.png"))
            time.sleep(0.5)
        if not menu_seen:
            print("no boot menu in the first 24 s", flush=True)
        time.sleep(max(0.0, start + 90 - time.time()))
        if vm.poll() is not None:
            print("installed system's VM exited early", file=sys.stderr)
            return 1
        shot = os.path.join(args.workdir, "installed.png")
        mon.screenshot(shot)
        print(f"screenshot: {shot}", flush=True)
    finally:
        vm.kill()
    print("OK", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
