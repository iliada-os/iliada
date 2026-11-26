# ADR 0007: One readable permissions file, truthful about what is enforced

**Status:** accepted, implemented

## Context
ADR 0006 asks for folders, the camera and the microphone on first launch and
stores the answers in `~/.local/share/iliada/permissions.toml`. That file
held only answers. Nothing said, in one place, what an app could reach, how
far, and what it could do at worst, and nothing said which capabilities
Iliada cannot restrict yet. People could not easily check an app, and Homer
or another agent could not answer "could this app delete my photos, or send
my files online?" precisely.

## Decision

### 1. Two files, one format
* **`~/.config/iliada/permissions.toml`: each person's answers.** Written by
  the first-launch question, Settings → Privacy, `iliada permissions set`
  and changes Homer proposes (after the person confirms them). It is
  per-user because answers are personal and multi-user computers need
  separate ones; writable without root; and part of the Iliada account
  backup (`iliada-account`), so it comes back on a new computer.
* **`/etc/iliada/permissions.toml`: the computer's limits**, for every
  person on it. Values can only narrow: `deny`, or `read-only` for a
  folder, under `[all]` (every app) or `[apps."<id>"]`. It lives in
  `/etc/iliada` because that directory is git-backed, backed up and
  undoable (ADR 0006); it is changed only through `iliada-systemd`
  (`SetPermissionLimit`, administrators, polkit
  `org.iliada.system.configure`), which commits and backs up each change.
  `iliada permissions limit` is the CLI for it.

What an app gets is the smallest of: what its recipe declares, the
computer's limit, and the person's answer. Undeclared is never granted.

### 2. The file explains itself
Iliada generates the whole user file: a header explaining the format, one
table per program (agora apps, built-in programs, command-line packages)
with `name`, `source`, `isolation`, a `worst_case` sentence and a `risk`
list, then one line per capability with a comment listing the choices and,
where it differs, the value *in effect*. People can edit values by hand;
everything else is regenerated from the values the next time Iliada writes,
so the words can never drift from the values. Capabilities:

`network`, `files.documents|downloads|music|pictures|videos`, `camera`,
`microphone`, `sound`, `screen_capture`, `clipboard.read`,
`clipboard.write`, `background`, `notifications`, `location`, `autostart`,
`devices.usb`, `devices.bluetooth`, `system_changes`, `other_apps`.

### 3. Truthful: the file says exactly what is enforced
Every capability has one of three rules (`policy.rs`, `Key::rule`):

| Rule | Capabilities | Enforced by |
|---|---|---|
| Decided by the person | network (default allow), sound (default allow), each folder (ask; read-write, read-only or deny), camera (ask), microphone (ask) | bubblewrap binds (`--bind` / `--ro-bind`), device nodes, the sound server's sockets, network namespace (`--unshare-net`) or no passt in a micro-VM |
| Never given | notifications, location, autostart, USB, Bluetooth, system changes, talking to other apps | the sandbox has no D-Bus, no devices, no system paths |
| **Not restricted yet** (`unrestricted`) | screen capture, clipboard read and write, running in the background | nothing yet; said in the file, Settings, agora and the JSON |

The report adjusts values to how the sandbox really works, so it never
implies safety: allowing sound makes the microphone `unrestricted` (one
socket carries both, until PipeWire's portal access control is used);
allowing the microphone does the same for sound; a micro-VM has no camera;
with network on the light sandbox, `other_apps` is in effect unrestricted
(it can reach programs listening on localhost); with network on, location
notes that a rough location follows from the IP address. Built-in programs
and command-line tools are listed with `all = "unrestricted"`: they run as
the person.

### 4. A broken file never widens access
If the user file cannot be parsed, has an unknown `version` or wrong types,
every decided capability is denied for every app (`decided_by =
"broken-file"`), nothing is asked and nothing is written over it until it is
fixed or reset (`iliada permissions reset` keeps a `.broken` copy). A value
that is not understood denies that one capability. Unknown keys grant
nothing. A broken system file denies too. Hand edits apply at the next
launch; `iliada permissions check` reports mistakes with the file, app and
key.

### 5. One description for people and agents
`policy::Policy::report` produces one `AppReport` per program (summary,
worst case, risk, and per capability: value, setting, who decided it,
whether it is enforced, its exact scope and a note). The same report feeds:
the file's comments; Settings → Privacy (per app and per capability, with a
card per app); agora's card before install (`Policy::preview` over the
capabilities the catalog lists for the app, `AppEntry.permissions`);
`iliada permissions [--json]`; and Homer's `list_permissions` and
`set_permission` tools. Schema: [docs/permissions-schema.md](../permissions-schema.md),
`iliada.permissions/1`.

Homer can only propose changes: `set_permission` always goes through the
confirmation window, widening or narrowing, and says the worst case after
the change.

### 6. The broker's grants stay separate
`iliada-broker` is one system-wide service that identifies apps by socket,
not by person, so it cannot own per-person answers. It now reads the
computer's limits (`/etc/iliada/permissions.toml`, `--limits`) on every
decision and refuses what they deny; its Once / While running / Always
grants stay in its own store and audit log. Moving them into the per-user
file needs per-user brokers (later work).

## Consequences
* The old answers file is read once when the new one is missing; folder
  answers of `allow` become `read-write`. The account backup carries both
  paths.
* Network is now a person's choice per app (it was fixed by the recipe).
* Catalogs published before this change do not list `permissions`; agora
  says so rather than showing an empty card. Regenerating the catalog fills
  it.
* Clipboard, screen capture and background running are visibly
  "not restricted yet" until the compositor and portals enforce them.
  Sandboxed apps are already refused clipboard watching and screen
  capture (their display socket). Clipboard history is held only by the
  unsandboxed search service (Super+V) and is not a per-app grant.

See also ADR 0008: Homer's use of other apps is gated by this
file (`[homer]` and `controlled_by_homer`).
