# The Iliada installer (English).

install-iliada = Install Iliada
language = Language
welcome-body = Iliada will be installed on this computer's disk. You can also keep trying it: nothing is changed until you confirm, a few steps from now.
keep-trying = Keep trying Iliada
continue = Continue
back = Back

## Disk
disk = Disk
removable = (removable)
other-files = other files
the-other-system = the other system
disks-failed = Could not list the disks: { $error }
no-disk = There is no disk of at least 20 GB to install on. Connect one, then go back and try again.
next-to =
    { $kind ->
       *[system] Next to { $what }, in its { $free } of free space
    }
erase-whole =
    { $kind ->
       *[system] Erase { $what } and use the whole disk
    }
needs-free =
    { $kind ->
       *[system] To keep { $what }, Iliada needs { $need } of free space next to it; this disk has { $free }.
    }
whole-empty = Use the whole disk (it is empty)
where-title = Where should Iliada go?
where-body = Iliada can go next to what is already on a disk, if there is room, or replace it.

## About you
your-name = Your name
account-name = Account name
computer-name = Computer name
password = Password
repeat-password = Repeat the password
time-zone = Time zone
tz-found = Found from your internet connection.
hint-user = The account name can use lowercase letters, digits, - and _.
hint-hostname = The computer name can use letters, digits and -.
hint-passwords = The passwords don't match.
about-you = About you
about-you-body = Your account is an administrator: it can install apps and change settings.

## Time zone regions
region-africa = Africa
region-america = America
region-antarctica = Antarctica
region-arctic = Arctic
region-asia = Asia
region-atlantic = Atlantic
region-australia = Australia
region-europe = Europe
region-indian = Indian
region-pacific = Pacific
region-other = Other

## Confirm
confirm-keep-title =
    { $kind ->
       *[system] Install Iliada next to { $what }?
    }
confirm-keep-body =
    { $kind ->
       *[system] { $what } stays as it is. When the computer starts, Iliada starts; to start { $what } instead, open the start-up menu (usually F9, F12 or Esc while the computer starts).
    }
install = Install
confirm-erase-title = Erase the disk and install Iliada?
confirm-erase-body = Everything on this disk will be erased. This can't be undone.
erase-and-install = Erase and install
confirm-summary = { $name } ({ $user }) on “{ $hostname }”, time zone { $timezone }, in { $language }.

## Installing
installing-title = Installing Iliada
installing-body = This takes a while: Iliada is downloaded and set up for this computer. Keep it connected to the internet and to power.
stage-preparing = Preparing
stage-room = Making room on the disk (everything on it stays)
stage-partitioning = Partitioning the disk
stage-mounting = Mounting
stage-configuration = Writing the configuration
stage-installing = Installing Iliada (this downloads and builds the system)
stage-password = Setting the password
stage-done = Done

## Done
installed-title = Iliada is installed
installed-body = Remove the installation USB stick (or disc), then restart. Log in with the password you chose.
restart-now = Restart now
failed-title = The installation did not finish
failed-body = Nothing on this computer is usable yet. Check the internet connection and try again.
quit = Quit
try-again = Try again
start-failed = could not start the installer: { $error }
install-failed = the installation failed
