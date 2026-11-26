# ADR 0009: Search extensions, sandboxed, from agora

**Status:** accepted, implemented

## Context
Search (Super) finds apps, files, settings and windows. People want more
sources in it: GitHub repositories, translation, a color preview, the way
Raycast and Alfred have extensions. Those extensions usually run as the
person, with all their files and the network. In Iliada nothing from
agora runs outside a sandbox, and the person can always see what an app
may do (ADR 0007). Extensions have to follow the same rules.

## Decision

### 1. An extension is an ordinary bundle
A `[search]` table in `Contents/Info.toml` (`keyword`, `title`, optional
`placeholder`) makes a bundle an extension. It may also be an app. It is
installed, updated and removed through agora like any app, and listed in
the permissions file like any app, marked with its keyword.

### 2. stdin and stdout, one JSON object per line
When the query starts with the keyword and a space, the search service
sends what follows as `{"query":"…","id":N}` and reads back
`{"id":N,"results":[…]}`. We chose this over a socket, D-Bus or a library:

* a pipe is the one channel the sandbox already allows and nothing else
  can reach; no new socket or bus name for apps to find;
* any language can read and write lines of JSON, with no Iliada library;
* the extension cannot start a conversation: it only answers, and only
  what it is asked.

The service debounces keystrokes (about 120 ms), drops replies to older
ids, waits at most 1.5 s, and keeps at most 20 results and 64 KB per
reply. It starts the extension when its keyword is first typed and stops
it when Search closes or after 30 s unused.

### 3. Three actions, nothing else
A result can copy text, open an `https://` address in the browser, or ask
Homer with text. These are the actions that cannot reach anything on the
computer: they hand text to the person, to the browser or to Homer (who
still asks before anything risky, ADR 0008). Running a command, opening a
local file or pasting would let an extension act as the person outside its
sandbox, so they do not exist. The service checks every field (lengths,
`https` with a host, icons inside the bundle) and leaves out anything
that does not fit.

### 4. How it is sandboxed
`iliada-run --search-extension` always uses the namespace tier (a
micro-VM would start too slowly for typing). Compared with an app it gets:

* no Wayland socket, so no window, clipboard or screen;
* no sound, camera, folders, accessibility bus or D-Bus;
* no broker socket, so it cannot ask for any permission, and no prompts
  are ever shown for it;
* a private writable folder, `~/.cache/iliada/search-extensions/<id>`, as
  its home, apart from the app's own data;
* the internet only when the bundle declares `network = true` and the
  permissions files allow it (`network = "deny"` turns it off, per
  extension, the same as for apps). An undecided answer means no.

The query text after the keyword is the only thing it ever learns from
the person.

### 5. Keywords
1 to 12 lowercase letters or digits. Search's own prefixes (`>`, `?`, `w`,
`~`, `/`) can never be claimed. When two extensions want the same keyword,
the one installed first keeps it; the other waits until the person gives
it another in Settings → Privacy → Search.

## Consequences
* agora's card says before installing: "Adds results to Search when you
  type 'gh'. It only sees what you type after 'gh'", and whether it can
  use the internet.
* `iliada permissions [--json]` and Homer's `list_permissions` show each
  extension's keyword, whether it is on, and whether it can use the
  internet.
* Extensions cannot show rich previews or act on files. If that is ever
  needed it will be a new, narrow action, decided in a new ADR.
