#!/usr/bin/env bash
# Boot the Iliada dev VM under QEMU/KVM, wait for the COSMIC desktop, take
# screenshots, and check that core services are up.
# Used by .github/workflows/vm.yml. Login shell is fish: scripts go through bash -s.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

work="$root/vm-run"
rm -rf "$work"
mkdir -p "$work"

run_vm=$(echo "$root"/result-vm/bin/run-*-vm)
if [[ ! -x $run_vm ]]; then
  echo "no run-*-vm in result-vm/bin; build the VM first" >&2
  exit 1
fi

qmp="$work/qmp.sock"
control="$work/ssh-control.sock"
disk="$work/iliada-dev.qcow2"
export NIX_DISK_IMAGE=$disk
export TMPDIR=$work
# Do not pass -m: the wrapper already sets -m from virtualisation.memorySize,
# and virtiofs pins a matching memory-backend-memfd. A second -m makes QEMU
# exit with "memory size does not match the size of the memory backend".
# Extra QEMU flags only. Do not replace qemu.options in the VM config:
# that list carries -kernel, virtiofs and the virtiofs memfd.
export QEMU_OPTS="-qmp unix:${qmp},server,nowait -serial file:${work}/serial.log"

vm_alive() {
  kill -0 "$vm_pid" 2>/dev/null
}

cleanup() {
  if [[ -n ${control-} && -S $control ]]; then
    ssh -O exit -o ControlPath="$control" dummy 2>/dev/null || true
  fi
  if [[ -n ${vm_pid-} ]] && kill -0 "$vm_pid" 2>/dev/null; then
    kill -TERM -"$vm_pid" 2>/dev/null || kill -TERM "$vm_pid" 2>/dev/null || true
    sleep 2
    kill -KILL -"$vm_pid" 2>/dev/null || kill -KILL "$vm_pid" 2>/dev/null || true
  fi
  pkill -f qemu-system-x86_64 2>/dev/null || true
}
trap cleanup EXIT

echo "starting $run_vm"
# New session so we can stop QEMU and its helpers together.
set +e
setsid "$run_vm" >"$work/qemu.log" 2>&1 &
vm_pid=$!
set -e
# Give the wrapper time to format the disk and exec QEMU before we treat a
# missing process as a crash.
sleep 5
if ! vm_alive; then
  echo "VM process exited while starting" >&2
  tail -n 200 "$work/qemu.log" >&2 || true
  exit 1
fi

ssh_base=(
  sshpass -p iliada
  ssh
  -p 2222
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o NumberOfPasswordPrompts=1
  -o ConnectTimeout=5
  -o ServerAliveInterval=15
  -o ControlMaster=auto
  -o ControlPath="$control"
  -o ControlPersist=60
  -o LogLevel=ERROR
  iliada@127.0.0.1
)

guest() {
  "${ssh_base[@]}" 'bash -s' "$@"
}

# Press keys in the VM through QMP: each argument is one press, keys held
# together joined by "+" (qcodes: meta_l, ret, esc, a, b, ...).
keys() {
  python3 - "$qmp" "$@" <<'PY' || true
import json, socket, sys, time

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sys.argv[1])
f = s.makefile("rwb")
f.readline()
f.write(b'{"execute":"qmp_capabilities"}\n'); f.flush()
while b'"return"' not in f.readline():
    pass
for press in sys.argv[2:]:
    combo = [{"type": "qcode", "data": k} for k in press.split("+")]
    f.write((json.dumps({"execute": "send-key", "arguments": {"keys": combo, "hold-time": 60}}) + "\n").encode())
    f.flush()
    while True:
        line = f.readline()
        if not line or b'"return"' in line or b'"error"' in line:
            break
    time.sleep(0.15)
s.close()
PY
}

# QEMU QMP screendump, then localhost VNC. VNC still uses password
# iliada (same as the mutable VM); QMP is the one that must work.
screendump() {
  local dest=$1
  python3 - "$qmp" "$dest" <<'PY' || true
import json, os, socket, struct, sys, time

qmp_path, dest = sys.argv[1], sys.argv[2]


def log(msg):
    print(f"screendump: {msg}", flush=True)


def qmp_screenshot(path, dest):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(10)
    for _ in range(50):
        try:
            s.connect(path)
            break
        except OSError:
            time.sleep(0.2)
    else:
        log("QMP socket never appeared")
        return False

    buf = bytearray()

    def read_obj():
        nonlocal buf
        while True:
            try:
                text = buf.decode("utf-8")
                obj, idx = json.JSONDecoder().raw_decode(text.lstrip())
                rest = text.lstrip()[idx:].lstrip()
                buf[:] = rest.encode()
                return obj
            except (json.JSONDecodeError, UnicodeDecodeError):
                chunk = s.recv(4096)
                if not chunk:
                    raise RuntimeError("QMP closed")
                buf.extend(chunk)

    greeting = read_obj()
    if "QMP" not in greeting:
        log(f"unexpected QMP greeting: {greeting!r}")
        s.close()
        return False
    s.sendall(b'{"execute":"qmp_capabilities"}\n')
    while True:
        msg = read_obj()
        if msg.get("error"):
            log(f"qmp_capabilities failed: {msg}")
            s.close()
            return False
        if "return" in msg:
            break
    base, ext = os.path.splitext(dest)
    for fmt, out in (("png", dest), ("ppm", base + ".ppm")):
        cmd = {
            "execute": "screendump",
            "arguments": {"filename": out, "format": fmt},
        }
        s.sendall((json.dumps(cmd) + "\n").encode())
        while True:
            msg = read_obj()
            if msg.get("event"):
                continue
            break
        if msg.get("error"):
            log(f"QMP {fmt} rejected: {msg['error']}")
            continue
        time.sleep(0.4)
        if os.path.isfile(out) and os.path.getsize(out) > 0:
            log(f"wrote {out} via QMP ({os.path.getsize(out)} bytes)")
            s.close()
            return True
        log(f"QMP {fmt} returned but {out} is missing or empty")
    s.close()
    return False


def vnc_screenshot(dest):
    s = socket.create_connection(("127.0.0.1", 5900), 8)
    s.settimeout(20)
    ver = s.recv(12)
    if not ver.startswith(b"RFB "):
        log(f"not a VNC server: {ver!r}")
        s.close()
        return False
    s.sendall(b"RFB 003.008\n")
    nsec = s.recv(1)
    if not nsec:
        log("VNC closed during security types")
        s.close()
        return False
    types = s.recv(nsec[0])
    if 1 not in types:
        log(f"VNC has no None security (types={list(types)}); need passwordless VNC")
        s.close()
        return False
    s.sendall(bytes([1]))
    result = struct.unpack(">I", s.recv(4))[0]
    if result != 0:
        reason_len = struct.unpack(">I", s.recv(4))[0] if result else 0
        reason = s.recv(reason_len) if reason_len else b""
        log(f"VNC security failed ({result}): {reason!r}")
        s.close()
        return False
    s.sendall(bytes([1]))  # shared
    hdr = b""
    while len(hdr) < 24:
        chunk = s.recv(24 - len(hdr))
        if not chunk:
            log("VNC closed during ServerInit")
            s.close()
            return False
        hdr += chunk
    width, height = struct.unpack(">HH", hdr[:4])
    name_len = struct.unpack(">I", hdr[20:24])[0]
    leftover = hdr[24:]
    while len(leftover) < name_len:
        leftover += s.recv(name_len - len(leftover))
    # 32-bit little-endian RGB, raw encoding only.
    pixfmt = struct.pack(
        ">BBBBHHHBBBxxx", 32, 24, 0, 1, 255, 255, 255, 16, 8, 0
    )
    s.sendall(b"\x00\x00\x00\x00" + pixfmt)
    s.sendall(struct.pack(">BBH", 2, 0, 1) + struct.pack(">i", 0))
    s.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, width, height))
    pixels = bytearray(width * height * 4)
    got = 0
    deadline = time.time() + 15
    while got < width * height and time.time() < deadline:
        kind = s.recv(1)
        if not kind:
            break
        if kind[0] != 0:
            log(f"unexpected VNC message type {kind[0]}")
            s.close()
            return False
        s.recv(1)
        nrect_b = b""
        while len(nrect_b) < 2:
            nrect_b += s.recv(2 - len(nrect_b))
        nrect = struct.unpack(">H", nrect_b)[0]
        for _ in range(nrect):
            rh = b""
            while len(rh) < 12:
                chunk = s.recv(12 - len(rh))
                if not chunk:
                    log("VNC closed during rectangle")
                    s.close()
                    return False
                rh += chunk
            x, y, w, h, enc = struct.unpack(">HHHHi", rh)
            if enc != 0:
                log(f"unsupported VNC encoding {enc}")
                s.close()
                return False
            need = w * h * 4
            data = b""
            while len(data) < need:
                chunk = s.recv(need - len(data))
                if not chunk:
                    log("VNC closed during pixels")
                    s.close()
                    return False
                data += chunk
            for row in range(h):
                src = row * w * 4
                dst = ((y + row) * width + x) * 4
                pixels[dst : dst + w * 4] = data[src : src + w * 4]
            got += w * h
    s.close()
    if got < width * height:
        log(f"VNC incomplete frame ({got}/{width * height} pixels)")
        return False
    ppm = os.path.splitext(dest)[0] + ".ppm"
    with open(ppm, "wb") as f:
        f.write(f"P6\n{width} {height}\n255\n".encode())
        for i in range(0, len(pixels), 4):
            f.write(bytes([pixels[i + 2], pixels[i + 1], pixels[i]]))
    log(f"wrote {ppm} via VNC ({os.path.getsize(ppm)} bytes, {width}x{height})")
    return True


ok = False
try:
    ok = qmp_screenshot(qmp_path, dest)
except Exception as e:
    log(f"QMP failed: {e}")
if not ok:
    try:
        ok = vnc_screenshot(dest)
    except Exception as e:
        log(f"VNC failed: {e}")
if not ok:
    sys.exit(1)
PY
}

echo "waiting for SSH on 127.0.0.1:2222"
deadline=$((SECONDS + 300))
until guest <<<'echo ssh-ok' | grep -q ssh-ok; do
  if (( SECONDS >= deadline )); then
    echo "SSH never came up" >&2
    screendump "$work/never-ssh.png" || true
    tail -n 200 "$work/qemu.log" >&2 || true
    if [[ -f $work/serial.log ]]; then
      echo "=== serial ===" >&2
      tail -n 200 "$work/serial.log" >&2 || true
    fi
    exit 1
  fi
  if ! vm_alive; then
    echo "VM process exited before SSH" >&2
    tail -n 200 "$work/qemu.log" >&2 || true
    exit 1
  fi
  sleep 3
done
echo "SSH is up"
screendump "$work/01-ssh-up.png" || true

echo "waiting for cosmic-comp (the COSMIC compositor)"
deadline=$((SECONDS + 300))
desktop_up=0
while (( SECONDS < deadline )); do
  if ! vm_alive; then
    echo "VM process exited while waiting for the desktop" >&2
    tail -n 200 "$work/qemu.log" >&2 || true
    exit 1
  fi
  # cosmic-comp is wrapped (.cosmic-comp-wrapped); comm is truncated to 15
  # chars, so pgrep -x cosmic-comp never matches. Scan command lines instead.
  # A ControlMaster keeps this from opening a new logind session each poll.
  if guest <<'EOF' | grep -q cosmic-comp-up
# [c]osmic-comp: this grep's own argv must not match.
for c in /proc/[0-9]*/cmdline; do
  tr '\0' ' ' < "$c" 2>/dev/null | grep -q '[c]osmic-comp' || continue
  echo cosmic-comp-up
  break
done
EOF
  then
    desktop_up=1
    break
  fi
  sleep 4
done

if [[ $desktop_up -ne 1 ]]; then
  echo "COSMIC desktop never came up (no cosmic-comp)" >&2
  screendump "$work/never-desktop.png" || true
  guest <<'EOF' >"$work/guest-debug.txt" || true
echo '=== failed units ==='
systemctl --failed --no-legend --plain || true
echo '=== display-manager ==='
systemctl status display-manager --no-pager -l || true
echo '=== greetd ==='
systemctl status greetd --no-pager -l || true
journalctl -u greetd -b --no-pager -n 80 || true
echo '=== cosmic-session ==='
journalctl -t cosmic-session -b --no-pager -n 80 || true
echo '=== loginctl ==='
loginctl || true
echo '=== dri ==='
ls -l /dev/dri 2>/dev/null || true
echo '=== processes matching cosmic/greetd/session ==='
for c in /proc/[0-9]*/cmdline; do
  cmd=$(tr '\0' ' ' < "$c" 2>/dev/null) || continue
  case $cmd in
    *cosmic*|*greetd*|*start-cosmic*) echo "${c#/proc/} $cmd";;
  esac
done
echo '=== comms matching cosmic ==='
grep -H . /proc/[0-9]*/comm 2>/dev/null | grep -i cosmic || true
EOF
  exit 1
fi
echo "cosmic-comp is running"
sleep 8
screendump "$work/02-desktop.png"
sleep 15
screendump "$work/03-desktop-later.png"

echo "checking core Iliada services"
set +e
guest <<'EOF' | tee "$work/guest-status.txt"
set -u
fail=0
echo '=== core services ==='
for u in iliada-broker iliada-appd iliada-system; do
  if systemctl is-active --quiet "$u"; then
    echo "$u: active"
  else
    echo "$u: NOT active"
    systemctl status "$u" --no-pager -l || true
    fail=1
  fi
done
echo '=== failed units ==='
systemctl --failed --no-legend --plain || true
echo '=== session ==='
loginctl || true
pgrep -a -f cosmic || true
echo "CORE_FAIL=$fail"
EOF
guest_status=${PIPESTATUS[0]}
set -e

if [[ $guest_status -ne 0 ]]; then
  echo "guest status script failed" >&2
  exit 1
fi
if grep -q 'CORE_FAIL=1' "$work/guest-status.txt"; then
  echo "a core Iliada service is not active" >&2
  exit 1
fi

echo "checking the search (iliada-searchd, the Super key overlay)"
set +e
guest <<'EOF' | tee "$work/search.txt"
export XDG_RUNTIME_DIR=/run/user/$(id -u)
fail=0
for i in $(seq 1 30); do
  iliada-search stats >/dev/null 2>&1 && break
  sleep 2
done
systemctl --user status iliada-searchd --no-pager -l | head -n 20 || true
iliada-search stats || fail=1
echo '=== query: files ==='
iliada-search query files | tee /tmp/q.txt || fail=1
grep -q '^Application *Files' /tmp/q.txt || { echo "Files app not found by the search"; fail=1; }
echo '=== query: 2+3*4 ==='
iliada-search query '2+3*4' | grep -q '= 14' || { echo "no answer for 2+3*4"; fail=1; }
pgrep -a -f iliada-search-overlay || echo "overlay not running yet (started on first Super)"
echo "SEARCH_FAIL=$fail"
EOF
set -e
# Super shows the search; type "term"; Esc closes.
keys meta_l
sleep 2
screendump "$work/04-search.png"
keys t e r m
sleep 1
screendump "$work/05-search-term.png"
keys esc
if grep -q 'SEARCH_FAIL=1' "$work/search.txt"; then
  echo "the search service does not answer" >&2
  exit 1
fi

if [[ ! -s $work/02-desktop.png && ! -s $work/02-desktop.ppm ]]; then
  echo "desktop screenshot was not written" >&2
  ls -l "$work" >&2 || true
  exit 1
fi

echo "desktop is up; screenshots in $work"
ls -l "$work"/*.png "$work"/*.ppm 2>/dev/null || true
