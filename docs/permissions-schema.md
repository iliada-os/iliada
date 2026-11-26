# Permissions: file format and JSON schema (`iliada.permissions/1`)

What every app may use, as Iliada enforces it (ADR 0007). Two ways in:

* the files, for people: `~/.config/iliada/permissions.toml` (your answers)
  and `/etc/iliada/permissions.toml` (the computer's limits);
* `iliada permissions --json [list|show <app>]`, for agents and tools.

Both come from the same code (`iliada_bundle::policy`), so they agree.

## The user file

```toml
version = 1

[apps."org.gimp.GIMP"]
name = "GIMP"
source = "agora"
isolation = "sandbox"
worst_case = "It could delete or change everything in your Pictures folder and send your Pictures files, what your microphone hears, what you copy and your screen to anyone on the internet."
risk = [ "Can read, change and delete everything in your Pictures folder.", "Can connect to any address on the internet and send out anything it can read.", ... ]
network             = "allow"          # allow | deny | ask
"files.pictures"    = "read-write"     # read-write | read-only | deny | ask
camera              = "deny"           # allow | deny | ask
microphone          = "not-requested"  # the app does not ask for this; in effect: unrestricted; ...
"clipboard.read"    = "unrestricted"   # NOT RESTRICTED YET; the desktop gives every window the clipboard, ...
notifications       = "deny"           # fixed; there is no notification service inside the sandbox (no D-Bus)

[apps."cosmic-term"]
name = "Terminal"
source = "built-in"
all = "unrestricted"  # runs as you, outside any sandbox: it can do whatever you can
```

Only the capability values are read back; `name`, `source`, `isolation`,
`worst_case`, `risk` and the comments are regenerated from them.

## The system file (limits)

```toml
version = 1
[all]
network = "deny"                 # no app on this computer gets the internet
[apps."org.gimp.GIMP"]
"files.pictures" = "read-only"
```

Only `deny` and `read-only` (folders) are accepted: limits narrow, never
widen. Change it with `iliada permissions limit <app|all> <capability>
<deny|read-only|none>` (administrators; committed to `/etc/iliada`'s git).

## Capabilities

| Key | Values a person can set | Rule |
|---|---|---|
| `network` | allow, deny, ask | decided (default allow) |
| `files.documents`, `files.downloads`, `files.music`, `files.pictures`, `files.videos` | read-write, read-only, deny, ask | decided (default ask) |
| `camera` | allow, deny, ask | decided (default ask) |
| `microphone` | allow, deny, ask | decided (default ask) |
| `sound` | allow, deny | decided (default allow) |
| `screen_capture`, `clipboard.read`, `clipboard.write`, `background` | none | not restricted yet: always `unrestricted` |
| `notifications`, `location`, `autostart`, `devices.usb`, `devices.bluetooth`, `system_changes`, `other_apps` | none | never: always `deny` |

An app only gets a decided capability its recipe declares; otherwise it is
`not-requested`.

## JSON

```json
{
  "schema": "iliada.permissions/1",
  "files": { "user": "/home/you/.config/iliada/permissions.toml", "system": "/etc/iliada/permissions.toml" },
  "readable": true,
  "problems": [ { "file": "...", "app": "org.gimp.GIMP", "key": "camera", "message": "..." } ],
  "apps": [ AppReport, ... ]
}
```

`readable: false` means a file is broken and every decided capability is
denied until it is fixed (`problems` says where).

### AppReport

| Field | Meaning |
|---|---|
| `id`, `name` | the app's id and name |
| `source` | `agora`, `built-in` or `package` (a command-line tool) |
| `sandboxed` | false for built-in programs and packages: they can do whatever the person can |
| `isolation` | `namespace` (light sandbox; `sandbox` in the file) or `micro-vm` |
| `summary` | short list of what it can touch ("Pictures (read-write)", "Internet") |
| `worst_case` | one sentence: the most it could damage or leak |
| `risk` | plain sentences, one per risk, generated from the capabilities |
| `always` | what every sandboxed app has: its private folder (with path), files opened with it, a window |
| `asks` | capabilities it will ask about the next time it opens |
| `capabilities` | map from key to Entry |

### Entry

| Field | Meaning |
|---|---|
| `key` | the capability key |
| `value` | **what is in effect now** (allow, deny, ask, read-write, read-only, not-requested, unrestricted) |
| `setting` | what the file says (differs from `value` when a limit, the sandbox type or a broken file changes it) |
| `decided_by` | `you`, `default`, `system` (the computer's limits), `declaration` (not requested), `iliada` (fixed), `broken-file` |
| `enforced` | false when Iliada cannot restrict it yet: treat the app as able to do it |
| `changeable` | whether a person can change it in the user file |
| `scope` | exact reach, e.g. the folder path `/home/you/Pictures` |
| `note` | why the value is what it is |
| `purpose` | the app's own reason, from its recipe |

### Answering questions with it
* *Could it delete my photos?* `capabilities["files.pictures"].value ==
  "read-write"`.
* *Could it send my files online?* `capabilities.network.value == "allow"`
  and some `files.*` value is `read-write` or `read-only`; `worst_case`
  says it in one sentence.
* Anything with `enforced: false` is possible today, whatever the setting.

Stability: fields are only added within `iliada.permissions/1`; a change
in meaning gets a new schema version.
