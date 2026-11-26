# Iliada Settings, the Setup Assistant, Homer and the permission prompt (English).


## Shared words
cancel = Cancel
save = Save
remove = Remove
open = Open
edit = Edit
retry = Retry
try-again = Try again
continue = Continue
back = Back
done = Done
skip = Skip
restore = Restore
sign-out = Sign out
loading = Loading…
iliada = Iliada
homer = Homer
agora = agora
settings = Settings

## Times
ago-now = just now
ago-minutes =
    { $count ->
        [one] 1 minute ago
       *[other] { $count } minutes ago
    }
ago-hours =
    { $count ->
        [one] 1 hour ago
       *[other] { $count } hours ago
    }
ago-days =
    { $count ->
        [one] 1 day ago
       *[other] { $count } days ago
    }
test-update-version = { $version }, test update of { $date }
month-1 = Jan
month-2 = Feb
month-3 = Mar
month-4 = Apr
month-5 = May
month-6 = Jun
month-7 = Jul
month-8 = Aug
month-9 = Sep
month-10 = Oct
month-11 = Nov
month-12 = Dec

## What the system service is doing
activity-working = Working
activity-installing-tools = Installing tools
activity-removing-tools = Removing tools
activity-applying = Applying settings
activity-assistant = Saving assistant settings
activity-language = Changing the language
activity-updating-apps = Updating apps
activity-updating-iliada = Updating Iliada
activity-update-channel = Changing how Iliada updates
activity-github-sign-in = Signing in to GitHub
activity-backing-up = Backing up
activity-restoring = Restoring a backup
activity-signing-out = Signing out
activity-restore-settings = Bringing back your apps and tools

## Pages
page-account = Iliada account
page-account-desc = Your settings on a new computer
page-updates = Updates
page-updates-desc = Iliada and app updates
page-backup = Backup
page-backup-desc = Back up apps and settings to GitHub
page-tools = Command-line tools
page-tools-desc = Software for the terminal
page-services = Services
page-services-desc = SSH, printing, web servers and more
page-privacy = Privacy
page-privacy-desc = What each app may use
page-homer = Homer
page-homer-desc = Iliada's assistant
page-about = About Iliada
page-about-desc = Version and hardware report
page-language = Language
page-language-desc = Iliada's language and keyboard layouts

## Permissions
cap-camera = Camera
cap-microphone = Microphone
cap-folder = { $folder } folder
folder-documents = Documents
folder-downloads = Downloads
folder-music = Music
folder-pictures = Pictures
folder-videos = Videos

## Done
installed-tools-done = Installed { $names }.
removed-tools-done = Removed { $names }.
settings-applied = Settings applied.
apps-up-to-date = Apps are up to date.
iliada-installed-restart = Iliada { $version } is installed. Restart the computer to finish.
iliada-up-to-date = Iliada is up to date.
iliada-available = Iliada { $version } is available.
testing-on-ready = This computer now gets test updates. One is ready: choose Update now.
testing-on = This computer now gets test updates.
testing-off = This computer now gets only releases. It keeps the test update it has until the next release.
signed-in-done = Signed in as { $account }.
backed-up = Backed up.
restored-installing = Restored. Your apps are being installed in the background.
signed-out-backups-off = Signed out. Backups are off.
saved = Saved.
language-changed = Done. Log out and back in to see Iliada in { $language }.

## Updates
update-check-failed = Could not check for updates: { $error }
up-to-date-checked = Up to date (checked { $ago }).
updates-daily = Iliada looks for updates every day.
update-now = Update now
check-now = Check now
iliada-version = Iliada { $version }
test-updates = Test updates
test-updates-desc = Get Iliada's newest work before it is released. Fixes arrive sooner, and so do new problems.
apps = Apps
app-updates = App updates
app-updates-desc = Apps from agora update automatically, every few hours.
update-apps-now = Update apps now

## Backup
backup-intro = Your apps and settings, kept in a private repository on your GitHub account. On a new computer, restore them in a few clicks.
backup-unavailable = Backup is not available
backup-unavailable-desc = This system has no GitHub app configured for backups (iliada.system.githubClientId).
github-enter-code = Open GitHub and enter this code:
open-github = Open GitHub
github-waiting = { $url } · Waiting for you to approve it…
back-up-to-github = Back up to GitHub
sign-in-github = Sign in with GitHub
back-up = Back up
name-this-backup = Name this backup
signed-in-as = Signed in as { $account }
last-backup-failed = The last backup failed: { $error }
last-backup = Last backup: { $ago }.
not-backed-up = Not backed up yet.
backing-up-to = Backing up to github.com/{ $repo }
backup-schedule = { $last } Iliada backs up after every change, and every hour.
back-up-now = Back up now
restore-from-backup = Restore from a backup
restore-from-backup-desc = Bring back the apps and settings of another computer.
show-backups = Show backups
backups-list-failed = Could not list your backups
no-backups = No backups yet
backup-item = github.com/{ $repo } · last backup { $date }

## Command-line tools
installed-tools = Installed tools
no-tools = No tools installed
tool-placeholder = Package name, e.g. ripgrep
install = Install
add-a-tool = Add a tool
tools-desc = Programs for the Terminal, like vim or git, installed for everyone on this computer. Apps with windows come from agora.

## Services
start-automatically = Start automatically
starts-at-boot = Starts every time the computer starts.
starts-when-asked = Installed, but starts only when asked (systemctl start { $unit }).
service-ssh = SSH server (remote login)
service-printing = Printing (CUPS)
service-nginx = nginx web server (/srv/www)
service-postgresql = PostgreSQL database
service-mariadb = MariaDB database
service-redis = Redis
service-docker = Docker

## Privacy
privacy-intro-title = What apps may use
privacy-intro = Every app runs in its own sandbox and sees only its own files. Apps ask the first time they need a folder, the camera or the microphone. Changes apply the next time the app opens. All of this is in your permissions file too, where you, Homer and other tools can read it.
no-apps = No apps are installed yet
not-asked = Not asked yet
extra-isolation = Extra isolation
extra-isolation-desc = Run it in its own virtual machine: stronger isolation, slower start and graphics, no camera yet.

privacy-by-app = By app
privacy-by-permission = By permission
open-the-file = Open the file
permissions-file-mistake = Your permissions file has a mistake
until-fixed-none = Until it is fixed, apps get none of what you decide here
reset = Reset
can-touch = Can touch
worst-case = Worst case
not-restricted-yet = Not restricted yet
not-restricted-yet-list = Iliada cannot limit these yet: { $list }.
not-sandboxed = Not sandboxed
not-sandboxed-list = Built into Iliada or installed as command-line tools, these can do whatever you can: { $list }. Homer may use one only if you allow it below.
cap-not-restricted-yet = { $name }: not restricted yet
every-app-why = Every app: { $why }.
cap-never = { $name }: never
no-app-why = No app: { $why }.
folder-read-write = Read and change
folder-read-only = Read only
folder-not-allowed = Not allowed
folder-ask-next = Ask next time
asks-next-time = Asks the next time it opens
reset-permissions-q = Reset your permissions file?
reset-permissions-body = Your answers are forgotten and every app asks again the next time it opens. The file with the mistake is kept next to it, as permissions.toml.broken.
permissions-reset = Every app asks again the next time it opens.
permissions-reset-failed = Could not reset: { $error }

clipboard-history = Clipboard history
clipboard-history-desc = Super+V shows the last 200 things you copied: text, images and files. They stay on this computer, encrypted. Passwords marked secret are never saved, and sandboxed apps cannot read the history.
clipboard-history-on = Super+V shows what you have copied.
clipboard-history-off = Super+V will not keep what you copy.
clipboard-history-clear = Clear clipboard history
clipboard-history-clear-desc = Deletes every saved copy, including pinned ones.
clipboard-history-clear-ellipsis = Clear History…
clipboard-history-cleared = Clipboard history is empty.
clipboard-history-clear-failed = Couldn't clear clipboard history: { $error }
clipboard-history-setting-failed = Couldn't change clipboard history: { $error }
clipboard-exclude = Don't save copies from these apps
clipboard-exclude-desc = Useful for password managers and banking apps. Copies marked secret are always skipped, even from apps that are not listed.

## Homer

history = History
keep-conversations = Keep conversations
keep-conversations-on = Homer keeps your chats on this computer, so you can go back to them: Terminal ones by folder. Quick tasks like "Switch to dark mode" are kept for a day. Only you can read them.
keep-conversations-off = Nothing new is kept: each conversation is gone when you close it.
clear-history = Clear history
clear-history-desc = Deletes every conversation Homer kept. What he did stays in Activity.
clear-history-ellipsis = Clear History…
delete-all = Delete All
history-setting-failed = Couldn't change Homer's history setting: { $error }
history-already-empty = There were no conversations to delete.
history-cleared = Deleted { $n } conversations.
history-clear-failed = Couldn't clear Homer's history: { $error }
use-homer = Use Homer
use-homer-desc = Homer, Iliada's assistant, does things for you when you ask in plain words. It uses Claude (Anthropic) or a model on OpenRouter, with your own API key.
use-other-apps = Use other apps
use-other-apps-desc = Homer can read and use the apps you have open, as you would: press buttons, type, and, in his own Chrome window, work on the web and in Google Docs. A banner says when he is in control, and Super+Escape or Stop ends it. He asks before sending, deleting or paying. Each app is also listed in Privacy.
use-other-apps-off-by-admin = An administrator has turned this off on this computer.
not-sandboxed-homer = Homer may still use these, if you allow it
what-it-may-do = What it may do
ask-me-first = Ask me first
ask-me-first-desc = It shows every action and does it only if you allow it.
act-on-its-own = Act on its own
act-on-its-own-desc = It installs, removes and changes things without asking. Important things done in your name, like sending a message, still wait for your yes unless you allow them below.
kind-without-asking = { $what } without asking
kind-messages = Send messages and emails
kind-allowed = Homer sends them on his own. Every one is listed in Homer's Activity.
kind-asks = Homer shows you each one first and waits for your yes.
model-more-mistakes = { $model } may make more mistakes on its own
model-careful = Opus, Fable and Sonnet are the most careful. Every action is recorded either way.
full-access = Full access
full-access-desc = It may also run commands as you: read and change your files, change desktop settings, anything you can do without an administrator's password.
model = Model
model-desc = Opus is the default: careful and reliable. DeepSeek V4 Flash (through OpenRouter) costs a fraction of a cent per request.
voice = Voice
homers-voice = Homer's voice
homers-voice-desc = How he talks: plainly, with a light Homeric touch now and then ("Sing, O Muse…"). It is a text file you can change; delete it to go back to this.
api-key = API key
checking-key = Checking the key…
key-does-not-work = This key does not work: { $error }.
key-private = Only you can read it; it is never backed up.
replace = Replace
key-saved = { $provider } key saved ({ $masked })
key-create-at = Create one at { $site }, then paste it here. Only you can read it; it is never backed up.
key-wrong-provider = That is an { $other } key, but the model above needs an { $provider } key ({ $prefix }…). Choose another model, or paste an { $provider } key.
key-starts-with = { $provider } keys start with { $prefix }
key-looks-right = Looks right. Press Add key.
add-key = Add key
replace-key = Replace your { $provider } key
add-api-key = Add your { $provider } API key
spent-so-far = Spent so far: { $amount }
email = Email
homer-sends-as = Homer sends email as { $address }
let-homer-email = Let Homer send email for you
email-desc = From your own email account. You see every email before it is sent. For Gmail, iCloud and Yahoo use an app password from your account's security settings.
email-address = Email address
outgoing-server = Outgoing server
connection = Connection
security-tls = SSL/TLS (port 465)
security-starttls = STARTTLS (port 587)
password = Password
password-saved = Saved (type to change)
email-password-first = Type the email account's password first.
email-save-failed = Could not save the email account: { $error }
email-saved = Homer can now send email as { $address }. Only you can read the password.
email-remove-failed = Could not remove the email account: { $error }
email-removed = The email account was removed.
key-save-failed = Could not save the key: { $error }
key-added = { $provider } key added.
key-remove-failed = Could not remove the key: { $error }
key-removed = { $provider } key removed.
save-failed = Could not save: { $error }
voice-file-failed = Couldn't make Homer's voice file: { $error }
no-home-for-voice = There is no home folder to keep Homer's voice in.

## About
this-computer = This computer
name = Name
configuration = Configuration
configuration-value = /etc/iliada (every change is recorded)
report-desc = If something doesn't work (screen, Wi-Fi, sound, sleep), save a report on this computer's hardware and share it with whoever helps.
save-report = Save report
looking-at-computer = Looking at this computer…
report-saved = Saved as { $path }. It is plain text: read it before you share it.
report-failed = The report could not be saved: { $error }
help = Help
hardware-report = Hardware report

## Language
language-title = Iliada's language
language-desc = Menus, apps and the login screen, for everyone on this computer. It changes the next time you log in.
language-pending = Log out and back in to see Iliada in { $language }.
log-out = Log out
keyboard-layouts = Keyboard layouts
keyboard-layouts-desc = English (US) and Greek. Press Super+Space to switch between them.
keyboard-settings = Keyboard settings

## Dialogs
restore-backup-q = Restore this backup?
restore-backup-body = The apps and settings from github.com/{ $repo } replace this computer's. From now on, this computer backs up there.
remove-key-q = Remove the API key?
remove-key-body = Homer stops working until you add a { $provider } key again.
restore-settings-q = Restore your settings?
restore-settings-body = Your desktop settings, Privacy answers and conversations with Homer from { $from } replace this computer's.
account-sign-out-q = Sign out of your Iliada account?
account-sign-out-body = Backups stop on this computer, and it forgets your account key: signing in again needs your recovery key. Your account and its backup are kept.
testing-q = Get test updates?
testing-body = This computer gets Iliada's newest work as soon as it is ready, before it is released to everyone: fixes arrive sooner, and so do new problems. If an update causes trouble, restart and choose the previous version in the start-up menu.
get-test-updates = Get test updates
github-sign-out-q = Sign out of GitHub?
github-sign-out-body = Backups stop. Your backups on GitHub are kept.

## Status
settings-unavailable = Settings are not available
admin-only = Only an administrator can change these settings.
iliada-busy = Iliada is busy: { $what }…

## Iliada account
account-no-home = There is no home folder to keep the account in.
account-busy-checking-key = Checking the recovery key
account-busy-asking = Asking
account-busy-handing-over = Handing over the key
account-busy-restoring = Restoring
account-busy-asking-code = Asking for a code
account-can-read-backup = This computer can read your backup now.
account-key-handed = “{ $computer }” has your account key now.
account-backup-failed = The backup didn't work: { $error }
account-settings-back = Your settings from { $computer } are back on this computer.
account-apps-installing = The apps and tools from { $computer } are installing.
account-apps-failed = The apps and tools didn't come back: { $error }
account-restore-failed = Couldn't restore: { $error }
account-signed-out = Signed out. Your account and its backup are still there.
account-coming-soon = Coming soon
account-coming-soon-desc = Iliada accounts will remember your settings and bring them to a new computer. They're not ready yet. Until then, you can back up to GitHub in Settings → Backup.
account-sign-in-title = Sign in to your Iliada account
account-sign-in-desc = Your desktop settings, Privacy answers and conversations with Homer are backed up, encrypted on this computer first so only you can read them. On a new computer, sign in and they come back. New here? Signing in makes your account.
sign-in = Sign in
account-key-request-title = A new computer asks for your account key
refuse = Refuse
approve = Approve
account-wants-backup = “{ $computer }” wants to read your backup
account-approve-only-if = Approve only if that computer shows the code { $code } and you are setting it up yourself.
account-last-backup-failed = The last backup didn't work: { $error }
account-backup-schedule = { $last } Iliada backs up every hour while you're logged in, encrypted on this computer first.
recovery-key = Recovery key
recovery-key-desc = A new computer needs it once to read your backup.
show = Show
whats-backed-up = What's backed up
part-desktop = Desktop: appearance, dock and panel, wallpaper, shortcuts
part-privacy = Privacy: what each app may use
part-conversations = Assistant conversations
looking-for-backup = Looking for your backup…
backup-look-failed = Couldn't look for your backup
account-no-backup = Your account has no backup yet
restore-ellipsis = Restore…
get-from-other-computer = Get it from another computer
get-from-other-computer-desc = Your backup is encrypted with a key only your computers have. If one of them is signed in, it can hand the key to this one.
ask-other-computer = Ask another computer
type-recovery-key = Or type your recovery key
type-recovery-key-desc = The one you saved when you made your account.
sign-in-page-code = Sign in on your Iliada account page, then approve this code:
open-sign-in-page = Open the sign-in page
waiting-approval = { $url } · Waiting for you to approve it…
recovery-key-new = Your backup is encrypted on this computer with a key that only your computers have, so nobody else can read it, Iliada included. Write this recovery key down or print it, and keep it somewhere safe: a new computer needs it once. If you lose every computer and this key, the backup can't be read.
recovery-key-again = A new computer needs this once to read your backup. Keep it somewhere safe, away from this computer.
saved-it = I've saved it
copy = Copy
your-recovery-key = Your recovery key
backup-from = Backup from { $computer }
backup-made = Made { $ago }. Restoring puts its settings on this computer; the desktop changes right away.
key-wait = On a computer already signed in to your account, open Settings → Iliada account. Approve this computer there if it shows this code:
waiting-other-computer = Waiting for the other computer…

## Setup Assistant
setup-welcome = Welcome
setup-welcome-title = Welcome to Iliada
setup-welcome-body = Apps are in Applications, each in its own sandbox. Get more from agora; install one by dragging it into Applications, remove it by dragging it to the Trash.
setup-language = Language
not-now = Not now
setup-account-soon = Iliada accounts are coming soon. They'll remember your settings and bring them to a new computer. Until then, you can back up to GitHub in a moment.
setup-account-body = Sign in to keep your desktop settings, Privacy answers and conversations with Homer backed up, encrypted so only you can read them. New here? Signing in makes your account.
setup-account-title = Your Iliada account
setup-used-before = Have you used Iliada before?
setup-used-before-body = If you backed up another computer to GitHub, bring its apps and settings here.
set-up-as-new = Set up as new
enter-code-github = Enter this code on GitHub:
waiting-for-approval = Waiting for you to approve it…
sign-in-to-github = Sign in to GitHub
sign-in-to-github-body = Iliada keeps your backup in a private repository on your account.
backups-list-failed-error = Could not list your backups: { $error }
no-iliada-backups = There are no Iliada backups on this account yet.
last-backup-date = Last backup { $date }
set-up-as-new-instead = Set up as new instead
choose-backup = Choose a backup
choose-backup-body = Its apps and settings replace this computer's.
looking-in-agora = Looking in agora…
agora-unreachable = agora can't be reached right now (is the internet connected?). Get apps from agora in the dock any time.
browser = Browser
nothing-to-download = Nothing to download.
to-download = { $size } to download, while you carry on.
install-and-continue = Install and continue
choose-apps = Choose your apps
choose-apps-body = Pick what you'd like to start with. Each app comes sandboxed and sees only what you allow. agora has more.
installed = Installed
back-up-this-computer = Back up this computer
back-up-this-computer-body = Iliada can keep your apps and settings in a private GitHub repository, updated after every change. A new computer is then ready in minutes.
few-minutes = This can take a few minutes.
last-backup-ago = Last backup { $ago }
change-in-settings = Change these any time in Settings.
start-using = Start using Iliada
all-set = You're all set
all-set-body = Find apps in Applications and agora, both in the dock.
summary-account = Your settings are backed up to your Iliada account, encrypted.
summary-restored = Restored your apps and settings from { $name }.
restoring-apps-settings = Restoring your apps and settings
summary-backed-up = Your apps and settings are backed up to iliada_{ $name } on GitHub.
setting-up-backup = Setting up your backup
install-start-failed = Could not start installing the apps: { $error }
summary-downloading =
    { $count ->
        [one] { $apps } is downloading. It appears in Applications when it's ready.
       *[other] { $apps } are downloading. Each one appears in Applications when it's ready.
    }
and-list = { $rest } and { $last }
size-gb = About { $size } GB
size-mb = About { $size } MB
pick-firefox = Fast, private browsing
pick-chrome = Google's browser
pick-chromium = Chrome without Google's extras
pick-writer = Documents, like Word
pick-calc = Spreadsheets, like Excel
pick-impress = Slides, like PowerPoint
pick-thunderbird = Email and calendar
pick-vlc = Plays any video or music
pick-loupe = Another photo viewer
pick-evince = Read and fill in PDFs
pick-gimp = Edit photos
pick-inkscape = Drawings and logos
pick-audacity = Record and edit sound
pick-clocks = Alarms, timers, world clocks
pick-file-roller = Open and make zip files
pick-transmission = Download torrents
pick-mines = A quick game
pick-sudoku = A number puzzle

## Homer
homer-welcome = What can I do for you?

## Permission prompt
prompt-title = “{ $app }” would like to { $wants }.
prompt-later = You can change this later in Settings → Privacy.
dont-allow = Don't Allow
allow = Allow
ok = OK

## What an app asks for, after “would like to”
wants-camera = use your camera
wants-microphone = use your microphone
wants-location-coarse = know your approximate location
wants-location-precise = know your precise location
wants-clipboard-read = read your clipboard
wants-clipboard-write = write to your clipboard
wants-file = open a file you choose
wants-folder = access your { $folder } folder
wants-notifications = show notifications
wants-screen = record your screen
wants-sound = play sound
