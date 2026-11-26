# ADR 0008: Homer uses other apps, gated by the permissions file

**Status:** accepted, implemented

## Context
Homer could manage Iliada itself (install and open apps, change settings,
send email, run commands) but could not see or use another app's window,
so he could not drive Chrome or type into a Google Doc. People asked for
this without adding MCP or a per-app plugin. Iliada controls the desktop,
so it can grant this to Homer only, and the permissions file (ADR 0007)
is already the one description of what each app may do, for people and
for agents.

## Decision

### 1. Four layers, one service
One user service, `iliada-control`, is the only way Homer touches another
app. It listens on a private socket (`$XDG_RUNTIME_DIR/iliada-control.sock`),
not a port. Four ways to act, used in this order:

1. **Accessibility (AT-SPI).** `read_ui` and `act_ui` read and press the
   controls of apps that describe themselves (GTK, Qt, COSMIC, many
   others). Sandboxed apps get a filtered `xdg-dbus-proxy` to the
   accessibility bus, not the session bus; Homer sees only that app's
   tree. Apps that do not speak accessibility, or whose tree is empty,
   fall through.
2. **The screen, a pointer and a keyboard.** A screenshot, then a virtual
   pointer and keyboard granted only to this service through the desktop
   portal (not to sandboxed apps). Used when accessibility is missing.
3. **Chrome, over a pipe.** Homer's own Chrome window, launched with
   `--remote-debugging-pipe`. The protocol runs on two pipes this process
   holds. There is no port, so nothing on the computer or the network can
   attach. This is how he reads pages, fills forms and edits Google Docs
   in a signed-in session. The everyday Chrome profile is never opened
   this way: Chrome itself refuses remote control of it.
4. **App actions (intents).** Apps declare exact actions in an
   `Intents.toml` (or `/run/current-system/sw/share/iliada/intents` for
   Iliada's own apps): new note, search in agora, and so on. These are
   faster and exact, and do not need control of the window.

### 2. The permissions file is the gate
The same `~/.config/iliada/permissions.toml` (ADR 0007) holds:

* `[homer] control = true|false`: the global switch (Settings → Homer →
  Use other apps, or `iliada-control on|off`).
* `[homer] screen = "allow"|"deny"|"ask"`: screenshots and the virtual
  pointer and keyboard.
* `controlled_by_homer = "allow"|"deny"|"ask"` on every app, including
  built-in programs that otherwise say `all = "unrestricted"`. This is
  decided for every app, even if the app does not declare it: it is about
  what Homer may do, not what the app asks for. The default is `ask`.

`/etc/iliada/permissions.toml` can only narrow: `[homer] control = false`
or `controlled_by_homer = "deny"` for one app or `[all]`. A "no" there
always wins. A file that cannot be read means off.

### 3. Visible, stoppable, confirmed, logged
A banner, "Homer is controlling <App>", stays on screen while he works,
with Stop. **Super+Escape** (and `iliada-control stop`) ends it at once;
Homer lets go until the person sends a new message. He asks before
anything that sends, submits, deletes, posts or pays, even in Auto mode
and even when the app was already allowed. Permission prompts, password
windows, Settings and Homer's own banner are never readable or
controllable. What he reads in apps and pages is wrapped so the model
treats it as data, never as instructions, whatever it says. Every action
is appended to a log (`~/.local/share/iliada/homer-control-log.jsonl`).

## Consequences
* People and agents read the same file to see whether Homer may use an
  app, just as they do for folders and the camera.
* Apps without accessibility still work, through the screen, at the cost
  of being less exact.
* Chrome's everyday windows are out of reach on purpose; Docs and the web
  go through Homer's own window.
* Clipboard history, Quick Look and drag and drop in Search are separate
  work, not part of this decision.
