#!/usr/bin/env bash
# Boot the Iliada dev VM under QEMU/KVM, wait for the COSMIC desktop, take
# screenshots (QEMU monitor screendump), and check that core services are up.
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

monitor="$work/qemu-monitor.sock"
disk="$work/iliada-dev.qcow2"
export NIX_DISK_IMAGE=$disk
export TMPDIR=$work
# Runner RAM is tight (~7 GiB). The NixOS default for this VM is 6 GiB / 4 cores;
# last -m / -smp on the QEMU command line win.
export QEMU_OPTS="-m 3584 -smp 2 -monitor unix:${monitor},server,nowait"

echo "starting $run_vm"
# New session so we can stop QEMU and its helpers together.
set +e
setsid "$run_vm" >"$work/qemu.log" 2>&1 &
vm_pid=$!
set -e

cleanup() {
  if kill -0 "$vm_pid" 2>/dev/null; then
    kill -TERM -"$vm_pid" 2>/dev/null || kill -TERM "$vm_pid" 2>/dev/null || true
    sleep 2
    kill -KILL -"$vm_pid" 2>/dev/null || kill -KILL "$vm_pid" 2>/dev/null || true
  fi
  pkill -f qemu-system-x86_64 2>/dev/null || true
}
trap cleanup EXIT

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
  -o LogLevel=ERROR
  iliada@127.0.0.1
)

guest() {
  "${ssh_base[@]}" 'bash -s' "$@"
}

screendump() {
  local dest=$1
  python3 - "$monitor" "$dest" <<'PY'
import socket, sys, time
monitor, dest = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(10)
for _ in range(50):
    try:
        s.connect(monitor)
        break
    except OSError:
        time.sleep(0.2)
else:
    sys.exit("qemu monitor socket never appeared")
try:
    s.recv(4096)
except socket.timeout:
    pass
# PNG if this QEMU knows it; fall back to PPM.
for cmd in (f"screendump {dest} png\n", f"screendump {dest}\n"):
    s.sendall(cmd.encode())
    time.sleep(1.5)
    try:
        reply = s.recv(4096).decode("utf-8", "replace")
    except socket.timeout:
        reply = ""
    if "unknown" in reply.lower() or "error" in reply.lower():
        continue
    break
s.close()
PY
}

vm_alive() {
  kill -0 "$vm_pid" 2>/dev/null
}

echo "waiting for SSH on 127.0.0.1:2222"
deadline=$((SECONDS + 300))
until guest <<<'echo ssh-ok' | grep -q ssh-ok; do
  if (( SECONDS >= deadline )); then
    echo "SSH never came up" >&2
    screendump "$work/never-ssh.png" || true
    tail -n 200 "$work/qemu.log" >&2 || true
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
  if guest <<'EOF' | grep -q cosmic-comp-up
if pgrep -u iliada -x cosmic-comp >/dev/null; then
  echo cosmic-comp-up
fi
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
echo '=== cosmic-greeter ==='
systemctl status cosmic-greeter --no-pager -l || true
echo '=== processes ==='
ps -u iliada -o pid,cmd || true
echo '=== journal (last 150) ==='
journalctl -b -n 150 --no-pager || true
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
pgrep -a -u iliada cosmic || true
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

if [[ ! -s $work/02-desktop.png && ! -s $work/02-desktop.ppm ]]; then
  echo "desktop screenshot was not written" >&2
  ls -l "$work" >&2 || true
  exit 1
fi

echo "desktop is up; screenshots in $work"
ls -l "$work"/*.png "$work"/*.ppm 2>/dev/null || true
