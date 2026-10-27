# Changelog

What each release of Shell++ holds. The desktop app and the sync server
share one version. Each section below is the text of its release's notes:
the release workflow takes the section whose heading names the tag's
version (`docs/release.md`). The Android app has versions of its own
([docs/mobile/](docs/mobile/README.md)).

## [1.0.0-rc.1]

The release candidate of Shell++ 1.0: the first release, for Windows and
Linux. It becomes 1.0.0 once it has been checked on a real installation
with a sync server and two devices.

### Desktop app

- **Hosts and groups:** cards and a list, tags, colors, search, quick
  connect by typing `ssh user@host`, sorting, multi-select, Recently deleted
  (90 days).
- **Keychain:** SSH keys generated in the app or imported (OpenSSH and PuTTY
  formats, certificates), identities, FIDO2 security keys on Windows
  (signing and creating keys through the system's authenticator), a
  built-in SSH agent, agent forwarding (off by default per host) and Known
  Hosts with the host key dialogs.
- **Terminal:** SSH sessions, a local terminal, Telnet and serial
  connections; tabs, split panes, workspaces and broadcast input; jump
  hosts and SOCKS or HTTP proxies; highlighting rules; autocomplete from the
  host's history (off by default); connection logs and encrypted session
  recordings; command history with a secret filter.
- **SFTP:** a local and a remote panel, a transfer queue, folder transfers,
  opening a remote file in a local program.
- **Port forwarding:** local, remote and dynamic (SOCKS) rules, started by
  hand, with a session to their host, or after unlock.
- **Snippets:** saved commands, scripts on many hosts at once, and optional
  script generation through an OpenAI-compatible API or a local model that
  you set up (off by default).
- **Import and export:** import from `~/.ssh/config`, `known_hosts`, PuTTY
  sessions, CSV and Shell++ JSON; export to JSON (plain or encrypted with a
  password), `~/.ssh/config`, OpenSSH keys and `known_hosts`.
- **Security:** everything stored is encrypted with a key that your master
  password or recovery key opens (Argon2id, XChaCha20-Poly1305);
  auto-lock; "Remember on this device" keeps the device key in the OS
  keychain; secrets never reach the logs.
- **Settings:** themes (light, dark, system), interface scale, terminal
  themes and fonts, shortcuts that can be changed.
- **Updates:** Help > Check for Updates shows a new release only when its
  manifest is signed with the release key built into the app, and opens its
  page; nothing is downloaded or installed. A release candidate is never
  offered to a stable version, and a release candidate sees no update until
  1.0.0 is published.

### Sync

- Optional sync of the personal vault between devices through a server you
  host yourself; end-to-end encrypted, the server sees only encrypted
  records and sync metadata.
- Registration and sign-in with a merge of what the device already has,
  conflict copies instead of lost edits, Recently deleted across devices, a
  recovery key shown at registration and reissued on demand, password
  changes and "Lost or stolen" device revocation, optional TOTP.
- Certificates: the system's roots, or for one server a pinned certificate
  or a CA you add.

### Sync server

- Rust (axum) with SQLite or PostgreSQL; invite-only registration by
  default; sign-in limits; a Docker image and a static Linux binary.
- Docker Compose deployment with an encrypted backup and a rotation that
  keeps good backups; three ways of access: your own domain with Caddy and
  Let's Encrypt, Cloudflare Tunnel, or only over a VPN
  (`server/README.md`).

### Packages

- `shellpp-1.0.0-rc.1-windows-x64.zip`: the portable app for Windows (not
  code-signed).
- `shellpp-1.0.0-rc.1-linux-x64.tar.gz`: the app for Linux (x86-64, glibc
  2.35 or later) with a menu entry and its icon.
- `shellpp-server-1.0.0-rc.1-docker-image.tar.gz` and
  `shellpp-server-1.0.0-rc.1-linux-x64.tar.gz`: the sync server.
- `SHA256SUMS` and a minisign signature of every file; `latest.json`, the
  signed update manifest.

### Known limitations

- No macOS build, no team vaults, no Mosh; the archive of a previous
  account, changing the account's email and publishing the SSH ID again
  come later.
- Linux is checked in containers, not yet on a real desktop; FIDO2 security
  keys do not sign on Linux in this release.
- The rest is in the README, "Known limitations", and in
  `docs/security.md`, section 17.
