# ADR 0010: Notification Center and Do Not Disturb

**Status:** accepted, implemented

## Context
On a Mac, notifications that pop up are not lost: they pile up in a
panel at the right edge of the screen, grouped by app, and Focus (Do Not
Disturb) silences them without throwing them away. COSMIC ships two
pieces: `cosmic-notifications`, the daemon that owns
`org.freedesktop.Notifications` and draws the banners, and
`cosmic-applet-notifications`, a top bar applet with a short list and a
Do Not Disturb switch. What Iliada needs and they do not have:

* the list is in memory only, so it is gone after a restart;
* COSMIC's Do Not Disturb hides every banner, critical ones too, which the
  freedesktop spec says should still show;
* no per-app rules (banners, sound, history), no schedule, no way for the
  CLI or Homer to read or change anything;
* no way to keep a private app's notifications out of any history;
* the daemon claims notification sounds but plays none.

## Decision

### 1. Iliada's own notification service, COSMIC's banners
`iliada-notifyd` owns `org.freedesktop.Notifications`. Every notification
comes to it first. It lists the app for Settings, applies the app's rules
and Do Not Disturb (`iliada_notify::rules`), keeps the notification in the
history, plays its sound (`pw-play`), and hands the ones that should pop
up to `cosmic-notifications`, which still draws them. Clicks on a banner
come back to `iliada-notifyd` and go to the app under the id the app was
given.

`cosmic-notifications` gets a two-line patch: it answers on
`org.iliada.NotificationBanners` instead, and places banners next to
Iliada's top bar button. It has to keep running anyway: `cosmic-session`
starts it and restarts the top bar whenever it exits.

We chose this over the alternatives:

* **Patching COSMIC's daemon and applet** to add history, rules, a
  schedule and a socket for the CLI and Homer would be a large patch to
  carry across every COSMIC release. The rules here are plain Rust with
  unit tests, shared by the service, Settings, the CLI and Homer.
* **Replacing the daemon outright** would mean drawing banners
  ourselves, and stopping `cosmic-session` from starting its own, which
  would then fight for the name.
* **Listening on the bus** (as a monitor) cannot hold banners back for Do
  Not Disturb or per app, and cannot send actions back to apps as the
  name's owner.

The bus starts `iliada-notifyd` the first time an app sends a
notification (`org.freedesktop.Notifications.service`, which points at
the systemd user service), so none are lost early in the session.

### 2. The panel and the top bar
`iliada-notification-center` is a layer-shell panel along the right
edge, started hidden with the session like Search, and shown by Super+N or
by `iliada-notification-applet`, the top bar button that replaces COSMIC's
clock and notifications applets: the time, and a bell, or a moon during Do
Not Disturb. Notifications are grouped by app, newest first; a group
collapses to its newest one and has Clear; Clear All clears everything.
Clicking a notification runs its default action and switches to the app
(Search's service focuses its window, as `w ` does). Esc or a click
outside closes it.

### 3. History
The last 200 notifications are kept in
`~/.local/state/iliada/notifications.json`, readable only by the person;
the oldest go first. Never kept:

* notifications from private apps: those whose "Show in history" is off
  in Settings → Notifications;
* notifications from the protected windows nobody but the person may see
  (permission prompts, the portal's consent, Homer's banner): the same
  `PROTECTED_APP_IDS` Homer's app control and Search skip;
* notifications the app marked `transient`.

### 4. Do Not Disturb
On by hand (the panel, Super+Shift+N, Settings, `iliada dnd on [--until
HH:MM | --for 1h]`, Homer), until a time, or every day on a schedule
(across midnight when the end is earlier). Switched off during the
schedule, it stays off until that stretch ends. While it is on: no
banners and no sounds, notifications still go to the panel, and the top
bar shows a moon. Critical notifications still show, as the spec says;
Settings can turn that off. It lives in
`~/.config/iliada/notifications.toml` with the per-app rules (allow,
banners, sound, history), like Iliada's other settings, and the service
reads the file again whenever it changes.

### 5. Homer
`read_notifications` ("what did I miss?"), `clear_notifications` and
`set_do_not_disturb`. Clearing and Do Not Disturb are actions, approved
like any other. Reading is sensitive: what notifications say can be
private. It is a grant in the permissions file next to Homer's other
access (`[homer] notifications = "ask"`, allow, deny or ask; ADR 0007),
shown in Settings → Notifications, and `ask` (the default) asks the person
each time. What Homer reads is fenced as untrusted text, like pages and
apps it reads.

## Consequences
* One more user service; if it is not running, apps' notifications wait
  for the bus to start it.
* `cosmic-notifications` is built from source with the patch instead of
  coming from the binary cache.
* COSMIC's own Do Not Disturb switch is gone with its applet;
  `iliada-notifyd` turns COSMIC's setting off if an earlier version left
  it on, since it would hide every banner.
* People who rearranged their top bar keep their layout: the new one is
  a default. They can add the Notification Center's button in COSMIC
  Settings → Desktop → Panel.
* Apps that send a sound of their own (`sound-file`, `sound-name`) get it;
  others get the theme's message sound, except low-urgency ones.
