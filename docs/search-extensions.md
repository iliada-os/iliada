# Writing a Search extension

A Search extension adds results to Search (Super) under a keyword. Type
the keyword and a space, then what you are looking for: `color #ff0000`,
`gh ripgrep`, `tr bonjour`. The extension gets what you typed after the
keyword and answers with results. Why it works this way: ADR 0009.

An extension is an ordinary `.app` bundle from agora. It always runs in
its sandbox, with no window, no folders and no clipboard, and it only
ever sees what is typed after its keyword.

## The bundle

Add a `[search]` table at the end of `Contents/Info.toml`:

```toml
id = "org.example.color"
name = "Color"
version = "0.1.0"
executable = "color"
entry = "usr/bin/iliada-search-color"   # the program, inside Contents/Root
runtime = "iliada-base-1"
isolation = "namespace"
# network = true                        # only if it needs the internet

[search]
keyword = "color"                       # 1 to 12 lowercase letters or digits
title = "Color"                         # shown in results and Settings
placeholder = "#ff0000, rgb(255, 0, 0) or a color name"   # optional
```

* `keyword` cannot be one of Search's own prefixes: `>`, `?`, `w`, `~`,
  `/`. If another extension has it already, the one installed first keeps
  it and yours waits for the person to choose another keyword in Settings.
* `network = true` is shown in agora before installing ("and can use the
  internet"), and the person can still turn it off in the permissions
  file, as for any app.

## The protocol

The program reads requests on stdin and writes replies on stdout, one
JSON object per line (end each reply with a newline and flush).

```text
→ {"query":"#ff0000","id":7}
← {"id":7,"results":[
     {"title":"#ff0000","subtitle":"Hex","action":{"copy":"#ff0000"}},
     {"title":"Open in a color picker","icon":"Resources/picker.png",
      "action":{"url":"https://example.org/?c=ff0000"}}]}
```

* Answer with the same `id`. Replies to older ids are dropped: the person
  has typed on.
* Each result has a `title` (up to 200 characters), an optional
  `subtitle` (up to 300), an optional `icon` (a PNG or SVG path inside
  `Contents/`) and one `action`.
* The action is exactly one of:
  * `{"copy": "text"}` copies the text (up to 10,000 characters);
  * `{"url": "https://…"}` opens the address in the browser (`https` only);
  * `{"ask_homer": "text"}` asks Homer.
* Anything else is left out: other actions, unknown fields, `http://` or
  `file://` addresses, icons outside the bundle. An extension can never
  run a command, open a local file or paste.
* At most 20 results and 64 KB per reply. No reply within 1.5 seconds
  shows "No results from Color".

Search waits about 120 ms after a keystroke before asking, so you get one
request per pause in typing, not one per key. Your program is started
when its keyword is first typed, kept running while Search is open, and
stopped when Search closes or after 30 seconds without a request. Write
anything you want to keep (a cache) to `$HOME`, which is your private
folder (`~/.cache/iliada/search-extensions/<id>` outside the sandbox).
stderr is discarded.

## The sandbox

It runs like an app in the namespace sandbox, with less:

| | Apps | Search extensions |
|---|---|---|
| Window, clipboard, screen | yes (a window) | no |
| Folders, camera, microphone, sound | when the person allows | never |
| Asking for permissions | yes | never |
| Internet | when declared and allowed | when declared and allowed |
| Private folder | `~/.local/share/iliada/apps/<id>` | `~/.cache/iliada/search-extensions/<id>` |

## An example

`examples/search-extension-color` is a small extension in Rust (keyword
`color`, no internet). Build it into its bundle and install it:

```sh
cargo xtask bundle-color
cp -r examples/bundles/color.app /Applications/
```

Then press Super and type `color teal`.

## Checking yours

* `iliada permissions show <your app>` shows its keyword and whether it
  can use the internet.
* Settings → Privacy → Search lists it, with its keyword, and lets the
  person turn it off or change its keyword.
