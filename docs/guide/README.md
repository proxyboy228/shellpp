# Shell++ user guide

English | [Русский](ru/README.md)

Shell++ is an SSH client that keeps your data on your own devices: hosts,
keys, passwords and snippets live in an encrypted vault, and sync between
devices runs through a server you host yourself, if you want it at all.

1. [First run and the master password](01-first-run.md): creating the
   vault, unlocking and locking, a forgotten password.
2. [Hosts, groups and tags](02-hosts.md): adding hosts, groups and tags,
   quick connect, importing from `~/.ssh/config`, PuTTY and CSV, known
   hosts.
3. [Keys, the Keychain and Export to host](03-keys.md): generating and
   importing keys, FIDO2 security keys, putting a key on servers, agent
   forwarding.
4. [The terminal](04-terminal.md): tabs, splits, workspaces, broadcast
   input, the terminal sidebar, shortcuts.
5. [SFTP, port forwarding and snippets](05-sftp-forwarding-snippets.md),
   and the connection log.
6. [Sync between your devices](06-sync.md): installing the server in a
   homelab, three ways to reach it, a second device, Recently deleted,
   backups.
7. [Exporting your data and moving from another client](07-export-and-migration.md).

## Conventions

- The interface is in English; the guide quotes its labels in **bold**.
- Keyboard shortcuts are the Windows ones. A Mac uses Cmd and a few
  other keys; Settings > **Shortcuts** lists the app's and the terminal's
  shortcuts, and they can be changed there.
- "The app menu" is the button with three bars at the left of the top bar;
  "Settings" opens with Ctrl+, or from the app menu.
- The screenshots show demo data (made-up hosts and keys).

## Getting Shell++

Shell++ 1.0 is for Windows and Linux; the releases are on the project's
[Releases](https://github.com/proxyboy228/shellpp/releases) page, each with
`SHA256SUMS` and a minisign signature of every file
([docs/release.md](../release.md#checking-a-download) shows how to check a
download).

- **Windows:** `shellpp-<version>-windows-x64.zip` is a portable app: unpack
  it and run `shellpp.exe`, nothing is installed. The executable is not
  code-signed, so SmartScreen may warn the first time: **More info**, then
  **Run anyway**.
- **Linux:** `shellpp-<version>-linux-x64.tar.gz` holds the program, a menu
  entry and its icon; the [README](../../README.md#linux) lists what it needs
  (glibc 2.35 or later, a Vulkan driver, a Secret Service provider for
  "Remember on this device") and the three commands that install it for your
  user. Linux is checked in containers, not yet on a real desktop.
- **Updates:** Help > **Check for Updates** tells you about a new release and
  opens its page; it never downloads or installs anything. A release
  candidate (`1.0.0-rc.1`) sees no update until 1.0.0 is published.

You can also build it from source, as the
[README](../../README.md#building-from-source) describes.
