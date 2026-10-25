# Design decisions (stage 0)

This is the public record of the decisions taken before stage 0 (see `SPEC.md`, "Work order"). The numbering follows the list of 26 open questions raised against the specifications. All 26 proposals were accepted as written, with no owner overrides.

- Status: accepted, 2026-10-18.
- Decisions 27 and later were taken after stage 2. Each states its own date and the earlier decisions it supersedes.
- Each entry has three parts: the context (what was unclear), the decision, and the spec impact (which specification is updated to match).
- Decisions that set UI defaults are marked [decision] in `docs/ui-behavior-spec.md` and can be changed later through the normal specification change process.
- Technical details are in `docs/architecture.md`, `docs/data-model.md`, `docs/storage-format.md`, `docs/sync-protocol.md` and `docs/security.md`.

## Stage 0 and work order

### 1. Stage 0 deliverables and their location

**Context.** `SPEC.md`, "Work order", asks for a tokens file and base components in stage 0, but the monorepo skeleton and the Tauri app only appear in stage 1, while `docs/design-system.md` §7 places the tokens in `apps/desktop/src/theme/tokens.css`. The list of design documents differs between "Work order" and "Default stack", and "tests are green" had no meaning without code.

**Decision:**

- In stage 0, `apps/desktop` is created as a frontend-only package (pnpm, Vite, React, TypeScript) without `src-tauri`. Stage 1 initializes Tauri around the same directory.
- The package contains `src/theme/tokens.css`, `src/theme/terminal/*.json` and `src/components/`: Button (primary, secondary, danger, text), SplitButton, IconButton, TextInput, PasswordInput, Toggle, Chip, host card, Menu, Modal, StatusDot, SegmentedControl.
- The components are shown on one showcase page with a Light / Dark / Auto switch.
- "Tests are green" in stage 0 means: the contrast test and the Vitest render tests pass, and `tsc` and eslint report no errors. A `ci.yml` workflow with one Linux job runs them on GitHub.
- Five design documents: `docs/architecture.md`, `docs/data-model.md`, `docs/storage-format.md`, `docs/sync-protocol.md` and `docs/security.md` (threat model). SQL migrations come in stage 1 and the export JSON Schema in stage 3.

**Spec impact:** `SPEC.md`, "Work order" (stage 0) and "Default stack" (list of documents).

### 2. Technical identifiers

**Context.** The project is named Shell++ with the slug `shellpp`, but `+` is not allowed in crate and package names, bundle identifiers or paths. The keychain service, data directory, bundle identifier, URL scheme and export format id are hard to change once users have data.

**Decision:**

- The display name is "Shell++". Every technical identifier uses `shellpp`.
- Crates `shellpp-core` and `shellpp-ssh`, npm package `@shellpp/desktop`.
- URL scheme `shellpp://`, data directory `shellpp`, OS keychain service `shellpp`.
- Bundle identifier `io.github.proxyboy228.shellpp`.
- Export format id `shellpp.export.v1`.
- All identifiers are defined once as constants that code and documents refer to. The name of a local working-copy directory is never used in code, identifiers or UI.

**Spec impact:** none. The identifiers are recorded in `docs/architecture.md`.

### 3. Features that do not match the stages

**Context.** UI sections assigned to early stages contain elements of later stages, for example the vault switcher, "Collaborate" and "Copy to" (stage 8), "Add to Workspace" (stage 9), "Import" (stage 3) and "Host Chaining", "Proxy" and "Startup snippet" (stage 5). Several `SPEC.md` features had no stage, and stage 2 needs key authentication before the Keychain screen exists (stage 3).

**Decision:**

- Elements of future stages are hidden behind a stage flag, not shown disabled. Their e2e tests are written in the stage that implements them, and every stage report lists what is still hidden.
- Stage 2: local terminal, system ssh-agent / Pageant, agent forwarding, OS detection with the default script. The host panel gets a short path "+ → Key → paste or choose a file" with a passphrase prompt. Autoreconnect, keepalive and TERM are stored without a settings screen.
- Stage 3: the import dialog with all formats from `SPEC.md`, the built-in agent.
- Stage 6: keyword highlighting.
- Stage 9: Telnet and serial connections.
- Mosh gets its own later stage; its toggle stays hidden until then.

**Spec impact:** `SPEC.md`, "Work order" (stage assignments and the hidden-feature rule).

## Stack

### 4. License allowlist and the chosen stack

**Context.** `SPEC.md`, "Clean room", allows MIT, Apache-2.0, BSD, ISC, MPL-2.0 and OFL. Read literally, this excludes SQLite (public domain), common transitive Rust licenses and LGPL system libraries inside an AppImage. russh defaults to the aws-lc-rs backend, and the `rsa` crate it enables has the open advisory RUSTSEC-2023-0071.

**Decision:**

- The allowlist is extended with Zlib, BSL-1.0, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, CDLA-Permissive-2.0 and public domain (SQLite).
- System components that the project does not ship (WebView2, WebKitGTK, GTK, glibc) are platform dependencies. LGPL libraries bundled inside an AppImage are listed in `THIRD_PARTY_NOTICES.md`. GPL and AGPL code is forbidden in the client distribution.
- russh uses the `ring` backend (simpler mobile builds) as an explicit exception to "RustCrypto, dalek". cargo-deny bans the `fips` feature, ring older than 0.17.10 and aws-lc-sys older than 0.39.
- keyring is built without its dbus features (libdbus is AFL-2.1 / GPL-2.0).
- `rsa` is a documented exception: signing happens locally and the risk is low. Upstream is tracked.
- CI checks licenses with cargo-deny and license-checker. devDependencies that are never shipped have a separate list that also allows BlueOak-1.0.0 and CC-BY-4.0.

**Spec impact:** `SPEC.md`, "Clean room" (allowlist); `CONTRIBUTING.md`, "Dependencies"; the rules in `THIRD_PARTY_NOTICES.md`.

### 5. Stack extensions for uncovered features

**Context.** The default stack (`SPEC.md`, "Default stack") has nothing for a local PTY, serial ports, Telnet, FIDO2, PEM and PuTTY `.ppk` import or desktop biometrics. The monorepo has no place for PTY and serial code, keyring does not support Android, Linux may have no Secret Service, and "built-in agent" was undefined.

**Decision:**

- The stack is extended, not replaced, with permissive crates recorded in `docs/architecture.md`: portable-pty, serialport, a minimal in-house Telnet implementation, and PEM and `.ppk` import built on RustCrypto crates.
- A new desktop-only crate `crates/term-local` holds PTY and serial support.
- `crates/core` accesses secrets through a trait; keyring is only an implementation in `apps/desktop`. Without Secret Service, features that need the OS keychain are disabled. There is no fallback to a file.
- In the MVP the built-in agent works only inside the app: keys from the Keychain, plus proxying of the system agent for agent forwarding.
- FIDO2: a short spike before stage 3. If it fails, sk keys work only through the system agent.
- Biometrics: a separate decision after stage 6, for Windows and macOS only.

**Spec impact:** `SPEC.md`, "Default stack" (the monorepo gains `crates/term-local`).

### 6. E2E acceptance across platforms

**Context.** `SPEC.md`, "UI/UX", requires a WebdriverIO + tauri-driver e2e test for every screen and scenario. tauri-driver has no macOS support, WebDriver cannot see native UI, GitHub Windows runners cannot run Linux containers, and macOS runners have no Docker.

**Decision:**

- e2e tests run on Linux (xvfb, WebKitWebDriver) and Windows (msedgedriver matching the WebView2 version). A screen is done when its tests pass on these platforms.
- macOS: build, unit tests, Rust integration tests and a manual checklist.
- In e2e builds, native dialogs, the clipboard and launching external programs are replaced by mocks behind a cargo feature. The mocks never reach release builds.
- OpenSSH in Docker runs only in the Linux job. For Windows e2e, the repository contains a test SSH/SFTP server built on russh.

**Spec impact:** `SPEC.md`, "UI/UX" (acceptance) and "Quality".

### 7. Auto-update, packaging and signing

**Context.** Principle 2 forbids phone-home, while stage 9 asks for an auto-update from the owner's own source that can be turned off. The default, the settings location and the package formats were not set, although `docs/ui-behavior-spec.md` 1.1 already shows an update button.

**Decision:**

- tauri-plugin-updater with the project's own signing key.
- The automatic check is off by default. A manual "Check for updates" item in Help is always available.
- The check is a plain GET of the manifest with no identifiers.
- The default source is the repository's releases. It can be overridden in a new Settings → Updates section.
- Packages: per-user NSIS installer on Windows, `.dmg` on macOS, AppImage (with updater) and `.deb` on Linux.
- Buying Apple and Windows signing certificates is the owner's decision. Unsigned macOS builds disable biometrics and "remember on this device".

**Spec impact:** `docs/ui-behavior-spec.md`, 12.1 and a new Updates section (12.7), 1.1 (update button) and 1.4 (Help); `SPEC.md`, stage 9.

## Security and sync

### 8. Master password and the OS keychain

**Context.** `SPEC.md`, "Default stack", keeps "the key" in the OS keychain, while "Sync and team access" derives a KEK from the master password. A key stored in the keychain can be read by any process of the user, which makes locking pointless. It was not said whether the master password is required without an account.

**Decision:**

- The master password is required from the first launch, even without an account. The same password later becomes the account password.
- Key chain: password → Argon2id → KEK → vault key. The vault key is stored in SQLite, wrapped.
- By default nothing is stored in the OS keychain. Only the "biometrics / remember on this device" option (off by default, with a warning when enabled) puts a device key there, which wraps the vault key a second time.
- The password can be changed in local mode too.
- The model is described in `docs/security.md`.

**Spec impact:** `SPEC.md`, "Default stack" (local storage) and "Security"; `docs/ui-behavior-spec.md`, new "Lock and account" section (13; see 9).

### 9. Lock, first run and account screens

**Context.** Stage 1 includes encrypted storage and a master password, but `docs/ui-behavior-spec.md` has no first-run flow, unlock screen, auto-lock settings or sync server wizard. It also does not say what happens to open sessions and forwards on lock.

**Decision:**

- A new "Lock and account" section, marked [decision], is added to `docs/ui-behavior-spec.md` in stage 0 and shown together with the documents:
  - first run: password and confirmation, a warning that data cannot be recovered without the password and the recovery key, the recovery key with a required "I saved it" checkbox;
  - an unlock screen and a Lock item in the app menu;
  - Settings → Security: auto-lock after 15 minutes (with a "never" option), password change;
  - Connect sync server wizard: URL → sign in or register → TOTP → recovery key.
- On lock, a password screen covers the window and terminal output is hidden. Sessions and forwards keep running; keys are wiped from memory. Reconnects and forward autostart wait for unlock.

**Spec impact:** `docs/ui-behavior-spec.md`, new "Lock and account" section (13), 1.4 (Lock item) and 12.1 (Security section).

### 10. What is encrypted and what the server sees

**Context.** `SPEC.md` says "Secrets are encrypted at the record level" ("Default stack") but also "Each record is encrypted separately" ("Sync and team access"), and `docs/ui-behavior-spec.md` 4.2 says history is stored encrypted "like the rest of the data". Principle 5 allows "sync metadata" without listing it.

**Decision:**

- The entire payload of every record is encrypted. Only the record id, coarse type, vault id, revision, key epoch and sync service fields stay in the clear.
- The local record format is the same as the blob stored on the server.
- Search and sorting run in memory after unlock.
- The server sees: vault id, record id, coarse type, revision, sequence number, tombstone flag, size padded to a multiple of 256 bytes, and the time it received the record. The full list is in `docs/security.md`.

**Spec impact:** `SPEC.md`, "Default stack" (local storage wording).

### 11. Server login, password change and recovery key

**Context.** `SPEC.md`, "Sync and team access", does not say how the client signs in. If the master password or the KEK reached the server, zero-knowledge (principle 5) would be broken. The source of the salt on a new device, the role of the recovery key and the meaning of "master password change (with key re-encryption)" (`docs/ui-behavior-spec.md` 12.2) were open.

**Decision:**

- Argon2id(password, random salt) gives a master key. HKDF derives two independent keys from it: the KEK and the auth key. Only the auth key is sent to the server, which stores a hash of it. TOTP is a second step.
- The salt is random and the KDF parameters carry a version. A pre-login endpoint returns both; for an unknown email it returns a deterministic fake salt.
- The recovery key is 256 random bits, created together with the first password, also without an account. It wraps the personal vault key and yields a recovery auth key that allows setting a new password. TOTP can only be reset with the admin CLI.
- A password change rewraps the vault key and atomically replaces the auth key and the salt. Tokens of the other devices are revoked.
- The risk of old database backups is described in the threat model.
- Amended in stage 0 after security review: the Argon2id salt is SHA-256 over a fixed label, the normalized account email and the random stored salt, so a server that hands one salt to many accounts cannot test a password guess against all of them at once. Because the email is part of the salt, an email change re-derives the keys (new salt, auth key and KEK wrap) and signs out the other devices, and a recovery binds the new salt to the email the server accepted for the recovery.
- Amended in stage 0 after security review: the auth key and the recovery auth key are also bound to the server origin, and registering a device whose data was bound to another account before generates a new salt and a new recovery key, so no server receives values that sign in to or recover an account on another server. The requests that install new key material for an email change or a recovery name the email it was derived for, and the server refuses them when that is no longer the pending or current email; an email change ends open recovery tickets. The client never lowers the personal vault's key epoch: a server that reports an older one is shown as rolled back and changes no local key. A bound device whose server cannot be reached, refuses or rolled back can recover its local copy with the recovery key alone, without a key rotation, and stays signed out of the account until the next sign-in or recovery through the server.

**Spec impact:** `SPEC.md`, "Sync and team access" (key chain).

### 12. Sync conflicts

**Context.** `SPEC.md` requires last-writer-wins per record with the losing version kept for manual resolution, but does not define "last" (server order or client clocks), the role of the revision, where the loser is stored, delete-versus-edit, tombstone retention or the cursor scope. The UI specification had no conflict screen or indicator.

**Decision:**

- A push carries `base_revision`. On a mismatch the server rejects the write and returns the current version.
- The client picks the winner by a hybrid logical clock stored inside the record; ties are broken by device id.
- The losing version is kept as a separate conflict record in the same vault and is visible on all devices. Delete versus edit follows the same rule.
- There is one cursor per vault. Tombstones are kept for 90 days; a device with an older cursor does a full resync.
- UI: a badge on the object, a "Conflicts: N" row in 12.2 and a comparison of the two versions with "Keep this" and "Keep other" buttons [decision].

**Spec impact:** `SPEC.md`, "Sync and team access" → "Protocol"; `docs/ui-behavior-spec.md`, 2.3 (conflict badge), 12.2 and a new conflict comparison dialog (13.6).

### 13. Sharing unit: whole vault or single host

**Context.** `SPEC.md` shares whole vaults only (vault key wrapped per member, roles owner, editor and viewer), while the UI shares single objects ("Collaborate", "Share this host", "Share this snippet") and `docs/ui-behavior-spec.md` 12.3 promises that members connect with their own key. SSH ID had no counterpart in `SPEC.md`, and with end-to-end encryption a viewer must be able to decrypt the passwords it connects with.

**Decision:**

- The only unit of sharing is a vault. "Collaborate" and "Share this host / snippet" open a "move or copy to a team vault" dialog.
- A moved or copied record is re-encrypted and gets a new record id. The dialog lists linked objects and offers to "copy together" (with a warning about secrets) or to "unlink".
- A host in a team vault may have no credentials. Each member attaches their own SSH ID or identity, and that binding is stored in the member's personal vault.
- SSH ID is a separate Ed25519 key in the personal Keychain; only its public part is published to the server. The X25519 key pair is separate; its private part is synced inside the personal vault.
- The server enforces roles on push. Records are not signed in the MVP. The threat model and the invitation dialog say plainly that every member, viewers included, receives the vault's secrets.

**Spec impact:** `docs/ui-behavior-spec.md`, 2.4 (Collaborate, Move to, Copy to), a new "Move or copy to a vault" dialog (2.6), 3.1 (Share this host), 9.3 (Share this snippet) and 12.3; `SPEC.md`, "Sync and team access" (shared vaults).

### 14. Invitations, member removal and protection from the server

**Context.** It was not defined which server attacks the design resists: an active server could swap a member's X25519 key during an invitation, or reorder and roll back blobs. A key inside an invitation link would violate "secrets never end up in URLs", and email invitations need SMTP, which `SPEC.md` does not list. The scope of key rotation, the removed member's view and vaults whose only owner deletes the account were open.

**Decision:**

- Confidentiality against the server is guaranteed. Substituted or reordered blobs are detected through the AEAD associated data (vault id, record id, type, revision, key epoch). Rollback and deletion by the server are not detected, and the threat model says so.
- Members' public keys are pinned on first use (TOFU), and the fingerprint is shown in Team for comparison outside the server. Vault keys are wrapped with HPKE (RFC 9180) in Auth mode.
- Invitations take two steps: the link carries only a one-time id, the owner confirms the new member by fingerprint, and only then does the owner's client wrap the vault key. SMTP is optional: without it only links work and the "verified" mark is hidden.
- Removing a member starts a new key epoch. The owner's client re-encrypts the records in the background, and the dialog suggests changing the host secrets.
- A removed member keeps the vault as a read-only local archive with "Export" and "Copy to personal vault" actions.
- A vault can have several owners. Deleting the account of a vault's only owner is blocked until ownership is transferred.
- Amended in stage 0 after security review: each client keeps its own roster of every team vault with the epoch its account last wrapped to each member, so an automatic rotation runs only when every remaining member is accepted for the current epoch, and a device without an earlier key opens its first vault key only from its own account or from a sender already pinned in the personal vault. Epochs from users the client saw removed are refused, accepting such a sender again makes it a seen member rather than a rotation recipient, and the threat model states that a server can still hide a removal from devices that do not hold the removed roster entry.
- Amended in stage 0 after security review: a roster entry counts for an automatic rotation only when it commits to the key of the current epoch, not just its number, and the member's pin is verified, so a second device kept on a forked view or a withheld pin cannot make an automatic rotation wrap the key to a removed member or to a key the server chose. When two versions of a roster entry conflict, the one that says removed wins. A device without an earlier key opens its first vault key without a fingerprint comparison only from its own account or from a sender whose pin is verified or was made when this account accepted that sender's invitation; a pin made on first sight of a member list, on any device, no longer counts.

**Spec impact:** `SPEC.md`, "Sync and team access" (shared vaults); `docs/ui-behavior-spec.md`, 2.2 (Invite members) and 12.3 (Team).

### 15. Sync scope and the connection log

**Context.** The toggles in `docs/ui-behavior-spec.md` 12.2 cover hosts, keys, snippets, forwarding, history and the log, but not known hosts, workspaces, themes, highlighting rules or settings, and their scope (device or account, push or pull) was not set. 11.1 shows who connected in a team vault, while 11.2 and 12.2 keep the log local and off by default, and viewers cannot write.

**Decision:**

- Sync toggles are set per device and apply to both push and pull. Turning a toggle off does not delete data from the server.
- Known hosts, workspaces, custom themes and highlighting rules sync in the personal vault (on by default). App settings stay local. Team vaults always sync in full.
- The connection log is always written to the personal vault. A team vault shows only the entries of members who turned on publishing; this is an append-only type that viewers can also write. Until stage 8 the User column shows the current device.
- Signing in to an existing account from a device with local data re-encrypts the local records under the account's key and merges them with deduplication, as an import does.
- Amended in stage 0 after security review: signing in again to the account the local personal vault was last bound to is a rebind, not a merge: records keep their ids and the vault runs a full resync. Registering or signing in to another account moves the local data to a new personal vault with new record ids and does not carry over the earlier account's key pair, SSH ID, member pins, rosters and credential bindings, which stay in a read-only archive of that account.

**Spec impact:** `docs/ui-behavior-spec.md`, 11.1, 11.2, 12.2 (Synchronization block) and 12.3 (Publish my connection log); `SPEC.md`, "Sync and team access".

### 16. Command history, session recording and plaintext on disk

**Context.** `docs/ui-behavior-spec.md` 4.2 requires a list of entered commands that skips input typed without echo, but the client sees keystrokes and output, not commands, and "looks like a secret" had no criteria. Output recording is described differently in 3.1, 11.2 and 12.6 (and `SPEC.md`, "Security", points to 12.4 instead of 12.6). Plain export, `.txt` log export and SFTP temporary files conflict with "no plaintext on disk".

**Decision:**

- A command is the logical line in the xterm.js buffer at the moment of Enter, without the prompt. The prompt boundary is the cursor position at the first typed character.
- Nothing is recorded in the alternate screen, while echo is off, or after a line that looks like a password, passphrase, PIN or OTP prompt. OSC 133 is not in the MVP. The documentation describes this as a heuristic with known limits.
- The secret detector matches `-p<value>`, `--password`, `*PASSWORD=`, `TOKEN=`, `SECRET`, `API_KEY`, `user:pass@`, `Authorization:` or `Bearer`, known token prefixes and high-entropy strings of 20 or more characters. Matching commands are not stored and are skipped when shell history is imported.
- Output recording: a global toggle in 12.6 (off by default) and a per-host override under "Show more". Recordings are stored encrypted (chunked stream encryption) and are never synced.
- Explicit user actions (plain export, `.txt` export, SFTP temporary files) are allowed exceptions and show a warning. Temporary files are deleted when the file is closed and when the app exits.

**Spec impact:** `docs/ui-behavior-spec.md`, 3.1 ("Show more"), 4.2, 11.2 and 12.6; `SPEC.md`, "Security" (reference corrected from 12.4 to 12.6).

## UI specification

### 17. Keyboard shortcuts in the terminal and on macOS

**Context.** `docs/ui-behavior-spec.md` 12.5 sends unshifted shortcuts to the shell when the terminal has focus and promises Ctrl+Shift variants, but the table has none for Ctrl+K, Ctrl+L and Ctrl+P. It does not say which other shortcuts the app captures, splits (4.7) have no shortcuts, Ctrl+Alt equals AltGr on Windows, and the Mac set is not described.

**Decision:**

- Ctrl+Shift+K, Ctrl+Shift+L and Ctrl+Shift+P are added and work everywhere.
- Ctrl+letter without Shift goes to the shell when the terminal has focus.
- The app always captures Ctrl+1…9, `Ctrl+,`, all Ctrl+Shift shortcuts, Ctrl+Alt+arrows and Alt+←/→.
- Splits: Ctrl+Shift+D and Ctrl+Shift+E.
- No shortcut fires while AltGr is active.
- The Mac set is the default on macOS: app actions use Cmd, Ctrl+letter always goes to the shell, and Option types characters. The full Mac table is added to 12.5 before stage 6.

**Spec impact:** `docs/ui-behavior-spec.md`, 4.7 and 12.5.

### 18. Connecting, authentication and session end

**Context.** Stage 2 needs password and key authentication, but `docs/ui-behavior-spec.md` describes only TOFU (10.2): there is no connecting state, no handling of initial connection errors and no prompts for passwords, passphrases or keyboard-interactive. 4.6 treats a non-zero exit code as a drop, which with autoreconnect on (12.4) would loop, and the neighbor-tab rule ignores the persistent SFTP tab.

**Decision:**

- While connecting, the tab shows a connection screen: host, current step and a Cancel button.
- Password, passphrase, username and keyboard-interactive prompts are a form inside the tab (a modal dialog when outside a terminal). The "save" checkbox is off by default. keyboard-interactive questions are asked one at a time, masked according to their echo flag.
- A drop is only a transport error. A non-zero exit status shows "Session ended with code N" without reconnecting.
- Autoreconnect is on by default, with delays of 1, 2, 4 … 60 seconds, a countdown and a Cancel button.
- "Neighboring" means the neighboring session tab, and the rule applies to every way of closing a tab. Exit and closing the window while sessions, forwards or transfers are active require confirmation.
- A new split pane shows host selection as on the new tab page (section 6).

**Spec impact:** `docs/ui-behavior-spec.md`, section 4 (new connecting and authentication subsection, 4.8), 4.6, 4.7 and 1.4 (Exit).

### 19. SFTP tabs, transfer queue and settings

**Context.** `docs/ui-behavior-spec.md` 1.1 describes a persistent SFTP tab but also SFTP session tabs, and 2.4 opens a host "in the SFTP manager" without saying where. The transfer queue (5.3), the "ask" dialog for name conflicts and most defaults in 12.6 were missing, and the "default program for Open with" (12.6) contradicts 5.4.

**Decision:**

- "Connect via SFTP" opens a new SFTP session tab with the same dual-pane manager: Local on the left, the host on the right. The persistent SFTP tab stays for manual work.
- The transfer queue is a collapsible bottom panel: name, direction, progress and speed, with Pause, Resume, Cancel and Retry buttons.
- Name conflicts: Replace, Skip and Rename, plus an "apply to all" checkbox.
- Defaults: hidden files not shown, delete confirmation on, name conflicts "ask", 3 parallel transfers, connection log kept for 90 days.
- "Default program for Open with" becomes "Program for Open" (empty means the system association).

**Spec impact:** `docs/ui-behavior-spec.md`, 1.1, 2.4, section 5 (5.1, transfer queue 5.6 and conflict dialog 5.7) and 12.6.

### 20. Host form, groups and autosave

**Context.** `SPEC.md` lists notes, color and keepalive as host fields and requires nested groups with inherited settings, and `docs/design-system.md` 3.1 has 8 tile colors, but the form in `docs/ui-behavior-spec.md` 3.1 has none of these, and there is no group panel or menu. Autosave (3.2) is defined only for hosts, although keys and snippets have required fields.

**Decision:**

- The General block gains Color (8 colors) and Notes.
- "Show more" gains Keepalive (empty means the global value), plus "Record history" and "Record output" (default "as in settings").
- The group panel opens in the details panel: Vault, Name, Parent Group, Color and an SSH block (username, identity or key, port, Host Chaining). Inherited values are shown as a "from group X" placeholder. The group menu has Open, Edit, Move to and Remove. Removing a non-empty group offers to move its content one level up.
- General rule for all panels: an object is created once its required fields are valid. Until then it is a draft that is discarded on close. An invalid value is highlighted and not saved.

**Spec impact:** `docs/ui-behavior-spec.md`, 1.3 (saving rule), 3.1, 3.2 and a new group panel subsection (3.4).

### 21. Import and known_hosts

**Context.** The import dialog (`docs/ui-behavior-spec.md` 2.2) is not described, nor are the CSV columns, the PuTTY fields or the idempotency key behind "running it again does not create duplicates" (`SPEC.md`, "Import"). Real `known_hosts` files contain hashed entries that cannot be shown as an address (10.1), and the `@cert-authority` and `@revoked` markers were not covered.

**Decision:**

- Import is a modal dialog: source (PuTTY only on Windows) → file with a default path → preview with "new / already exists / error" statuses → summary "added N, skipped M, errors K".
- CSV columns: label, address, port, username, group, tags.
- Idempotency uses the source key (type and Host alias, PuTTY session name or UUID), or (address, port, username) when there is none. Existing records are never overwritten.
- Keys referenced by IdentityFile are imported into the Keychain, with a passphrase prompt.
- Hashed known_hosts entries are stored as they are and matched on connect through HMAC. `@revoked` entries are imported as revoked keys; `@cert-authority` entries are skipped and counted.
- Connections use only the app's own known hosts store.

**Spec impact:** `docs/ui-behavior-spec.md`, 2.2 and a new import dialog (2.5), 10.1 and 10.2; `SPEC.md`, "Import".

### 22. Interface and documentation language

**Context.** `SPEC.md`, "Quality", requires English code and documentation. The specifications are now in English with the Russian originals in `docs/ru/`, and the interface strings were translated. It was unclear whether these strings are final (e2e tests would check them), whether i18n is needed and which version of the specifications is canonical.

**Decision:**

- The interface is in English. All strings live in an i18n catalog (`en`). Strings in the specification define the meaning; the final texts live in the catalog.
- e2e tests locate elements by `data-testid`, not by text.
- The English specifications are canonical. Changes are made to both language versions in the same commit.
- New `docs/*.md` files are written in English only. Stage reports are in Russian.

**Spec impact:** `SPEC.md`, "Quality"; `CONTRIBUTING.md`.

### 23. Unspecified parts of the shell and the Hosts screen

**Context.** `docs/ui-behavior-spec.md` 2.2 has a button marked [clarify] without a [decision]; drag-to-reorder (2.3) conflicts with the sort menu (7.1); the notification center (1.1) and multiple windows are mentioned but not described. A bare `host` counts as an address (2.1), and broadcast, theme import and cursor style from `SPEC.md` have no UI.

**Decision:**

- The unnamed button in 2.2 is the sort menu, with a "manual" mode that is the default on Hosts. Dragging reorders cards only in this mode.
- Notification center: a toast for 5 seconds and a list with an unread counter. It is kept in memory only; there are no system notifications.
- All windows are equal. Data and settings live in the Rust core and are broadcast to windows as events. Moving tabs between windows is not in the MVP.
- A single word counts as an address only if it contains a dot or a colon, or is an IP address or `localhost`. If the text exactly matches a saved host, Enter connects to that host.
- Broadcast is an item in the menu of a tab with splits; receiving panes are marked with a border. "Import theme…" and Cursor style (block / bar / underline, blinking) go to 12.4. Tags go to the snippet panel.

**Spec impact:** `docs/ui-behavior-spec.md`, 1.1, 1.4, a new notification center (1.5), 2.1, 2.2, 2.3, 4.4, 4.7, 9.3 and 12.4; the window model is recorded in `docs/architecture.md`.

## Design system

### 24. Contrast test: pairs, thresholds and failing values

**Context.** `docs/design-system.md` §7 requires a test of all "text / background" pairs against the §1 thresholds, but does not list the pairs. Checking the full matrix fails from day one: `--text-muted`, the light primary button, light `--warning` and `--success` as text, white on two tag colors, several ANSI colors and text on `*-subtle` backgrounds fall below their thresholds.

**Decision:**

- The pairs are listed in an explicit file with their thresholds. Values are compared without rounding.
- 4.5:1: `--text-primary`, `--text-secondary`, `--accent-text`, `--danger`, `--info`, and `--warning` and `--success` where they are used as text, on all 7 surfaces of both themes; `--text-on-accent` on `--accent`, `--accent-hover` and `--accent-pressed`; terminal foreground and ANSI colors 1–6 and 9–14 on their theme's background.
- 3:1: icons, white on `--tag-*`, the focus ring, status dots, `--text-muted` (only for placeholders and captions that repeat information shown elsewhere) and bright black.
- Not checked: `--text-disabled`; black, white and bright white of terminal themes; imported themes.
- Text on `*-subtle` backgrounds is always `--text-primary`; the semantic color goes only to the icon.
- Minimal value changes that keep the hue: light `--accent` and `--focus-ring` → `#13856E`; light `--text-muted` → `#88867F`; light `--warning` → `#8D620B` and `--success` → `#36782F`; `--tag-jade` → `#1D8571`, `--tag-amber` → `#9D6C1E`; Graphite bright black → `#62625C`; Paper bright red, green, yellow, blue, magenta and cyan → `#C6483F`, `#458034`, `#916D1A`, `#3E74B6`, `#9B57B3`, `#1B8070`.
- Found while implementing the test: dark `--danger` `#F0645C` gives 4.457:1 on `--bg-raised-hover`, so it becomes `#F0665E` (4.513:1, same hue), together with `--danger-subtle` and the dark Error highlight color. The light accent states follow the new base accent (`--accent-hover` and `--accent-text` `#0F7461`, `--accent-pressed` `#0B6051`).

**Spec impact:** `docs/design-system.md`, §1, §2.3–2.5, §3.1, §4.1, §4.2 and §7.

### 25. Terminal theme format, roles, highlighting and tab accent

**Context.** `docs/design-system.md` §7 stores themes "in a format that supports importing and exporting color schemes", but `SPEC.md` names no format, and the themes lack roles for text under the cursor, selected text and search matches. `docs/ui-behavior-spec.md` 1.1 colors the active tab with the theme's accent, which conflicts with the single jade accent, and the keyword highlighting colors (§4.3) are tied to the UI theme rather than the terminal background.

**Decision:**

- Terminal themes use our own documented JSON format: `name`, `version`, `background`, `foreground`, `cursor`, `cursorText`, `selection` (with alpha), `selectionForeground` (optional), `searchMatch`, `searchMatchActive` and the 16 ANSI colors.
- `.itermcolors` files and Windows Terminal schemes are imported through converters.
- Built-in themes live in `src/theme/terminal/`. Imported themes are stored as records and synced.
- The highlighting color column (dark or light) is chosen by the brightness of the terminal theme's background, not by the UI theme. Values are stored as hex.
- The active tab is marked with an `--accent` bar instead of a theme color.

**Spec impact:** `docs/design-system.md`, §4 and §7; `docs/ui-behavior-spec.md`, 1.1 (active tab) and 12.4 (highlighting colors).

### 26. Missing tokens for the base components

**Context.** `docs/design-system.md` §1 forbids raw hex values in component code, but the stage 0 components need states that have no tokens: "pressed: one step darker than hover" (§6) does not work in the dark theme, white on dark `--danger` is below 4.5:1, and menu hover is invisible on `--bg-overlay`. It was also unclear how `--text-disabled` combines with opacity 0.45, and tag chips on `--accent-subtle` conflict with "not used for decoration".

**Decision:**

- New tokens in `docs/design-system.md` and `tokens.css`: `--bg-pressed` (lighter than hover in the dark theme, darker in the light theme), `--bg-overlay-hover`, `--danger-pressed`, `--text-on-danger` (dark `#08201B`, light `#FFFFFF`), `--success-subtle` and `--info-subtle`.
- Other states derive from existing tokens: an off toggle uses a `--border-strong` track; progress uses `--accent` on `--border-subtle`; a toast uses `--bg-overlay` with `--shadow-overlay`.
- Opacity 0.45 applies to a whole disabled component, and `--text-disabled` only to the text of inactive menu items. The two are never combined.
- Unselected tag chips are neutral; `--accent-subtle` is kept only for the active filter.

**Spec impact:** `docs/design-system.md`, §2 and §6.

## After stage 2

### 27. UI stack: GPUI Kit instead of Tauri + React

Status: accepted, 2026-10-19 (owner's choice).

**Context.** Stages 0–2 were built on Tauri 2 with a React + TypeScript webview and xterm.js. Measured idle on Windows, that build used 386 MB working set and 204 MB private memory in 8 processes. A prototype of the main window on GPUI Kit (crate `gpui-kit` 0.7.3 over `gpui-pre` 0.3.9, GPUI from the Zed editor, both Apache-2.0), with a local terminal on alacritty_terminal and portable-pty, used about 53 MB working set on the Vaults screen and about 100 MB for the whole process tree after 100,000 terminal lines with a full 10,000-line scrollback, in 3 processes (the app, conhost and the shell). The cost: GPUI is published as frequent snapshots with breaking changes, and the terminal widget is our own code.

**Decision:**

- The desktop UI is GPUI Kit, without a webview. `apps/desktop` is the GPUI Kit application crate `shellpp-desktop`; there is no frontend package.
- The terminal is alacritty_terminal (parser, grid, scrollback) with our own GPUI element for rendering and input. Code from Zed's GPL-licensed crates (`terminal`, `terminal_view`) is not used or copied.
- The mobile client gets a separate UI later. The logic stays in `crates/core` and `crates/ssh`, which never depend on GPUI.
- UI acceptance: GPUI tests (`TestAppContext` / `VisualTestContext`, headless) plus screenshots of our own application's window.
- Performance targets are part of `SPEC.md` ("Performance") and are checked by a benchmark.
- The theme goes through the gpui-component theme system (`docs/design-system.md` §7).
- The Rust toolchain is pinned to 1.99.1 in `rust-toolchain.toml`: `gpui-pre` 0.3.9 needs Rust 1.93 or later.

Superseded parts of earlier decisions and documents:

- D1: the frontend-only package (pnpm, Vite, React, TypeScript), `tokens.css` and the Vitest tests.
- D2: the npm package name `@shellpp/desktop`; the desktop crate is `shellpp-desktop`.
- D4: WebView2 and WebKitGTK as platform dependencies, and the npm license check.
- D6: WebdriverIO and tauri-driver e2e tests and the macOS exception that came with them. Kept: the mocks behind a cargo feature that never reaches release builds, OpenSSH in Docker in the Linux job and the russh test server for Windows.
- D7: tauri-plugin-updater and the Tauri bundler. The update rules (off by default, manual check, own source and signing key) and the package formats stay; the tools are chosen before stage 9.
- D16: commands are read from the alacritty_terminal grid instead of the xterm.js buffer; the capture rules are unchanged.
- D22: `data-testid` and the JSON i18n catalog. UI tests find elements by element id, not by text; the interface stays in English with one `en` string catalog.
- `docs/security.md` §13: the Tauri CSP and capabilities.
- `docs/architecture.md` §2–3: the parts about IPC commands and events and the webview.

**Spec impact:** `SPEC.md`, "Clean room", "Default stack", "UI/UX" (acceptance), "Security", new "Performance" section, "Quality" and "Work order"; `docs/design-system.md` §7; `docs/ui-behavior-spec.md`, section 0. `docs/architecture.md` and `docs/security.md` carry a note and will be rewritten.

### 28. Unimplemented sections are visible with an "in development" placeholder

Status: accepted, 2026-10-19 (owner's audit of the main window).

**Context.** D3 hides elements of future stages behind a stage flag. The owner's audit of the GPUI main window asks for the full window frame from the start: the persistent SFTP tab (`docs/ui-behavior-spec.md` 1.1) and all six sidebar sections (1.2), even before their screens exist.

**Decision:**

- The top bar always shows the persistent Vaults and SFTP tabs. The sidebar always shows all six sections: Hosts, Keychain, Port Forwarding, Snippets, Known Hosts and Logs.
- A tab or section whose screen is not implemented yet opens an "In development" placeholder in the main area. It is not shown disabled; navigation to it works as usual.
- This replaces D3's hide-until-stage rule for the window frame and the sidebar only. Elements of future stages inside screens (menu items, buttons, fields) stay hidden behind the stage flag.
- Every stage report lists what is still hidden and what is shown as a placeholder.

**Spec impact:** `SPEC.md`, "Work order" (hidden-feature rule); `docs/ui-behavior-spec.md`, section 0.

### 29. Closing stage 1: master password, lock and the Hosts screen

Status: accepted, 2026-10-20 (owner's choice).

**Context.** The owner's task that closes stage 1 describes the first run, the unlock screen and locking (`docs/ui-behavior-spec.md` 13.1–13.3) and asks for a tag filter, a vault icon, every block of the host panel and a minimum window size. Several points differ from the stage 0 decisions, which chose stricter defaults: nothing in the OS keychain by default and auto-lock after 15 minutes (D8, D9), a recovery key shown at the first run (D9, D11), a 10-character minimum (`docs/security.md` 5.4), later-stage fields hidden (D3) and Ctrl+Shift+L for a new local terminal (D17).

**Decision:**

- Master password: at least 8 characters (Unicode characters after NFC), with no rules about the kind of characters. The first-run screen shows a strength indicator as advice only.
- "Remember on this device" is on by default and is set on the first-run screen. A device key in the OS keychain (crate keyring) wraps the vault key, so the app does not ask for the password at launch; lock and auto-lock still ask for it. Without an OS keychain the option is hidden, as before (D5, D7).
- Auto-lock: Never, 5, 15, 30 or 60 minutes, Never by default. Manual lock: the Lock item in the app menu and Ctrl+Shift+L, which no longer opens a local terminal.
- No recovery key is shown or required in local mode. The first-run screen warns that a forgotten master password cannot be recovered. The recovery key is shown when a sync account is registered (UI 13.5). The core still creates it at the first run and keeps it in the personal vault (the `recovery` wrap and the `recovery_key` record), because the key rotations and the sync design rely on that wrap; the app never shows it in local mode, so locally it cannot be used.
- "Forgot password?" on the unlock screen says that recovery is impossible and offers "Restore from sync", shown disabled until sync exists, and "Start with a new vault". The latter asks for an explicit confirmation, renames the data file (`shellpp.db` with its `-wal` and `-shm` files) instead of deleting it, and returns the app to the first run.
- After 5 wrong passwords in a row, unlocking pauses: 5 seconds, doubling after each further wrong password, at most 5 minutes. The core enforces the pause before it derives any key, not only the screen, and keeps the counter across restarts in plaintext in the database (`meta` table), because the counter is not a secret. A correct password resets it. Wrong passwords never delete data.
- While locked, open session tabs stay alive; their content is hidden behind the unlock screen.
- Tests, the demo mode and the screenshot mode never touch the OS keychain or the permanent profile: they use the in-memory secret store and a temporary `SHELLPP_DATA_DIR`. With "remember on this device" on by default, this keeps test runs from writing keychain items.
- Hosts screen: the tag filter is a button with the tag icon that opens a dropdown with every tag (with host counts), a search field, checkboxes and "Clear"; the selected tags are chips next to the button, and those that do not fit collapse into a "+N" chip that opens the dropdown (UI 2.2). The vault icon is the app's own safe icon (an original drawing on the Lucide 24 px grid, stroke 2: a safe with a dial, a handle and feet), served under `IconName::Vault` in the Vaults tab and in the Vault selectors of the host and group panels. It replaces Lucide "vault", whose four bolts read as a cross ("close") at 16 px; being our own drawing, it needs no third-party license line. Copy to ▸ works within the current vault for now: "Top level" or a group, a copy with the same label (UI 2.4).
- Host panel: every block of UI 3.1 is shown, including the "+" for a key, an identity or a certificate, all "Show more" fields with Mosh, and Add Telnet. Their values are saved; what they do in a session comes with the stages that implement them.
- Minimum window size: 960 × 600 px. Nothing is clipped or overlaps at that size, with the 380 px details panel open or closed.

The owner chose convenience at launch over the stricter defaults. The cost is stated in `docs/security.md` (section 8, RR8): with the key remembered, any program running as the same OS user can read the keychain item, and starting the app in the user's session opens the data without the password. Locking still wipes the keys from memory.

Superseded parts of earlier decisions:

- D3: Mosh and Telnet fields are no longer hidden until their stages: they are shown and their values are saved. "Restore from sync" is shown disabled instead of hidden.
- D8: "by default nothing is stored in the OS keychain" and "remember on this device, off by default".
- D9: the recovery key step of the first run and the auto-lock default of 15 minutes (the app no longer offers 1 minute). The lockout and "Forgot password?" are added.
- D11: "the recovery key is created together with the first password, also without an account", as far as the user sees it: in local mode the key is not shown or required.
- D17: Ctrl+Shift+L is the lock shortcut, not a second shortcut for a new local terminal.
- `docs/security.md` 5.4: the minimum length of 10 characters becomes 8.

**Spec impact:** `docs/ui-behavior-spec.md`, sections 0, 1, 1.4, 2.2, 2.4, 3.1, 12.2, 12.5 and 13; `docs/design-system.md` 6; `SPEC.md`, "Default stack", "Sync and team access" (recovery key) and "Work order"; `docs/security.md`; `docs/data-model.md` 7.3 (Mosh) and 16.1 (auto-lock default); `docs/architecture.md` 3.3 (auto-lock default); `docs/storage-format.md` 2.3 and 6.3 (keychain default); the new `docs/data-format.md`, which describes the implemented on-disk format.

### 30. Stage 2a: the SSH terminal

Status: accepted, 2026-10-20 (owner's task for stage 2a; the points below are the lead's choices where the specification leaves the details open).

**Context.** The owner split stage 2 (`SPEC.md`, "Work order") into 2a, the SSH terminal (UI 4.1, 4.3, 4.6, 4.8, 10.2, 12.4 "Terminal" and "Keepalive", 12.5), and 2b and later stages: splits, the terminal sidebar, the new tab page, Ctrl+K search, the Known Hosts screen, SFTP and port forwarding. The specification does not say how the terminal runs next to GPUI's executor, where the bundled font comes from, which keys a session can use before the Keychain screen (stage 3), how the entry points of 2b features look in the meantime, or which settings 2a shows.

**Decision:**

- Runtime: the session layer owns a small Tokio runtime on background threads for russh. Network I/O and terminal parsing (alacritty_terminal behind a fair mutex) never run on the UI thread; results reach GPUI through async channels, repaints are coalesced to at most one frame per display refresh, and output has flow control, so a flood such as `seq 1 3000000` neither freezes the UI nor grows memory without bound.
- Font: JetBrains Mono (OFL-1.1) Regular, Bold, Italic and Bold Italic, TTF files taken only from the official release of JetBrains/JetBrainsMono on GitHub, embedded with `include_bytes!`, listed under "Fonts" in `THIRD_PARTY_NOTICES.md` and shipped with the OFL text.
- Keys until stage 3: the files `~/.ssh/id_*` and the system agent (`SSH_AUTH_SOCK`, Pageant, the Windows OpenSSH agent pipe). An encrypted key asks for its passphrase in a dialog; the passphrase can be kept in memory for the rest of the app session and is never written to storage (the "Save" checkbox of UI 4.8 comes with the Keychain). The key directory and the agent are injected, so tests use a temporary directory with keys generated at run time and their own agent, never the real `~/.ssh` or agent.
- Entry points of 2b and later features (menu items, the "+" new-tab button) are shown disabled (`docs/design-system.md` 6) and do nothing, instead of being hidden or opening a placeholder. The "In development" placeholders of the sidebar sections and the SFTP tab stay (D28). From a session tab, a new local terminal is opened from the app menu until the "+" button works (UI 12.5).
- Settings → Terminal shows only the "Terminal" block with Autoreconnect (on), Copy on select, Bell sound (off), Bright colors for bold text and the TERM type (`xterm-256color`); the "Font" block (the bundled JetBrains Mono and the installed monospace fonts, size 14 with −/+); the default terminal theme (Graphite or Paper); and "Keepalive" (30 s, 0 = off). The other items and blocks of UI 12.4 stay hidden until their stages.

Superseded parts of earlier decisions and documents:

- D3 and D28: the menu items and buttons of 2b features are shown disabled instead of hidden.
- `SPEC.md`, "Work order", stage 2: "autoreconnect, keepalive, and TERM are stored without a settings screen".
- `docs/design-system.md` 5: "Stage 0 does not bundle Inter and JetBrains Mono yet", as far as JetBrains Mono goes.

**Spec impact:** `SPEC.md`, "Work order"; `docs/ui-behavior-spec.md`, section 0; `docs/design-system.md` 5.

### 31. Stage 2b: splits, windows, the new tab page, the terminal sidebar and Known Hosts

Status: accepted, 2026-10-20 (owner's task for stage 2b and the polish of stage 2a; the points below are the lead's choices where the specification leaves the details open).

**Context.** The owner accepted stage 2a, asked for its polish (tab icons and the state dot, the key change dialog, minimum contrast, the default local shell) and then for stage 2b: splits (UI 4.7), the tab menu and several windows (4.4, 1.4, 2.4), the new tab page with Ctrl+K search and Restore (6), the terminal sidebar (4.2) and the Known Hosts screen (10.1). The specification does not say how panes nest and close, how windows share state, what Restore keeps, which new shortcuts leave the shell alone, how items of later stages look inside these features, or the details of search, history, contrast and known_hosts import.

**Decision:**

- Splits: a tab holds a tree of binary splits that nest. A split halves the active pane; the new pane shows a compact new tab picker whose first row is the split pane's target (the same host or address, or a new local terminal with the same shell); Esc in it undoes the split. Ctrl+Shift+E splits below, Ctrl+Shift+D to the right, Ctrl+Alt+arrows move focus to the nearest pane on screen. The design system names no pane size, so each pane keeps at least 128 px (four times `--space-6`); a divider is a 1 px `--border-subtle` line with a `--space-2` grab area, the active pane of a split tab has the 2 px `--accent` bar along its top edge, and a tab holds at most 16 panes.
- Closing panes: exit code 0 closes the pane and its neighbors take its space; a non-zero exit and a drop keep the pane with their message (UI 4.6 per pane); the last pane closes the tab by the neighbor rule. Ctrl+Shift+W closes the active pane while the tab has more than one. A tab with splits shows the active pane's host and the worst state of its panes (a drop, then connecting, then an ended session, then connected).
- Tab menu: Duplicate, Duplicate in a new window, Rename, Split horizontally, Split vertically, Broadcast input (only on a tab with splits, disabled until stage 9), Close. Start multiplayer is not shown. Duplicate opens a new session to the same target, a local terminal with the same shell. Rename edits the title in place; an empty title brings back the host name, and the title lives as long as the tab and is part of Restore. A title keeps at most 256 characters and no control characters, an unchanged title stays the host's, and panes do not take the focus from the field while it is open. Duplicate, a new split pane and Restore of a local terminal use the shell it was resolved to when it opened, not the default at that later moment. The splits, in the menu and as shortcuts, are disabled while the active pane still chooses a host.
- Windows: each window has its own top bar, Vaults and SFTP tabs, session tabs, details panel and terminal sidebar; data, settings, sessions and the lock belong to the app. A new window takes the current window's size, offset by `--space-6`. Closing a window with live sessions asks first, and the app quits with its last window. Alt on a connect action (Connect ▸ items, Quick Connect, Connect in the host panel, double click) opens the session in a new window.
- New tab page: "+", Ctrl+T outside the terminal and Ctrl+Shift+T everywhere (Ctrl+T swaps characters in readline). Ctrl+K outside the terminal and Ctrl+Shift+K everywhere open the same search as an overlay over the current window; it covers the open tabs of all windows first, then the hosts. Recent connections come from the connection log: one row per target, newest first, at most 10. Create a workspace is shown disabled until stage 9.
- Restore: the set is saved when the app exits, when its last window closes and before it locks (it needs the vault's keys). It holds only structure and targets (host record ids, `user@host:port` of quick connects, local shell ids, renamed titles, the pane tree with its sizes), never secrets or terminal content, and is stored encrypted on the device without sync (the `last_session` record, `docs/data-model.md` 16.3; it replaces the plaintext `session.last` setting of stage 0). The first saved window is restored into the current window, the others open as new windows; Restore works once per launch. A tab the core would refuse is left out of the save, so one bad tab does not lose the others; when nothing can be restored, a notification says so.
- Terminal sidebar: one per window, 320 px (`--width-details-min`), acting on the active pane. Search is plain text, case-insensitive, over the screen and the scrollback, at most 10,000 matches, in the theme's `searchMatch` and `searchMatchActive` colors. Autocomplete is not shown; Snippets shows a stage 5 empty state. The font settings are global; a theme click is saved in the host and applies to the session only for a quick connect or a local terminal. Eight open schemes are bundled (Dracula, Nord, Solarized Dark and Light, Gruvbox Dark, Tokyo Night, Catppuccin Mocha and Latte; `docs/design-system.md` 4.4); they keep their original colors, are marked "new" and stay out of the contrast test of our own themes. Where a scheme has no search colors, `searchMatch` and `searchMatchActive` are its ANSI yellow over its background at 25 % and 40 %, a missing cursor color is its foreground and a missing cursor text color its background; the source and license of each scheme are recorded in the theme registry and `THIRD_PARTY_NOTICES.md`, since the bundled JSON files (also examples of the import format) cannot hold comments.
- History: recorded only for saved hosts, never for a quick connect or a local terminal. The command is read when the cursor leaves its line after Enter (at most 5 s later), keys typed ahead of the previous line drop both lines, and a password prompt word followed by `:` inside the captured line drops it (the long words also without a word boundary, since input typed ahead runs into `su`'s `Password:`). A line with no prompt on its own rows is held until the next key or 5 s and dropped when a password prompt follows it. The terminal applies "Record history" itself (read at the start and on every change of hosts, settings or the lock), and the core drops the lines of a host removed while its session runs. The hint ranks commands by count, then by latest use, checks each one again with the secret filter, keeps "Save" disabled until stage 5 and remembers "Close" per host (`history_hint_state`). Echo-off detection and its limits are in `docs/security.md` 10.1.
- Known Hosts: Import suggests `~/.ssh/known_hosts` through an injected path, so tests and the demo never read the real file. Hashed entries are kept as they are, shown as "Hashed host" (the owner's wording) with the key type and matched by HMAC at connect; `@revoked` entries are imported as revoked, and `@cert-authority` lines and negated (`!`) patterns are skipped. The summary counts imported entries, duplicates, skipped entries and invalid lines; a file over 16 MiB is refused. Against hostile files, a line holds at most 256 host patterns and a file gives at most 200,000 entries with at most 64 MiB of text; the new entries are written in transactions of 2,000, letting the core go between them, so connections can check host keys during a large import. Export writes hashed entries as they are and revoked ones with `@revoked`. Remove has a confirmation and undo, as for hosts. Until the full import dialog (stage 3) the screen's import has only the File step and the summary; the view toggle is remembered in `view.known_hosts.layout`, the sort is not.
- Polish of stage 2a: the tab's left icon is the OS icon or the neutral server icon, never the terminal glyph; Linux gets the app's own penguin on the Lucide grid (not a logo, no third-party license). The 8 px state dot follows the title; after a non-zero exit it is inactive (`--text-muted`). A dropped session dims its terminal to `--opacity-disabled` and hides the cursor until it reconnects. The key change dialog opens with the focus on Cancel; Enter and Esc cancel, and "Replace saved key" answers only a click or Tab + Space.
- Settings → Terminal: "Adjust indistinguishable colors" (`terminal.min_contrast`: `off`, `palette`, `always`, `palette` by default) moves text below 4.5:1 toward white or black, away from the background first, by the smallest step, cached per color pair; `palette` never touches truecolor. Line spacing (1.2) and "Record command history" (on) are shown. The local terminal defaults to PowerShell 7 (`pwsh`) when it is installed, else Windows PowerShell, and its tab shows the shell's display name.
- Not in 2b: workspaces, Broadcast input, multiplayer, Autocomplete, snippet content (stage 5), shell history import, keyword highlighting, the OS detection script settings, SFTP, port forwarding and the shortcut editor of UI 12.5.

Superseded parts of earlier decisions:

- D28 and D30: the Known Hosts placeholder becomes its screen, and the entry points of 2b features (splits, the terminal sidebar, the new tab page and its "+" button, Ctrl+K search) work instead of being shown disabled. Items of later stages inside them stay disabled by the D30 rule, except Start multiplayer and Autocomplete, which the owner asked to hide.
- D30: "Settings → Terminal shows only the Terminal block with ...": "Adjust indistinguishable colors", line spacing and "Record command history" are added.

**Spec impact:** `docs/ui-behavior-spec.md`, sections 0, 1.1, 1.4, 2.3, 2.4, 3.1, 4.2, 4.4, 4.6, 4.7, 6, 10.1, 10.2, 12.4 and 12.5; `docs/security.md` 10.1; `docs/data-model.md` (`terminal.min_contrast`, `terminal.line_height`, `terminal.local_shell`, `history_hint_state` and the Restore set); `docs/design-system.md` (the bundled terminal themes); `THIRD_PARTY_NOTICES.md` (terminal themes).

## Night of 2026-10-21

The decisions below were taken during the owner's night run (night plan, section 1.1): the lead chose where the specification leaves the details open, marked each one "night", and lists the disputable ones for the owner in the night report. The owner's answers to them are D50.

### 32. Keychain keys: generation and import formats (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, items 3 and 4; the points below are the lead's choices where the plan and the specification leave the details open).

**Context.** The night plan asks for Generate key with Ed25519 (default), ECDSA P-256/384/521 and RSA 3072 and 4096 "(2048 not offered)", and for the import of OpenSSH, PEM PKCS#1 and PKCS#8 (encrypted too) and PuTTY `.ppk` v2 and v3 keys, with a type "unknown" and a clear reason for an unrecognized file. `docs/data-model.md` 9.1 listed RSA 2048 among the generated sizes and says that unsupported keys are "kept and shown as unknown".

**Decision:**

- Generate key offers Ed25519 (default), ECDSA P-256, P-384, P-521, RSA 3072 (the default RSA length) and RSA 4096. RSA 2048 is not offered; imported RSA keys of 2048 bits or more stay accepted. The key comment is the label; a passphrase, when given, encrypts the new key in the OpenSSH format (aes256-ctr, bcrypt-pbkdf, 16 rounds), and "save in the vault" works as for an import.
- Import reads OpenSSH, PEM PKCS#1 (`RSA PRIVATE KEY`), PKCS#8 (`PRIVATE KEY`, and `ENCRYPTED PRIVATE KEY` with PBES2: PBKDF2 or scrypt with AES-CBC), SEC1 (`EC PRIVATE KEY`), the legacy OpenSSL encryption of PKCS#1 and SEC1 (`Proc-Type: 4,ENCRYPTED`, `DEK-Info` AES-128/192/256-CBC; DES-based ciphers are refused with the reason) and PuTTY `.ppk` v2 and v3. Every key is stored in the OpenSSH format; one that came encrypted is encrypted again with the same passphrase, so it is never stored unencrypted.
- Against hostile files the key derivations an imported file asks for are bounded before anything is derived, as the bcrypt-pbkdf rounds are (1024): PBKDF2 at most 10,000,000 iterations, scrypt at most 256 MiB, the Argon2 of `.ppk` v3 at most 1 GiB, 64 passes and 64 lanes; a `.ppk` header given twice is refused as damaged (ssh-key keeps the last one, so the first could hide a costlier value; amended after the night review).
- A text that is not a usable key is not saved: the key panel shows the type "unknown" and the reason (not a private key, damaged, unsupported key type, RSA under 2048 bits, unsupported encryption, too expensive key derivation, passphrase needed, wrong passphrase) before the user saves. A stored record that this client cannot parse (for example one synced from a newer client) is still kept and shown as "unknown" (data-model 9.1).
- Tests generate every key at run time: in the test with the crates that define each format; against PuTTY's own puttygen (`tests/putty`, a Docker image, also run by the CI integration job); and against the `openssl` command when it is on the PATH.

**For the owner:** whether an unrecognized file should rather be saved as an "unknown" key record (the literal reading of data-model 9.1) instead of being refused with its reason.

**Spec impact:** `docs/ui-behavior-spec.md` 7.1 (generation parameters); `docs/data-model.md` 9.1 (RSA sizes).

### 33. The Keychain screen: creating keys, the hidden private key and what is not built yet (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, items 1-4, UI side; the points below are the lead's choices where the plan and the specification leave the details open).

**Context.** The night plan asks for the Keychain screen (UI 7.1, 7.2), the key panel with the mandatory security requirements of 7.3, Generate key and the import of existing keys, on top of the core of D32. The specification does not say how a key is created under the saving rule of 1.3 when its creation runs a key derivation or a generation, how "Import from ~/.ssh" reaches the folder, what the panel shows of a key that cannot be read, or how the items of later work look meanwhile. The core could not rename a key, give its public key line or reveal and copy its private key.

**Decision:**

- Creating a key: New key and Generate key commit with an explicit button (Import, Generate) instead of "created once the required fields are valid" (1.3). Creating a key runs a key derivation (an encrypted import: bcrypt-pbkdf, PBKDF2, scrypt or Argon2) or a generation (RSA takes seconds) and settles the passphrase choice (save in the vault or ask on every connection), which must not happen while the user still types the passphrase or has not chosen. Afterwards the new key's panel opens and its label is saved as in 1.3 (autosave, an invalid label outlined and not saved). An import without a label takes the key comment, else the algorithm name, as the core does; Generate key requires a label.
- A duplicate (the same SHA256 fingerprint) opens the existing key's panel with the toast "This key is already in the Keychain"; merging stays a later item (7.2).
- The private key: Show reads it through the core (`secret_reveal` of the `private_key` path, which the core now allows for keys) and only the key panel keeps it, while it is shown. It is dropped on Hide, when the panel closes, shows another key or a form, on Esc, when the window shows another section or tab (the shell tells the screen on every frame) and on lock. Copy private key never passes through the UI: the core's `secret_copy` writes it to the OS clipboard as a sensitive value, and its clipboard guard clears it after 30 seconds if the clipboard still holds it (docs/security.md 9.4). The public key is copied as ordinary text. The core adds `key_label_set` (the label rules of a key record) and `key_public_key` (the stored line, else derived from the private key).
- The type shown: ED25519, ECDSA with its curve (P-256, P-384, P-521), RSA. A stored record this client cannot read shows "unknown" with the `--warning` icon on its card and the reason in its panel. In the import form, a text that is not a usable key shows "unknown" with the reason of the core's `inspect` (D32), and Import stays disabled.
- Import from ~/.ssh: GPUI's file dialog cannot start in a given folder, so the button opens the New key form with the private key files of `~/.ssh` (files up to 64 KiB that the core recognizes as a private key; `.pub` files, `config`, `known_hosts` and the like are not listed); a click puts one into the form. The folder is injected (`keychain::set_ssh_folder`), so tests, the demo and the screenshots never read the real one, and a test build sees none, as for known_hosts (D31).
- Not built tonight, shown disabled by the D30 rule: Certificate and FIDO2 in the action bar, New identity in the New key menu, Export to host in the panel's Key export block, Duplicate and Export to file in its "…" menu. Remove is left out: the core's record removal and its undo cover hosts and groups only. The panel's Certificate block (7.3, block 5) is not shown; a certificate is attached from the host panel ("+ -> Certificate") until the Certificate item is built. Identity cards are read-only until the identity panel (7.4).
- The view toggle and the sort are remembered (`view.keychain.layout`, `view.keychain.sort`: the screen's sort modes are those of the setting). The forms' Cancel and Import or Generate are pinned below their scrolling body, like the host panel's Connect; Import or Generate is the panel's one accent button, and the empty state's Generate is the screen's accent button while no panel is open.
- Demo and screenshots: the demo profile gets three keys generated when it is seeded (Ed25519 and ECDSA P-256, without a passphrase and without RSA, so the seeding stays fast) and two identities. The screenshot states are keychain, keychain-key-panel, keychain-generate, keychain-import-unknown and keychain-empty (`scripts/ui-screens.ps1 -Stage3`).

**For the owner:** whether a key should rather be created by the literal rule of 1.3 (an unencrypted key saved as soon as it is pasted), and whether Remove for keys should come with an extension of the core's record removal now.

**Spec impact:** `docs/ui-behavior-spec.md` 1.3, 7.2 and 7.3 (and `docs/ru`).

### 34. Export and import of all data in JSON (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, item 11; the points below are the lead's choices where the plan and the specification leave the details open).

**Context.** The night plan asks for the JSON export with a schema in `docs/export-format.md`, plain or encrypted with a password (Argon2id + XChaCha20-Poly1305, already defined in `docs/storage-format.md` 13.2–13.3), a warning before the plain export, and an import of our JSON that merges by id and, on a conflict, keeps both versions with a suffix on the imported one; a round trip into an empty store must give the same hosts, groups, tags, keys, identities, known hosts, forwarding rules and settings. Until now the specification said that a matching record is "already exists" and skipped (`docs/data-model.md` 18.4), and the export held records only.

**Decision:**

- The export holds every live record of the personal vault except the recovery key (storage-format 13.2), plus the app settings as an optional `settings` member (allowed within v1). `docs/export-format.md` gives the JSON Schemas of both files and the import rules.
- The export password follows the master password: at least 8 characters after NFC (D29), instead of the 10 of storage-format 13.3 written before D29.
- Import merges by record id into the personal vault. A new id is added under that id, so references between imported records stay valid. The same id with the same content (the manual order key aside), a key with the same fingerprint and a known host with the same entry are "already exists" and skipped. The same id with other content is a conflict: the local record stays as it is, the imported one is added with a new id and " (imported)" on its label or name, and references inside the import point to the copy. Existing records are never overwritten (D21).
- Import takes groups, hosts, keys, identities and known hosts, each checked by the rules of a save; command history, the connection log and device and vault records are skipped and counted, so a backup never mixes another device's log into this one. Forwarding rules join with stage 5, which defines their record.
- Settings from the file are applied key by key unless the user turns that off; an unknown or invalid value is left out.
- Limits: 64 MiB per file, 500,000 records; KDF parameters outside storage-format 6.5 are refused before deriving.

**For the owner:** whether importing history and the connection log should be offered as an option, and whether applying the file's settings should be on by default.

**Spec impact:** the new `docs/export-format.md`; `docs/storage-format.md` 13.2 (`settings`) and 13.3 (password length); `docs/data-model.md` 18.4 (native JSON: merge by id with conflict copies).

### 35. Import of `~/.ssh/config` (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, item 10; the points below are the lead's choices where the plan and the specification leave the details open).

**Context.** The night plan asks for the `~/.ssh/config` import with Include, basic Match and Host patterns (the options of `Host *` and other patterns apply to concrete hosts, the patterns never become hosts), the field mapping (HostName, User, Port; IdentityFile into the Keychain; ProxyJump into Host Chaining; ForwardAgent; ServerAliveInterval; SetEnv/SendEnv; the forwardings), ProxyCommand hosts listed as not imported with the reason, a preview with checkboxes, no duplicates on a second import and a list of everything that could not be read. The core part is done; the dialog joins the import dialog of UI 2.5 with the Keychain screen.

**Decision:**

- Evaluation as `ssh` does it: every matching block in file order, the first value of a keyword wins, except the keywords that add up (IdentityFile, CertificateFile, the forwardings, SendEnv, SetEnv). Include takes files and `*`/`?` patterns in the file name, relative to the folder of the main file, at most 16 levels and never in a loop. A network path (`\\server\share`, `//server/share`, the NT form `\??\UNC\server\share`) in Include or IdentityFile is never opened: it is listed instead (amended after the night review; on Windows opening one connects to the server with the user's NTLM credentials). Match takes `all`, `host` (against HostName once it is set, else the alias), `originalhost` and `user`, with `!`; a Match with another criterion (`exec`, `localuser`, `canonical`, `final`) never applies and is listed, because running a local command at import is not acceptable.
- A host is the first concrete alias (no wildcard, no negation) of each Host line; the other aliases of that line are not separate hosts. Its label is the alias, its `import_key` is `ssh_config:<alias>` (data-model 18.4).
- HostName may use `%h`; another token makes the host an error. IdentityFile takes `~` and `%d`; the files are imported into the Keychain under their file name as label (an encrypted one needs its passphrase in the preview, else it is skipped), and the first imported one becomes the host's key. ProxyJump hops (`[user@]host[:port]`) become the jump chain only when every hop is a host of this config or of an earlier import; ServerAliveInterval over 3600 is cut to 3600.
- Not mapped, and named in the host's notes: SendEnv (its values come from the local environment, which the import does not read), the forwardings (forwarding rules come with stage 5), CertificateFile, and every other keyword ("Not imported: ciphers, ...").
- Limits: 4 MiB per file, 200,000 lines, 64 KiB per key file.

**For the owner:** whether SendEnv should take the current local values once at import (they may hold secrets), and whether the other aliases of a Host line should become hosts of their own.

**Spec impact:** none beyond `docs/data-model.md` 18.4, which already names `ssh_config:<first alias of the Host line>`.

### 36. Authentication order and the public key limit (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, item 8).

**Context.** The plan asks for the order "the host's key or certificate or its identity, the password, the agent, `~/.ssh/id_*`; keyboard-interactive is supported" and for not trying every key in a row, because the server drops the connection after `MaxAuthTries` (usually 6). The SSH layer tried the effective key, the agent, the key files, then the password (saved or prompted), then keyboard-interactive, with no limit on the keys offered.

**Decision:**

- Order: the effective key (certificate and FIDO2 keys join it with their stages), then the saved password, then the agent's keys, then the key files, then a password prompt when none is saved, then keyboard-interactive. A refused saved password is asked again at once (as before).
- A prompted password stays after the keys: asking for a password before trying the agent would ask every agent user for a password they do not need.
- At most 4 public keys are offered in one authentication, the effective key included (`MAX_PUBLIC_KEY_OFFERS`); the next agent keys and key files are skipped. With OpenSSH's default of 6 this leaves room for the password and keyboard-interactive. A host that needs a later key gets it linked as its key.

**For the owner:** whether 4 is right, or whether the limit should be a setting.

**Spec impact:** `docs/data-model.md` 8.6.

### 37. FIDO2 security keys at the format level (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, item 6).

**Context.** The plan asks for the import of `sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com` keys with signing through the authenticator by the system's mechanism (WebAuthn on Windows), for generation only when a reliable library with a permissive license exists, and, without hardware tonight, tests at the format level and a manual check in the morning list.

**Decision:**

- Import: OpenSSH security-key private keys (application, flags and key handle, no secret scalar) are accepted and stored like other keys, with `fido2` = `{"resident": bool}` from the key's flags (OpenSSH's `SSH_SK_RESIDENT_KEY`, 0x20). The fingerprint and the type show in the Keychain.
- Signing is not there yet: the SSH layer treats a security key as unusable and goes on with the next method (never offers it), so a host linked to one still signs in with a password or another key. The key files `~/.ssh/*_sk` stay out of the default key files, as before.
- Generation is left out tonight: it needs the authenticator (CTAP2 over HID, or Windows `webauthn.dll`), and no crate was checked tonight for reliability and license. Candidates for the owner: Windows WebAuthn through the `windows` crate (MIT/Apache-2.0, needs `unsafe` FFI in a platform module), `ctap-hid-fido2` (MIT), libfido2 (BSD-2-Clause, a C library).

**For the owner:** the choice of the authenticator path (WebAuthn API on Windows, CTAP2 elsewhere) before signing and generation are built.

**Spec impact:** `docs/data-model.md` 9.3 already describes the record; nothing changes.

### 38. Export Data and Import Data in the app menu (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, item 11: "Export data…" and "Import data…" in the app menu, marked [decision] in the specification).

**Decision:**

- The app menu (UI 1.4) gets Import Data… and Export Data… after New Serial Connection, between separators (the menu's Title Case, as its other items).
- Export Data: "Encrypt with a password" is on by default, with the password typed twice (at least 8 characters, D34). Turned off, the dialog says in `--danger` that the file holds the passwords and private keys in clear text and enables Export only after "I understand". The core builds the file off the UI thread; then the save dialog suggests the core's file name in the Documents folder; the file gets mode 0600 on Unix; a toast says how many records it holds. Cancelling the save dialog writes nothing and keeps the form.
- Import Data: a file (typed or chosen) and "Apply the settings in the file" (on). An encrypted file asks for its password next (a wrong one is asked again); the summary lists added, already here, imported as copies, skipped, errors and settings applied.
- New host ▸ Import opens the import dialog of UI 2.5 (D39), whose Shell++ export source goes on in the same Import Data.

**Spec impact:** `docs/ui-behavior-spec.md` 1.4 (and `docs/ru/`).

### 39. The import dialog of the Hosts screen (night)

Status: night decision, 2026-10-21 (stage 3, the import dialog of UI 2.5 over the core's `~/.ssh/config` import, D35).

**Decision:**

- New host ▸ Import and the empty state's "Import from ~/.ssh/config" open the import dialog instead of the "In development" placeholder. Step 1 lists the sources: the OpenSSH config (selected), a Shell++ export, known_hosts, PuTTY sessions (Windows only) and CSV; the last two are shown disabled with "Arrives in a later version" (D30 rule).
- A Shell++ export goes on in Import Data (D38); known_hosts goes on in the import of the Known Hosts screen (its File step and summary, D31). Both close the sources dialog first.
- `~/.ssh/config`: the file step suggests `~/.ssh/config` (tests and the demo set their own file; a test run never reads the real one); the preview, read off the UI thread, lists each host with a checkbox (new hosts checked; existing and refused ones disabled), its status, the reason of a refusal and the options left out; the key files with their status, a passphrase field for each encrypted one and "Save the passphrases in the vault"; the lines that could not be read; the count of pattern Host lines. Import runs off the UI thread and ends with "added, skipped, errors" plus the keys, and, when there are any, the hosts left without their jump hosts and the commands skipped (amended after the night reviews: a ProxyJump hop that is not imported stays in the host's chain as a missing host (the nil id) with a note, so the host is refused at connect (D44) instead of connecting directly; the options left out are written into the host's notes).
- `~` in the config means the parent of the file's `.ssh` folder, else the user's home folder.

**Spec impact:** `docs/ui-behavior-spec.md` 2.5 (and `docs/ru/`).

### 40. The identity hint of the host panel (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, item 7; UI 3.1; data-model 9.4).

**Decision:**

- The hint shows in the SSH block above the username for a saved host without an identity, when at least one other host without an identity has the same effective username (its own or a group's) and the same saved password, or the same effective username and key; a password match wins over a key match. The text names how many other hosts share them; "Create an identity" is one link.
- The core creates the identity (label and username = the username; the password and the key copied when every host of the hint has the same) and moves every host of the hint onto it in one transaction: the identity linked, the username cleared, and the password and the key cleared only when the identity now holds them (data-model 9.4). Amended after the night review: clearing all three lost a shared key when the password matched, and each host's own password when the key matched. The second night review found that a connection did not read identities at all (the stage 1 resolution left `idf(L)` out), so moved hosts lost their username and password: connections now resolve data-model 8.3 in full (`username` and `key` from each level or the identity it names, `password = host.ssh.password ?? identity.password`). The open panel then shows its host as the core left it, so a later autosave writes the same values instead of undoing the move.
- Identical keys (same fingerprint) are merged by the core (data-model 18.3, `keys_merge`); since imports already match keys by fingerprint, duplicates arrive only by sync, so the Keychain's merge hint waits for stage 7.

**Spec impact:** `docs/ui-behavior-spec.md` 3.1 (and `docs/ru/`).

### 41. Which settings Import Data applies (night)

Status: night decision, 2026-10-21 (review of stage 3; D34, D38; security.md TB7).

**Decision:**

- Import Data applies only the settings of how the app and the terminal look and behave: `appearance.*`, `view.*`, `shortcuts.*`, `hints.*`, the terminal's autocomplete, autoreconnect, bell, bold as bright, copy on select, cursor, default theme, font, highlighting, keepalive, line height, minimum contrast and `TERM`, and SFTP's hidden files and parallel transfers.
- A setting that runs or opens something (`terminal.os_detection_script` and `terminal.os_detection_enabled`, `terminal.local_shell`, `sftp.open_program`), points somewhere (`updates.manifest_url`, `updates.auto_check`) or guards the user (`security.auto_lock_minutes`, `terminal.confirm_multiline_paste`, `sftp.confirm_delete`, `sftp.name_conflict`, `terminal.record_history`, `terminal.import_shell_history`, `logs.*`) never comes from a file: a shared "host list" could otherwise run its script on every server the user opens. A `terminal.` or `sftp.` key added later is left out until it is listed; the `appearance.`, `view.`, `shortcuts.` and `hints.` groups are taken whole, so a setting that runs, opens or sends something must never be added under them.
- The export still holds every setting (a backup); the summary counts the settings left out whose value in the file differs from the one here. "Apply the settings in the file" stays on by default, since what it can change is now harmless.
- Conflict copies get the `import_key` `json:<exported record_id>` and a later import matches it while the copy still holds the same (its label suffix and import key aside), so a newer version of the record becomes another copy instead of being dropped (data-model 18.4); keys and known hosts are matched before any reference is rewritten, and the copies before the other records, until no other one is found (export-format.md, section 4).

**Spec impact:** `docs/export-format.md` section 4, `docs/data-model.md` 18.4, `docs/ui-behavior-spec.md` 1.4 (and `docs/ru/`).

### 42. Private files outside the data folder (night)

Status: night decision, 2026-10-21 (review of stage 3; the owner's night plan for stage 3, item 11, asks for "on Windows, the current user's ACL"; storage-format 13.5; D38).

**Decision:**

- The private key files of the SSH files export and the plain Export Data file are written by one helper, the new workspace crate `shellpp-private-file` (`crates/private-file`): a symbolic link at the path is refused; the content always goes into a file the helper creates (the file itself, or a temporary file in the same folder renamed over an existing one, so a handle opened on the old file never sees it and a file another user owns is replaced, not reused); on Unix that file is created with mode `0600` and `O_NOFOLLOW`; on Windows it is opened without sharing and gets a protected DACL with one entry (full control for the user of the process token, nothing inherited) before a byte is written (amended after the second night review). A folder that every local user can read, or a file the save dialog replaces, no longer exposes the secrets.
- The core keeps `#![forbid(unsafe_code)]`; the advapi32 calls are the only `unsafe` code of the new crate, in `src/windows.rs`, each with its safety comment. The crate uses `windows-sys` 0.61.2 and `libc`, both already in the lockfile and in THIRD_PARTY_NOTICES.md: no new third-party package.
- The data folder itself is unchanged (storage-format section 1: the per-user profile ACLs apply).

**Spec impact:** `docs/storage-format.md` 13.5.

### 43. OpenSSH files from Export Data (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 3, item 11: "export of the ssh config and the keys in the OpenSSH format into a chosen folder, with an offer to set a passphrase"; storage-format 13.4–13.5; UI 7.3).

**Decision:**

- Export Data has a format choice: "Shell++ data (JSON)" (D34, D38) or "OpenSSH files". No other menu item: one place to export.
- OpenSSH files: "Include the key files" (on) and "Protect the key files with a passphrase" (on; passphrase and repeat, at least 8 characters, as the export password). The keys that already have a passphrase keep theirs. Turned off, the dialog says in `--danger` that keys without a passphrase are written in clear text and enables Export only after "I understand". Without key files no passphrase is asked.
- Export asks for a folder; the core writes `shellpp-ssh-config` and the key files (private keys for the user only, D42). When files of the export are already in the folder, the dialog says so and writes only after "Replace" (Back returns to the form); a toast names the hosts and keys written.

**Spec impact:** `docs/ui-behavior-spec.md` 1.4 (and `docs/ru/`).

### 44. Hosts with jump hosts before stage 5 (night)

Status: night decision, 2026-10-21 (second review of stage 3; D35; jump hosts are stage 5).

**Decision:**

- A connection to a saved host whose effective `jump_chain` (own, else inherited; `[]` stops the lookup) has hops is refused with "This host connects through jump hosts, which Shell++ does not support yet. It is not connected directly." The failure screen offers Close only.
- Until now such a host connected directly: its credentials would go to whatever answers at that address from this machine (often another host of the local network for a private address). The `~/.ssh/config` import now fills `jump_chain` from ProxyJump, so the case is common.
- A host never counts as its own hop (a bastion inside the group whose chain names it connects). An imported host whose ProxyJump hop was not imported keeps the missing hop in its chain, so it is refused too (D39).
- Stage 5 replaces the refusal with Host Chaining.

**Spec impact:** none (a temporary rule until stage 5).

### 45. The SFTP engine (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 4, item 1; UI 5); amended after the review of the engine.

**Decision:**

- SFTP is `russh-sftp` 3.0.1 (Apache-2.0; with `dashmap`, MIT) over a new channel of the existing `Connection` (`Connection::open_sftp`, the `sftp` subsystem): a host with an open terminal tab needs no second login, and the SFTP session keeps the connection alive while it is open.
- About 2 MiB are in flight per file each way: 8 reads of up to 255 KiB (the size OpenSSH allows) and 64 writes of 32 KiB, as OpenSSH's `sftp -R 64`. Requests are answered in order, so each waits behind what is in flight: their timeout is 60 s, which still lets a 300 kbit/s link through for one transfer per session, so parallel transfers each take their own session (D46); a dead link ends sooner through the connection's keepalive. A server without `limits@openssh.com` (OpenSSH before 8.6) reads 64 KiB at a time, so only 512 KiB of reads are in flight there. (16 MiB in flight, the first setting, would time out on links below about 9 Mbit/s.)
- `shellpp_ssh::sftp::Sftp`: list (a symbolic link is shown as a link with its target and whether that is a folder; links are resolved 16 at a time), stat, create and remove folders, remove files, rename, set permissions, and a recursive delete that removes a link itself and never enters it (trailing slashes are dropped, each folder is checked with its own lstat before it is entered).
- Download and upload go through `<name>.part`, renamed when complete. `resume` continues a `.part` from its length once its last 64 KiB match the source; otherwise the transfer starts over. The modification time is kept where the server allows (best effort). A new upload's permissions follow the server's umask; a replaced file keeps its own, set on the open `.part` (fchmod, so neither the umask nor an older `.part` changes it), also from one server to another. A resumed download writes at its offset (whatever lies past it is cut off) and its length is checked before the rename; an error while checking a `.part` is reported, never taken for a mismatch.
- SFTP v3 does not rename over a file. Only when the server refuses the rename and both files are there, and the old one is a file or a link (never a folder), the old file is renamed aside, the `.part` takes its name and the old one is removed; if that fails, the old one gets its name back, and if even that fails the error names where it is. A timeout or a lost connection never removes anything. Replacing a symbolic link replaces the link with the file.
- Errors are `not found`, `permission denied`, `the connection was lost` (also whenever the connection is gone, whatever the library reports), `timeout`, a local file error, or the server's own message (SFTP v3 has no code for a full disk).
- Names that are not UTF-8 on the server are shown with U+FFFD (russh-sftp decodes them lossily) and cannot be opened.
- Measured (`SHELLPP_SFTP_BENCH=1`, 200 MB, release build), to the dev server over a ~50 ms link: upload 7.0–7.2 MB/s and download 6.9–8.4 MB/s against 8.2 and 8.5 MB/s for OpenSSH's `sftp` with the same key (85–88 % and 81–99 %); to OpenSSH in local Docker about 150 and 170 MB/s against 108–120 and 138–148 MB/s.

**Spec impact:** none yet (UI 5 is unchanged; the engine has no UI tonight).

### 46. The transfer queue logic and local file names (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 4, items 3 and 8; UI 5).

**Decision:**

- The queue is a state machine without I/O (`shellpp_ssh::transfer_queue`): queued, running, waiting for a conflict answer, paused, failed with its reason (Retry queues it again, and the runner resumes from the `.part`), done, skipped, cancelled. At most `sftp.parallel_transfers` run at once (1 to 10, default 3), the oldest queued first; a lower limit lets the running ones finish. A transfer waiting for an answer holds no running slot.
- A name conflict follows `sftp.name_conflict` (ask, replace, skip, rename). An answer can apply to all: it also decides the transfers waiting now and every later conflict of the queue.
- "Rename" picks `name (2).ext`, `name (3).ext`, … with the extension after the last dot (`archive.tar (2).gz`); a leading dot is part of the name (`.bashrc (2)`).
- Speed is the progress of the last 5 seconds; the time left follows from it; a paused transfer shows neither.
- A downloaded name Windows cannot hold (`< > : " / \ | ? *`, control characters, a trailing dot or space, a reserved device name such as `CON` or `com1.txt`) is suggested in the rename dialog with each such character as `_` and a reserved name with a leading `_` (`shellpp_ssh::local_names`). An absolute Windows path of 260 characters or more is used with the `\\?\` prefix (`\\?\UNC\` for a share).

**Spec impact:** none yet (the panels and the queue UI come next).

### 47. Folder transfers and links (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 4, items 3 and 4; UI 5).

**Decision:**

- A folder transfer (`shellpp_ssh::transfer::{download_tree, upload_tree}`) creates the folders first, then sends each file through the engine (`.part`, modification time); the progress is the bytes of the whole folder.
- A symbolic link inside a copied folder is never followed, to a folder or to a file: it is left out and counted, so a link can neither pull in what lies outside the folder nor loop. (A link the user selects on its own is a later question of the panels.) Recursive delete removes links themselves (D45).
- Downloaded names that Windows cannot hold take the suggested name of D46 inside a folder transfer (no dialog per file); long paths get the `\\?\` prefix.
- Remote ↔ remote (`copy_between`): the bytes pass from one session's read to the other's write through this machine, without a local file, into `<target>.part` renamed when complete.

**Spec impact:** none yet.

### 48. The rows of a file panel (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 4, item 2; UI 5.3, 5.5).

**Decision:**

- The logic of a panel is kept apart from its view (`apps/desktop/src/sftp/{listing,local}.rs`), the same for the local and the remote panel.
- Sorting: folders and links to folders always come first, in both directions; then the column (Name, Date Modified, Size, Kind); names in natural order ignoring case (`file2` before `file10`); equal values by name. A click on the active column flips the direction; on another column it sorts by that one ascending.
- The filter matches a part of the name, ignoring case. Hidden entries (a dot-file; on Windows the hidden or system attribute) show only with Show Hidden Files.
- Back and forward as in a browser: opening a folder clears Forward; opening the folder that is open changes nothing.
- The ".." row opens the parent; there is none at a root (`/`, `C:\`). The breadcrumbs start with `/` or the drive (`C:`), whose click opens the drive selection (the letters that have a root folder now).
- Sizes are 1024-based (`512 bytes`, `1.5 KB`, `12 MB`, `1.2 GB`: one decimal below 10); folders show a dash and the kind "folder"; a file's kind is its extension in lowercase, or "file". Permissions read `-rw-r--r--` (`d` for a folder, `l` for a link), remote panel only.
- A symbolic link (or a junction) is listed as a link, never as what it points to, and shows no size (a link's own size is the length of its target path). Sorting by size orders folders by name.
- Hidden on Windows is the hidden or system attribute (as Explorer shows it), elsewhere a name with a leading dot; a remote entry is hidden when its name starts with a dot.
- Paths: POSIX roots and repeated slashes; Windows drives (`C:\`), `\\?\C:\` as `canonicalize` writes it, shares (`\\server\share\`, also `\\?\UNC\…`) as their own root, `\` for the current drive; `C:foo` is relative. A local row keeps its own path, so a name that is not UTF-8 still opens.
- Sizes are rounded before the unit is chosen (`10 KB`, not `10.0 KB`; `1.0 MB`, not `1024 KB`); permissions show setuid, setgid and sticky as `ls` does (`s`/`S`, `t`/`T`).
- The drive list touches each drive and runs off the UI thread; reading the letters without I/O (`GetLogicalDrives`) needs FFI, which this crate does not use (owner's question).

**Spec impact:** none (these are the rules of UI 5.3 written out).

### 49. Open and Open with a remote file (night)

Status: night decision, 2026-10-21 (the owner's night plan for stage 4, item 6; UI 5.4); amended after its review.

**Decision:**

- The engine part is `shellpp_ssh::open_with::OpenedCopy`; the app launches the program, warns that the copy is not encrypted, watches the copy and notifies after each upload.
- Each copy gets a folder of its own under a root inside the app's data folder (created with mode 0700 on Unix), under the file's own name (made safe for Windows, D46; long paths prefixed), so the program sees the right extension. A symbolic link is opened as the file it points to, and an upload goes there; when the server's realpath does not resolve the link, Open refuses.
- The server's state is read before and after the download; a file that changed meanwhile is downloaded again (up to 3 times).
- A copy changed locally (its size or modification time is not the one written) is uploaded back. When the file on the server changed since the copy was made (its size or modification time; for a file of up to 1 MiB also its content), nothing is written and the app asks; overwriting is an explicit `force`, which also recreates a file deleted on the server. A folder in its place is refused. What is recorded as uploaded is the copy as it was when the upload started, so a save during the upload goes up next time.
- A copy is removed with its folder when the file is closed (UI 5.4), but never with changes that are not uploaded unless forced. Each folder holds a note of what was written: at exit and at the next start after a crash the root's copies that hold nothing new are removed, and the others are returned for the user to upload or drop. Only folders this code made are touched.
- Limits: the check and the upload are not one step on the server, so a change made there during the upload of a large file is overwritten; a file over 1 MiB changed on the server with the same size within the same second is not noticed.

**Spec impact:** none (UI 5.4 states the behavior).

## The owner's answers of 2026-10-21

### 50. The owner's answers to the questions of the night

Status: accepted, 2026-10-21 (owner's choice; the 15 questions in section 3 of the night report). Amends D32, D34, D38, D41, D47 and D48; keeps D33, D35, D36, D42, D44 and D45 as written; settles the authenticator path left open in D37. What has to be built is item 3 of the plan of the next run; FIDO2 signing comes later.

**Decision:**

1. The Keychain branch is merged into main (2026-10-21, with D33).
2. Keys (D33): the key panel's "…" menu gets Remove, with a confirmation. When hosts or identities use the key, the confirmation lists them and warns that they lose it. The rule of D33 for creating a key (an explicit Import or Generate) is accepted.
3. Unrecognized keys (D32): a key imported by hand (the New key form) that is not recognized is never saved; the form shows the reason and keeps the text. Import Data of our JSON saves such a key as a key record of type "unknown", so nothing from the file is lost (data-model 9.1).
4. History in the export (D34, D38): Export Data gets two checkboxes, "Include command history" and "Include the connection log", both off by default. Their import is D51.
5. Settings on import (D41): D41 stays. Import Data gets the mode "Full restore from my backup", off by default: it applies every setting of the file, including those D41 leaves out. Turning it on shows a warning that the file may hold a script that will then run on the servers.
6. `~/.ssh/config` (D35): stays as it is: only the first alias of a `Host a b` line becomes a host, and SendEnv goes only into the host's notes.
7. Public keys per authentication (D36): the limit stays 4 and is not a setting.
8. FIDO2 signing (D37): on Windows only through the WebAuthn API; on macOS and Linux through `ctap-hid-fido2`. Built later, not in the next run.
9. and 15. `unsafe` code (D42, D48): allowed only in platform modules (`windows.rs`, `unix.rs` and the like); every `unsafe` block has a `SAFETY` comment and a test. `GetLogicalDrives` is allowed, so the drive list of a local file panel (D48) reads the drive letters without touching each drive. The rule is in `CONTRIBUTING.md`, section "Unsafe code".
10. Jump hosts (D44): the refusal stays until stage 5.
11. Export to host: built with the SFTP panels, over a shared service of SSH connections without a terminal tab, which the panels use too.
12. SFTP engine (D45): `russh-sftp` stays. Names that are not UTF-8 on the server (shown with U+FFFD, not openable) are a known limitation and are described as such in the documentation.
13. Links in folder transfers (D47): inside a copied folder, a link to a file is copied as the content of its target, and a link to a folder is skipped; at the end of the transfer the user sees the list of what was skipped.
14. `CODE_OF_CONDUCT.md`: the enforcement contact stays `iinfocertx@gmail.com`, the address the owner chose.

**Spec impact:** with the implementation: `docs/ui-behavior-spec.md` 1.4 (Export Data, Import Data) and 7.3 (Remove), `docs/export-format.md` section 4 and `docs/data-model.md` 9.1 and 18.4 (unknown keys, the full restore); now: `CONTRIBUTING.md` ("Unsafe code").

### 51. History and the connection log in Import Data

Status: accepted, 2026-10-21 (owner's choice; completes D50, item 4; amends D34).

**Context.** D34 has Import Data skip the command history and the connection log of a file, so that a backup never mixes another device's log into this one. With D50 both are parts of Export Data that are off by default, so a file that holds them was exported with them on purpose.

**Decision:**

- When the file holds command history or a connection log, Import Data shows "Import command history" and "Import the connection log" (each only when the file holds that part), both on by default. A part turned off is skipped and counted, as in D34.
- Imported history goes through the same secret filter as history recorded live (UI 4.2; `docs/security.md` 10.1, 10.2): an entry that the filter would not record is not imported.
- Device and vault records stay skipped (D34).

**Spec impact:** with the implementation: `docs/ui-behavior-spec.md` 1.4 (Import Data), `docs/export-format.md` section 4, `docs/data-model.md` 18.4.

## Run 2 of 2026-10-21

The decisions below were taken during the owner's second long run (its plan, section 1.1): the lead chose where the plan and the specification leave the details open, marked each one "run 2", and lists the disputable ones for the owner in the run report.

### 52. Deterministic UI tests: no motion, threads that wake the UI (run 2)

Status: run 2 decision, 2026-10-21 (plan, item 3.1: three flaky tests made deterministic).

**Context.** Three desktop tests failed now and then: `hosts::tests::empty_state_and_action_bar` and `hosts::tests::context_menu_runs_the_card_actions` (a click on a dialog's button had no effect) and `lock::tests::lock_and_auto_lock_keep_sessions_and_close_overlays` (the GPUI test scheduler's "not deterministic" panic). GPUI animations run on the real clock, not on the test executor's clock; a test click draws a frame after the pointer move, after the press and after the release, so a dialog that is still sliding into place moves its button between them on a loaded machine. Half of the UI test fixtures did not reduce motion. The lock test opens a local session tab whose terminal parses on its own thread and wakes its view from there, which the test scheduler reports whenever that happens while the view waits.

**Decision:**

- Every UI test fixture starts with `testing::init_ui` (GPUI Kit, the dark theme and reduced motion) instead of `gpui_kit::init` and `theme::install`; animations are off in tests. A test that checks an animation sets motion on itself.
- A test with a terminal that parses on its own thread allows parking (`cx.executor().allow_parking()`), as the session and pane tests already did; the other lock tests stay strict.
- Check: each of the three tests passed 50 runs in a row while two full desktop suites ran beside it. Before the change the Hosts test failed once in 20 runs under the same load; the lock test did not fail in 70 runs (it failed twice in about 30 during the night), so its cause is taken from the code: the pane of the local session creates its terminal, and with it the parser thread, even when the session layer is not installed, as in this test.

**Spec impact:** none.

### 53. Benchmark runs with an inactive or minimized window are invalid (run 2)

Status: run 2 decision, 2026-10-21 (plan, item 3.2).

**Decision:**

- While a measurement of `--bench-output` or `--bench-latency` runs, every painted frame checks that the window is active and shown, and so does every wait between frames (a minimized window draws none). The first lapse is kept; the run's output line ends with `(invalid: the window was not active)` or `(invalid: the window was hidden or minimized)` instead of the old check of the window at the end only.
- `scripts/measure-terminal.ps1` shows such a run as "invalid (… s)", prints a warning and leaves it out of "our terminal's cost".

**Spec impact:** none.

### 54. Import Data with history, the connection log and unknown keys (run 2)

Status: run 2 decision, 2026-10-21 (plan, item 3.3; how D50, items 3-5, and D51 are built).

**Decision:**

- Import Data opens the file first (`data_import_contents`: the number of history and connection log entries) and asks about them only when there are any; an encrypted file is then decrypted a second time by the import (about a second of Argon2id more, instead of keeping the plain text between the steps).
- An imported history entry is checked by the rules a stored command can still be checked by (`history::imported_command`): control sequences removed, then empty, a leading space, too long, masked echo, a password prompt in the line and the secret detector. What the terminal showed (the prompt, the alternate screen) is not in an entry, so those rules cannot be applied.
- A connection log entry keeps the members of data-model 13.3, each checked; its session recording (a file of the device that wrote it) and its team copy are not taken, and an entry still open in the file ends as `unknown`. The incoming entry is compared in that form, so a second import finds it again.
- A host link that names the exported personal vault is rewritten to the personal vault here.
- A key kept as "unknown" must have a valid label and fit in 256 KiB; it is stored exactly as in the file, members of later versions included.
- "Full restore from my backup" also turns "Apply the settings in the file" on (shown on and disabled).

**Spec impact:** `docs/export-format.md` section 4, `docs/data-model.md` 18.4, `docs/ui-behavior-spec.md` 1.4 (and `docs/ru`).

### 55. The drive list without touching the drives (run 2)

Status: run 2 decision, 2026-10-21 (plan, item 3.3; D50, items 9 and 15; amends D48).

**Decision:**

- The drive selection of the local file panel lists the letters `GetLogicalDrives` returns, in letter order, without touching any drive; before, each letter's root was checked with `is_dir`, which waits for a disconnected network drive. A drive that is not ready (an empty card reader) is listed and fails when it is opened, as any folder that cannot be read.
- The call is the only `unsafe` code of the new platform module `apps/desktop/src/platform/drives.rs`, with its `SAFETY` comment and a Windows test that calls it; `windows-sys` (already in the lockfile through `crates/private-file`) becomes a Windows dependency of the desktop crate with the feature `Win32_Storage_FileSystem`.

**Spec impact:** none (UI 5.3 does not say how the drives are found).

### 56. Links and unreadable names in folder transfers (run 2)

Status: run 2 decision, 2026-10-21 (plan, items 3.3 and 3.4; how D50, items 12 and 13, are built; amends D47).

**Decision:**

- A symbolic link inside a copied folder is decided by what it points to: a file is copied under the link's name as the target's content (`TreeSummary::links` counts them); a folder is left out; a missing or unreadable target is left out as a broken link. A lost connection or a timeout while the link is read ends the transfer as before.
- `TreeSummary::skipped` lists what was left out by its path inside the folder (`a/link`) with the reason: a link to a folder, a broken link, a special file (device, socket, pipe), or a name that is not UTF-8 on the server. The transfer UI of stage 4 shows that list at the end of the transfer.
- A name that is not UTF-8 on the server arrives in the listing with U+FFFD (russh-sftp decodes lossily, D45): such an entry that cannot be found under that name is left out of a download instead of ending it with "not found". The limitation is in the README ("Known limitations").

**Spec impact:** none yet (UI 5.6, the transfer queue, comes with stage 4).

### 57. Routes: jump hosts and the proxy in the SSH layer (run 2)

Status: run 2 decision, 2026-10-21 (plan, item 5.1 and 5.2; the core of stage 5 without its interface; replaces D44).

**Decision:**

- `shellpp-ssh` connects a route itself: `ConnectSpec::jump_hosts` are connected first, in order, each over a `direct-tcpip` channel of the one before; the first one over TCP, through `ConnectSpec::proxy` when one is set; the host last. Every connection of the route has its own spec and its own `Interaction`, so each host key is decided on its own address (TOFU per hop, the same known hosts rules, docs/security.md 11.1) and each hop's prompts are its own. The returned `Connection` owns the jump connections: disconnecting or dropping it closes the whole route; a dead hop ends the connections behind it.
- `SshError::place` says where it failed: the host, jump host N (0-based index of the chain) or the proxy. A jump host that refuses the channel (OpenSSH `AllowTcpForwarding no`) is `forwarding_refused` at that hop; a host the jump host cannot reach is `refused` at the host.
- The core expands the effective `jump_chain` (own, else inherited; `[]` stops the lookup; the host itself is never its own hop, as in D44) into the hops' own session configurations: effective port and credentials, known key types; their own chains and proxies are ignored, and a hop runs no session (no OS detection, no env). A hop that is not a live host of the vault, a hop named twice and more than 8 hops are a `route_problem`: the connection fails before it starts ("A jump host of this host is missing or listed twice"), never directly. The model already refuses a chain with a hop named twice when it is saved; the route check repeats that for data that bypassed it.
- Prompts of a jump host may not save (the session saves only for its own host); a jump host's typed password is asked again on the next attempt.
- The proxy is the host's own `ssh.proxy` (not inherited, data-model 8.1); its password is a record secret like `ssh.password`, so it is stored encrypted in the vault. HTTP uses `CONNECT host:port` with Basic authentication; SOCKS5 sends the host name for the proxy to resolve and authenticates with RFC 1929. The proxy's reply is read only up to its end (an HTTP reply over 8 KiB ends the attempt); nothing of it is kept or logged. New failure kinds: `proxy_failed`, `proxy_auth_failed`; a proxy that reports the host unreachable makes it the host's failure.
- The desktop's session view carries the place (`error_place`: jump host with its index, count and title, the proxy, or the chain); showing it is part of the stage 5 interface.

**Spec impact:** none (data-model 7.3, 7.5 and 8.3 are followed as written; D44 no longer applies).

### 58. Port forwarding in the SSH layer (run 2)

Status: run 2 decision, 2026-10-21 (plan, item 5.7, without the interface).

**Decision:**

- `Connection::forward` starts a rule: Local (a listener here, a `direct-tcpip` channel per connection), Remote (`tcpip-forward`; the server's `forwarded-tcpip` channels are accepted only for an active rule with that address and port, docs/security.md 11.4) and Dynamic (a SOCKS5 proxy here without authentication, CONNECT only; the requested name goes unresolved into the channel, so the SSH host resolves it). An empty bind address is `127.0.0.1`; `localhost` is `127.0.0.1` too.
- A local port in use is `PortBusy(port)` ("port N is already in use"); a port that needs elevated rights is `PortDenied(port)`; a remote listener the server refuses is `refused` (OpenSSH does not say why: its port in use, or forwarding not allowed).
- Stopping a rule closes its listener and its open connections (Remote: `cancel-tcpip-forward` too). A lost connection ends every rule on it with `ConnectionLost`.
- `HostForwarder` keeps the rules of one host on one shared connection (UI 8: several rules of a host use one SSH connection). After a drop it marks them `Reconnecting`, reconnects with the session reconnect backoff (1, 2, 4 … 60 s) and starts them again; a rule that cannot start (its port taken meanwhile) shows `Failed` with the reason. The connection closes with the last rule. The desktop service that owns these per host, autostart on connect and on launch (after unlock), and the rules from imported ssh config come with the forwarding screen.

**Spec impact:** none.

### 59. A reachable TEST-NET address on the dev server (run 2)

Status: run 2 note, 2026-10-21.

**Context.** On the dev server a TCP connection to `192.0.2.1:22` (TEST-NET-1, RFC 5737, never routed) is answered, by an SSH server even: the network routes or intercepts it. Two tests of "an address that never answers" relied on that address and failed there; one of them reached authentication with a made-up user and no secret.

**Decision:** those tests use a silence that does not depend on the network: the connect step is injected in the unit test, and the integration test uses a local listener whose accept queue is full (Linux and macOS drop further SYNs). On Windows the integration test keeps TEST-NET-1.

**Spec impact:** none.

### 60. Agent forwarding with the built-in agent (run 2)

Status: run 2 decision, 2026-10-21 (plan, item 5.3, without its interface; D5 "the built-in agent").

**Decision:**

- With Agent Forwarding on, the forwarded agent offers the Keychain's keys through the built-in agent: every stored private key of the vault that opens without asking (unencrypted, or with a saved passphrase). They are decrypted for the connection off the runtime's workers, signed with here and never sent; a lock wipes them at once, and none are offered while the app is locked (docs/security.md, sections 8 and 11.2). RSA keys sign with SHA-512 or SHA-256 as the request asks; a request for a SHA-1 `ssh-rsa` signature is refused.
- Two host options next to the switch, both off by default, stored in `host.ssh` (not inherited, like `agent_forwarding`): `agent_forward_system` also offers the system agent's keys (its listing merged after the built-in keys, its signatures relayed as before), and `agent_confirm` asks before every signature, naming the key's fingerprint.
- Until the confirmation dialog exists (stage 5 interface), a host with `agent_confirm` gets no forwarded signature at all: never one without the user's yes.
- Before this decision a host with Agent Forwarding forwarded the system agent; now it needs `agent_forward_system` for that.

**Spec impact:** `docs/data-model.md` 7.3 gains `agent_forward_system` and `agent_confirm` (with the interface).

### 61. Connections without a tab: forwarding rules and scripts on many hosts (run 2)

Status: run 2 decision, 2026-10-21 (plan, items 5.7 and 5.8, without their screens).

**Decision:**

- The session layer opens a saved host's route without a tab for forwarding rules and for a script on many hosts: its jump hosts, its proxy, its saved credentials, the agent and the key files (passphrases remembered for this app session). With nobody to ask, a host key is accepted only when Known Hosts holds it (an unknown, changed or revoked key fails), prompts are cancelled, nothing is saved and the agent is never forwarded. The TOFU dialog of such a connection comes with the stage 4 service "SSH connection without a tab".
- Forwarding rules of the personal vault run on one connection per host. Rules with `autostart_on_connect` start when a session to their host connects (its TOFU has saved the key by then); rules with `autostart_on_launch` start after unlock (D9). Team-vault rules never autostart (data-model 11). Locking does not stop running rules (sessions keep running too, UI 13.3); quitting does.
- A script on more than 4 hosts runs in exec channels, at most 8 at a time, with its timeout per host (UI 9.4); 256 KiB of output kept per host. The table, the confirmation for 2+ hosts and the split tab for up to 4 hosts are the interface's.
- The History tab's "Save" makes a snippet per (edited) command, targeting the host; a command the secret detector flags is left out; the hint is not shown again. Shell History lists every host's entries with a host filter (`history_search`).
- Imported `LocalForward`, `RemoteForward` and `DynamicForward` become rules with `autostart_on_connect` (OpenSSH starts them with every connection).

**Spec impact:** none (UI 8, 9 and data-model 11 are followed; the TOFU dialog without a tab is stage 4).

## The owner's answers to run 2

### 62. The owner's answers to the questions of run 2

Status: accepted, 2026-10-21 (owner's choice; the six questions in section 3 of the run 2 report). Keeps D60 and D61 as written; Import Data of snippets, snippet packages and forwarding rules is item 1 of the plan of run 3.

**Decision:**

1. Agent forwarding (D60): accepted. The forwarded agent offers the Keychain's keys through the built-in agent; the system agent only with the host option `agent_forward_system`. `ForwardAgent yes` in an imported `~/.ssh/config` turns the system agent on as well.
2. The startup snippet runs after every login of the session, a reconnect included: accepted.
3. Connections without a tab (D61) accept only host keys already in Known Hosts until the TOFU dialog of the stage 4 service "SSH connection without a tab" (plan, item 4.1) exists: accepted.
4. Import Data without snippets, snippet packages and forwarding rules: not accepted. Export Data writes them and Import Data skips them, so a backup does not come back whole, which breaks principle 4 (SPEC.md). Import Data imports these three record types by the rules of D34 (a conflict of ids keeps both versions, and references inside the import, such as a package's snippets and the hosts a snippet or a rule targets, point to the imported copy). It is item 1 of the plan of run 3; a round-trip test (Export Data, then Import Data) covers snippets, packages and rules.
5. Rules imported from `LocalForward`, `RemoteForward` and `DynamicForward` (D61) start on their own when a session to their host connects, as in OpenSSH (`autostart_on_connect`); they do not start on app launch (`autostart_on_launch` stays off). The import keeps its exception: a rule that listens beyond loopback is imported without autostart, with a note (security.md 11.3).
6. Locking does not stop running forwarding rules, as it does not close open sessions (UI 13.3). While the app is locked no rule starts: autostart on connect and on launch waits for unlock, and so does the restart of a rule whose connection dropped.

**Spec impact:** with the implementation: `docs/export-format.md` section 4 and `docs/data-model.md` 18.4 (the record types Import Data takes), `docs/ui-behavior-spec.md` 1.4 (and `docs/ru`).

## Run 3 of 2026-10-21

The decisions below were taken during the owner's third long run (its plan, section 1.1), marked "run 3"; the disputable ones are listed for the owner in the run report.

### 63. Import Data of snippets, packages and forwarding rules (run 3)

Status: run 3 decision, 2026-10-21 (plan, section 5; how D62, item 4, is built; amends D34).

**Decision:**

- Import Data takes `snippet_package`, `snippet` and `forwarding_rule` records by the rules of D34, after the hosts and before the history: each is checked by the rules of its save (`build_package`, `build_snippet`, `build_rule`); a conflict copy gets " (imported)" on its `name` (package) or `label` (snippet, rule) and the `import_key` `json:<exported id>`. A rule's `import_key` and the others' come back from the file as for hosts.
- References are rewritten as every other reference of an import: a snippet's package and target hosts, a host's startup snippet and a rule's host point to the imported copy when that record is a conflict copy and the naming record is new or a copy itself. A snippet or rule that is the same as here stays "already exists" with its references to the local records, even when its package or host became a copy (D34). The `missing_ref` check of a save is not applied: a reference to a record that is neither here nor in the file stays and reads as missing (data-model 3.4, principle 3).
- A forwarding rule from a file starts on its own only when the user connects to its host, and only when it listens on loopback, as the `~/.ssh/config` import (D62, item 5; security.md 11.3): `autostart_on_launch` is turned off, and `autostart_on_connect` too for a rule that listens beyond loopback (`0.0.0.0`, `::`, any non-loopback address). So a file from elsewhere never opens a port without a user's action: a Remote rule that starts on launch would expose a local service to its server after the next unlock. "Full restore from my backup" keeps the file's flags. A rule whose flags were turned off is compared without them: one that is the same here stays "already exists" with the local flags, and the next import of the same file finds the copy again. The summary counts the rules whose flags were turned off ("Forwarding rules imported with autostart turned off").
- A startup snippet comes with its host: it runs only on that host, which the same file defines, unlike the OS detection script that D41 keeps local because it runs on every server.

**Spec impact:** `docs/export-format.md` section 4, `docs/data-model.md` 18.4, `docs/ui-behavior-spec.md` 1.4 (and `docs/ru`).

### 64. Polish from the owner's review of the run 2 screenshots (run 3)

Status: run 3 decision, 2026-10-21 (plan, section 6).

**Decision:**

- Export Data was already encrypted with a password by default; the `export-data` screenshot showed the dialog after the switch was turned off, which read as the default. `export-data` now shows the dialog as it opens and `export-data-plain` the clear-text warning; a test checks the default.
- Generate key: the passphrase field has a show/hide button (the import form's too); once a passphrase is typed, "Repeat passphrase" appears, and Generate refuses two different values before any key is made.
- The core tells a public key from other text (`KeyProblem::PublicKey`): an OpenSSH line, also with `authorized_keys` options, an RFC 4716 block or a PEM `PUBLIC KEY`. The import says so and points to the file without `.pub`.
- The Keychain action bar: the search takes 200 to 320 px. When the bar cannot hold it at 200 px next to the labelled buttons (an open panel narrows the bar), Certificate and FIDO2 show only their icons, their names in the tooltips; the right group wraps below only when that is not enough either. The screen decides from its own width, as it does for the grid's columns.
- Certificate stays disabled; its tooltip says that certificates arrive in stage 4. FIDO2 opens the New key form as "FIDO2 key", which explains the file `ssh-keygen -t ed25519-sk` makes: the core reads such keys already (D37); creating one on a security key and signing with it come later (D50, item 8), as its tooltip says.
- A fingerprint is one line with a middle ellipsis (`SHA256:nTM…j9e5Q`), never broken at a `/`: in the key panel (with a new copy button), the Keychain list and Known Hosts. The TOFU and changed-key dialogs keep the whole fingerprint, since it is what the user compares (it fits their width on one line).
- History tab: one row per command, at its newest use (`history_list_distinct`), and deleting the row deletes every repeat. The "save as snippets" hint leaves out trivial commands: one simple command (no `;`, `&`, pipe, redirection or `$`) of `cd`, `ls`, `ll`, `la`, `l`, `dir`, `pwd`, `clear`, `cls`, `reset`, `exit`, `logout`, `history`, `whoami`, `top` or `htop`, whatever its arguments. "Save" works: each (edited) command becomes a snippet of the host (`history_hint_save`, run 2).

**Spec impact:** `docs/ui-behavior-spec.md` 4.2 and 7.1-7.3 (and `docs/ru`).

### 65. The SFTP tab (run 3)

Status: run 3 decision, 2026-10-21 (plan, section 7: stage 4, the interface of SFTP; UI 5, 7.3).

**Decision:**

- The shared service of SSH connections without a terminal tab (item 1) is an SFTP session of the session layer (`SessionManager::open_sftp`): the route, the credentials, the prompts, the first connection and changed key dialogs and the connection log of a terminal session, then the `sftp` subsystem instead of a shell. A remote panel shows its connection, its questions (the same prompt form and host key dialogs as a terminal tab) and its failure with Retry or Reconnect inside the panel. An SFTP session does not reconnect by itself: a dropped connection shows the reason with Reconnect. The panel's session ends with the panel (another source, Close, a closed tab).
- Connect via SFTP opens a session tab with the same two panels, Local on the left and the host connecting on the right (UI 5.1); the SFTP tab of the top bar stays for manual work.
- Dates in the table are `YYYY-MM-DD HH:MM UTC` (the format of Known Hosts) until the app formats dates in the local time zone; the column widths are fixed (resizing them comes later).
- Transfers: upload, download, between two hosts (files; a folder between two hosts is refused with a reason for now) and local copies (both panels local). A transfer that fails (a dropped connection included) shows its reason with Retry, which continues from its `.part`; Pause stops it the same way. Folders go with everything inside by D56, and the row of a finished folder counts what was left out.
- Open: a remote file is downloaded to `tmp/` (storage-format 9), opened in "Program for Open" or the system's program, and a notification says the copy is not encrypted. The copy is checked every 2 seconds; a saved copy is uploaded with a notification; when the server's file changed meanwhile, a dialog offers to overwrite it or keep the server's file. Open with asks for the program in the system's open dialog.
- The drive selection of a local panel on Windows (D55): a click on the drive crumb shows the drives under the path bar.
- Export to host (item 8) runs over connections without a tab (D61: saved credentials, the agent and key files, only a host key Known Hosts holds), at most four hosts at once; a host whose key is not saved yet is reported as "Connect once to save its host key first".
- Certificate (item 9): the Keychain's Certificate adds a `*-cert.pub` to the key it certifies (UI 7.1, 7.3; see the UI text).
- After the review of the stage (security.md 11.4): a name from a server with a separator, `.`, `..`, a control character or surrounding spaces is not listed, so it never reaches a local or remote path; names are shown with control and bidirectional override characters made visible (`<U+202E>`); on Windows a downloaded name the file system cannot hold gets the suggestion of D46. Open of a file the system would run (`.exe`, `.bat`, `.ps1`, `.lnk`, `.msi`, `.sh`, `.desktop` and the like) asks first, naming the type; an opened copy gets the Windows Mark of the Web. A local copy onto itself or of a folder into itself is refused; a local file is copied through `.part` and stops on Pause and Cancel; a local Rename never replaces an existing item. "Apply to all" answers the conflicts of one operation only; the conflict dialog cannot be dismissed without an answer. A transfer takes its host's channel when it starts, so Retry after Reconnect works; an opened copy uploads through the channel of the moment. `sftp.confirm_delete` off deletes without asking; Edit Permissions keeps setuid, setgid and sticky. A folder transfer's row lists what was left out (D56) in its tooltip.
- Known limits: an opened copy is removed when the app exits (`tmp/` is emptied), not when its program closes it; a save in the last two seconds before the tab closes is not uploaded.

**Spec impact:** `docs/ui-behavior-spec.md` 7.1, 7.3 (and `docs/ru`); UI 5 is followed as written otherwise.

### 66. The interface of stage 5 (run 3)

Status: run 3 decision, 2026-10-21 (plan, section 8: stage 5, the interface; UI 3.1, 8).

**Decision:**

- A route failure names its place (D57): "On jump host 1 of 2: bastion", "At the proxy", or that a jump host of the chain is missing or listed twice; the jump chain editor and the proxy fields of Show more were built in run 2.
- Agent Forwarding (D60): while the switch is on, the host panel shows the warning (anyone with root on the host can sign in elsewhere with your keys while a session is open) and two options: "Also offer the system agent's keys" (`agent_forward_system`) and "Ask before every signature" (`agent_confirm`). With the second, every signature the server asks of the forwarded agent becomes a question of the session (`PromptKind::AgentSignature`): the tab shows the key's SHA256 fingerprint with Deny and Allow; Enter and Esc deny, only Allow signs, and no answer (the tab closed, another question open, the app locked) refuses. An SFTP session forwards no agent: it opens no shell. After the review: a signature never takes the place of a question already shown (it is refused at once then), each request is its own dialog even for the same key, and the answer goes to the request on screen; a question waits at most two minutes, and ends with the shell (an end or a drop refuses it).
- The Port Forwarding screen (UI 8): split New forwarding (the main part opens a Local rule, the arrow offers Local, Remote and Dynamic) and search; the cards with the type badge (L, R, D), the label (the summary when there is none), the summary and the status (Stopped, Starting, Running, Reconnecting with its attempt, or the error in words: a busy port names the port and asks for another, UI 8.4). A click opens the rule in the panel, a double click (and Enter) starts or stops it, the menu has Start/Stop, Edit, Duplicate and Remove; Delete removes, Esc closes the panel.
- The panel is the full form of 8.4 for a new rule, a copy and a saved rule: type, host, label, where it listens and where it leads (each address with its port), and the two autostart switches; a saved rule's status with Start or Stop is above it, and Create, Create and start, or Save and Remove stay in the panel's footer below the long form. Ports are checked by the form (1 to 65535), everything else by the core, each reason under its field. Saving a rule that runs starts it again with the new values. Duplicate opens a new rule with the values and "<title> Copy"; nothing is saved until Create. Remove asks first, stops the rule, and offers Undo for 5 seconds.
- The bind address defaults to 127.0.0.1; any address beyond loopback (0.0.0.0, ::, an address of a network) shows the warning under it, and for Remote it names `GatewayPorts`. The explicit choice of 8.4 is typing such an address.
- The screen shows the rules of the personal vault (the only one whose autostart flags count, data-model 11). Statuses live in the session layer's forwarding service; the screen reads them once a second while a rule is started. `SessionManager::forward_start` and `forward_stop` start their work at once and return a future without a borrow, so the interface can wait for them on its own executor.
- After the review of the screen: the screen reads the statuses each time it is drawn, so a rule that autostarted elsewhere shows as running, and Remove and Save act on the session layer's status, not only on the rules this screen started. Remove always stops the rule; Stop and Remove while a start is still connecting stop it again when the start ends; a rule whose start failed is stopped, so a later reconnect of its host cannot bring it up on its own, and its card keeps the reason; Save during a start starts the rule again with the saved values once the start ends. A start cancelled that way shows no error.
- Not built in this run: the step-by-step wizard of 8.3 with its diagrams (the form is what its Skip wizard leads to), the tile/list toggle and sorting of 8.1, and Move to / Copy to another vault in the menu (there is one vault in local mode).
- The Snippets screen (UI 9): split New snippet (the arrow: New snippet package) and search; packages at the top as folders (a click opens one, with "Snippets › name" to go back; Open, Rename and Remove in its menu; the snippets of a removed package stay, outside any package), snippet cards below (the label, else the first line of the script, and the number of targets). A search looks through every package. A click opens the snippet in the panel, a double click (and Enter) runs it; the menu has Run, Edit and Remove with Undo.
- The snippet panel (9.3): description, package, tags, the script (monospaced, multi-line; its `{{name}}` variables listed under it) and the targets with Add targets (a checklist of the saved hosts). Create or Save and Run stay in the footer; Run saves first and is disabled without a script or a target. A snippet without targets opens in the panel instead of running.
- Run (9.4): the values of the variables (one field each), then, for two or more hosts, the confirmation that lists them, then the results table: per host the status (waiting, running, success, error, timed out), the exit code or the reason, and on click the output (control characters, escape sequences and bidirectional overrides shown as `·`). Every run, whatever the number of hosts, goes through exec channels (`run_on_hosts`: saved credentials and known host keys only, 8 hosts at once, 60 seconds each) and shows the table in a dialog.
- A host's startup snippet with `{{name}}` variables (3.1): after each login the session layer reports it in the session's view (`startup_ask`, counted per login) instead of skipping it; the session's pane, once it is the active pane with no other dialog, asks the values in the form of the Snippets screen and sends the filled script with Enter as injected input. Cancel skips it for that login. After the review: the request ends with its login (a new attempt, a drop or an end clears it), and the values are sent only while that login's shell still runs in the pane; a sidebar snippet goes to the pane it was clicked for, even when another pane becomes active while its values are typed.
- The Snippets tab of the terminal sidebar (4.2): the vault's snippets with a search; a click puts the snippet at the prompt of the active pane, the run button sends Enter after it; variables are asked first. Both are injected input (never recorded in history). A script of several lines goes as one bracketed paste; without bracketed paste each line would run as it arrives, so only Run takes it and a click says so in a toast.
- Shell History (9.1): the Shell History button of the Snippets screen shows, instead of the snippets, the commands of every host (newest first, the 500 newest) with a search and a host filter; each row has "Save as snippet", which opens a new snippet with the command as its script and its host as the target. The entries are what the history capture kept (4.2), so its rules hold; nothing is logged. "Snippets › Shell History" leads back.
- After the review of the screen: Enter on the confirmation closes it before the results open (it used to close the results and leave the confirmation for another run); the run takes the hosts the confirmation listed; Run from the menu of the snippet open in the panel saves it first, so what runs is what is on screen; selecting the open snippet again keeps its form and what is typed; hosts and a package removed since are left out of the form, so Save is not refused; a snippet whose hosts were all removed opens in the panel instead of running on no host.
- The snippet panel saves with Create and Save, not on every change as UI 1.3 says for the panels: a run must use a saved script, and a script is often typed in several goes. Changes not saved are dropped when another card opens.
- Not built in this run: the split tab for up to four hosts and the results in a tab (9.4; the dialog shows the same table), tags as chips with autocomplete (one comma-separated field), syntax highlighting of the script, the placeholder card of a draft (9.2), the Vault block and Share this snippet (one vault in local mode), the search in Add targets, Undo of a package removal (its snippets stay), the script generation hint (9.5, a later stage), and the tile/list toggle and sorting.

**Spec impact:** `docs/ui-behavior-spec.md` 3.1 and 4.2 (and `docs/ru`); UI 1.3 (the snippet panel's explicit Save), 8 and 9 are followed as written otherwise, with the parts above left for later.

### 67. The Logs screen (run 3)

Status: run 3 decision, 2026-10-21 (plan, section 9: stage 6, item 1; UI 11).

**Decision:**

- The core lists the connection log (`connection_log_list`: every entry, newest first, with the host's title now or the snapshot, its end, the bookmark and whether it ran on this device) and sets the bookmark (`connection_log_set_saved`).
- The Logs screen is the table of 11.1: Date (the start date; start and end times below, `(+1d)` when the session ended on a later day, "now" while it is open, "unknown" after a crash), User (the device: "This device" in local mode, before accounts in stage 8), Host (OS icon, title, protocol and remote user) and Saved (the bookmark). The Date header turns the order; the Saved header shows only the saved entries. Times are UTC, as elsewhere until the app formats dates in the local time zone (D65).
- A click opens the connection's details on the right (host, address and port, remote user, protocol, start, end, how it ended, device). Session output is not recorded yet, so the details say so; the recording viewer and the export to .txt of 11.2 come with the recordings.
- With the Logs screen every section of the sidebar has its screen; the "In development" placeholder of the sections is gone.
- Retention (12.6): when the log is read (the Logs screen), this device's entries whose end (or start, while open) is older than `logs.retention_days` and that are not saved are removed. "Clear the log" removes every entry of this device, saved ones too, after a confirmation; the end of a session whose entry was cleared is not written back.
- Settings (12.1): Account & Sync without a server shows "Connect sync server" disabled (sync arrives in stage 7) and Export Data / Import Data; SFTP has hidden files (the starting choice of new file panels), delete confirmation, the answer to name conflicts, parallel transfers (1, 2, 3, 4, 6 or 10) and the program for Open; Logs has the retention period (7, 30, 90, 180 days or a year), "Record session output" disabled until the recorder exists, and Clear the log. SSH ID and Team are hidden until accounts and teams; Updates keeps the "In development" placeholder.
- After the review: "Clear the log" removes every personal entry, whatever device wrote it (an imported log too), as data-model 13.3 says; the list hides other devices' expired unsaved entries; a new retention period makes the Logs screen read the log again at once; "Show hidden files" is the starting choice of each source a file panel opens (a local folder, a host), not only of new panels.
- Shortcuts (12.5): the table lists the app's own actions (go to Vaults, SFTP and tabs 3-9, new local terminal, lock, new serial connection, new tab, close tab or pane, next and previous tab, search hosts and tabs, Settings, and the Hosts screen, unassigned by default) with their keys. A click on the keys records new ones (Esc ends the recording, not the window); × leaves the action unassigned; ↺ and Reset all bring the defaults back; actions that share keys are marked in `--danger`. A change is written to `shortcuts.overrides` and bound at once over the defaults. Keys of the terminal (copy, paste, select all, splits, focus between panes, search in the terminal) keep their defaults; the PC/Mac set selector is not shown (the platform's command key is used). A key the app binds over dialogs (the lock's) cannot be recorded while it is still bound: it acts instead. After the review: an action's old keys are unbound for that action only (`Unbind`), and every action's keys are bound again after them, so swaps, resets and resolved conflicts work in any order and survive a restart; unassign then reset brings the default back; recording takes only keys with Ctrl, Alt, Cmd or Fn, or an F-key (a plain letter would fire while typing); keys compare in one canonical form, and the terminal's fixed keys (copy, paste, select all, splits, focus between panes, search) count as conflicts.

- Keyword highlighting (12.4): on by default (`terminal.highlighting_enabled`, a switch in Settings → Terminal). Words of Error (error, fail, failed, fatal, denied, refused, panic, critical, exception...), Warning (warn, warning, deprecated), OK (ok, success, succeeded, passed), Info (info, notice) and Debug (debug, trace), and IPv4 and MAC addresses, are drawn in their category's color, one for a dark and one for a light terminal background (the theme's background decides). Only text in the terminal's default color is recolored, so a program's own colors stay; never in the alternate screen; only when a row is laid out anew (the row cache keeps it, so the cost is per new row). The colors of the categories cannot be changed yet, and custom regex rules (the `highlight_rule` records) have no editor yet. After the review: only plain text is recolored (the default text and background colors, not reversed, dimmed or hidden), so a search match of `less` or a red background stays readable; an address inside a longer run (a hex dump, a fingerprint, an OID, a four-part version) is not one; the pattern uses ASCII word boundaries and digits.

**Spec impact:** none; UI 11, 12.2, 12.4 (highlighting) and 12.6 are followed as written, with the recordings of 11.2 and 12.6, the category colors and the custom rules of 12.4 left for later.

## The owner's answers to run 3

### 68. The owner's answers to the questions of run 3

Status: accepted, 2026-10-21 (owner's choice; the seven questions in section 3 of the run 3 report). Amends D65, D66 and D67 where they differ; items 1 to 7 of the plan of run 4.

**Decision:**

1. The snippet panel keeps its explicit Create and Save (D66). Leaving it with changes not saved (opening another card, going to another screen, closing the panel) asks "Save", "Discard changes" or "Stay"; while it has such changes, the panel's header marks the snippet "Not saved". Run from the panel with changes not saved asks "Save and run". UI 1.3 records the snippet panel as the exception to saving on every change. This replaces D66's "Changes not saved are dropped when another card opens".
2. A run on one to four hosts keeps the results table as the default, as on more hosts. The split tab comes with Broadcast (stage 9); UI 9.4 says so.
3. The signature question of a forwarded agent (D60, D66): the tab that waits for the answer shows a key icon in the warning color, and when the app's window is not focused a system notification names the host. The question still waits at most two minutes.
4. Retention of the connection log (D67, UI 12.6) applies when the app starts and every 24 hours after, not only when the Logs screen reads the log.
5. A snippet of several lines in a shell without bracketed paste still goes only by Run (D66); the insert button is disabled, and its tooltip gives the reason.
6. Dates are shown in local time from now on, everywhere (Logs, Known Hosts, SFTP, Import Data), in the format of the system's locale; a tooltip gives the full time in UTC. This replaces the UTC dates of D65 and D67.
7. The hang of `scripts\ui-screens.ps1` on `new-tab-search` in a full run is looked into for an hour at most. A cause in the app (a leak, a lock after many sessions) is fixed, with a test; a cause in the harness gets a timeout and one retry per state, and the cause goes to `CLAUDE.md`.

**Spec impact:** with the implementation (run 4): `docs/ui-behavior-spec.md` 1.3 (the snippet panel's exception), 3.1 (the signature question), 4.2 (the insert button), 9.4 (the split tab with Broadcast), 12.6 (when retention applies) and the dates of 2.5, 5, 10 and 11 (and `docs/ru`).

## Run 4 of 2026-10-21

The decisions below were taken during the owner's fourth long run (its plan, section 1.1), marked "run 4"; the disputable ones are listed for the owner in the run report.

### 69. Changes not saved in the snippet panel (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 4.1; how D68, item 1, is built).

**Decision:**

- A change is a difference from the saved values: the description and tags as they would be saved (trimmed; tags split at commas), the script as typed, the package and the targets. Typing the saved value back is no change. A new snippet untouched is no change; one made by "Save as snippet" (Shell History) is, since nothing of it is saved yet.
- Leaving asks on another card (a click, Edit in a menu, the arrow keys), New snippet, Save as snippet, the collapse button and Esc, and another section of the sidebar (a click, the Hosts shortcut). Switching to a session tab or the SFTP tab does not ask: the Snippets screen keeps its panel as it is, and the terminal is where a script is often tried before it is saved. Locking the app drops the panel without asking, as before.
- "Save" leaves only when the snippet was saved; a refusal keeps the panel with its reasons. "Discard changes" brings the saved values back (a new snippet's panel closes), then leaves. "Stay", Esc and a click outside keep everything. Enter is Save.
- Run in the panel, and Run in the menu of the snippet open in the panel, ask "Save and run" only when there are changes; without them Run runs the saved snippet at once.

**Spec impact:** `docs/ui-behavior-spec.md` 1.3 (and `docs/ru`).

### 70. The signature question while the user looks elsewhere (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 4.3; how D68, item 3, is built).

**Decision:**

- The key of a waiting signature question sits on the tab after the state dot, in `--warning`, with the tooltip "Waiting for your answer: sign with your key?". The pane that asks shows the dialog itself, so a split needs no mark of its own.
- The system notification goes through GPUI's notifications (the Windows toast, macOS and Linux notification centers): no new dependency. Its title names the host as the tab does, its body says what to do; the fingerprint stays in the app. "Not focused" is: the app's active window is not the one the pane was drawn in (no window drawn yet: no window of the app is active).
- Windows needs the app's identity for a notification (`Shellpp.Desktop`), which it registers for the user (HKCU) at the first one. A test run (`SHELLPP_TEST=1`) and a temporary profile (`--demo`) set no identity, so they show no notification and write nothing to the system; the GPUI tests check the notification on the test platform.
- A click on the notification brings its window forward with the tab, and the pane that asks becomes the active one. The answer, the end of the question (a drop, the timeout) and a new question replace or withdraw it.

**Spec impact:** `docs/ui-behavior-spec.md` 3.1 (and `docs/ru`).

### 71. Dates in local time (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 4.6; how D68, item 6, is built; replaces the UTC dates of D65 and D67).

**Decision:**

- One module shows every date (`apps/desktop/src/dates.rs`): it reads the records' RFC 3339 UTC timestamps and writes the short date and the time without seconds. On Windows the zone comes from `GetDynamicTimeZoneInformation` and `SystemTimeToTzSpecificLocalTimeEx` (the daylight saving rules of each date, also of past years), the format from `GetDateFormatEx` and `GetTimeFormatEx` with the user's locale (a platform module, `platform/local_time.rs`; `windows-sys`, already a dependency, with three more features). No time zone crate is added.
- Elsewhere, until the platform's zone is read (macOS and Linux builds come later), times stay in UTC and say so ("18:04 UTC"); `time`'s local offset is not used, as it cannot be read safely in a program with threads on Linux.
- The places: the Logs table (the start date; start and end times; "(+1d)" counted by local dates) and its details, Known Hosts (table, tiles and panel), SFTP (the Modified column and the name conflict dialog), the Keychain's certificate validity and Shell History. Each date's tooltip gives its full UTC time. Import Data and Export Data show no dates (checked); exports keep the UTC timestamps as they are.
- Tests fix the zone and the format per thread (`set_fixed_display`: dates `21.10.2026`, times `18:04`), so their expectations do not depend on the machine; the Windows calls have their own tests (a zone with fixed daylight saving rules, the `de-DE` and `en-US` formats).

**Spec impact:** `docs/ui-behavior-spec.md` 1.5, 1.6 (new) and 10.1 (and `docs/ru`); CONTRIBUTING.md (the new platform module).

### 72. The screenshot run's limits are per state (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 4.7; D68, item 7).

**Decision:**

- The cause was in the harness, not in the app: the 600 s watchdog of `shellpp --screenshots` counted from the start of the whole run, and a full run of every state (146 files) takes longer since run 3 added its states. It ended the run wherever it was at that moment, twice at `new-tab-search`, which a short run captures at once. No leak or lock after many sessions was involved: the same states pass in a run that starts later in the list.
- Each state now has `STATE_TIMEOUT` (120 s) per theme and is tried once more, from the demo data, when it fails or runs out of time; the run says so ("trying once more"). The watchdog ends the run only after 300 s without any state begun (the app's thread stuck, where no timeout of its own can fire). `scripts/ui-screens.ps1` waits up to an hour for the launch (was 900 s).
- `CLAUDE.md` records the cause under "Подводные камни".

**Spec impact:** none.

### 73. The Port Forwarding wizard (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 5.1; UI 8.1, 8.3; completes D66).

**Decision:**

- New forwarding (its main part, and the button of the empty screen) opens the wizard in the details panel; its menu still opens the full form of a Local, Remote or Dynamic rule directly (8.1).
- The wizard is the full form shown one question at a time, so its values are the form's: "Step N of M" and the question above the fields of the step. Steps: the type (the Local / Remote / Dynamic segments, the diagram and the explanation, and a note that the rule goes to the personal vault, the only vault in local mode), the host, where it listens (with the warning of an address beyond loopback, naming `GatewayPorts` for Remote), where it leads (not for Dynamic), and the label with the rule in one line (as its card will show it) and Create or Create and start.
- The footer: Back on the left from the second step; Skip wizard on the first step and Continue on every step but the last. Continue checks the fields of its step (a host chosen, ports 1-65535, a destination); Enter in a field is Continue. Back keeps everything typed. Skip wizard shows the full form with the values so far. When the core refuses a value at Create (an address it does not take, say), the step of that field comes back with the reason under it.
- The diagram is drawn by the app (`forwarding/diagram.rs`): Lucide icons for this computer, the firewall, the SSH host and the service or the internet, arrows the way the forwarded connections go, and the participant that holds the port marked ("port", "SOCKS").

**Spec impact:** none; UI 8.1 and 8.3 are followed as written.

### 74. Tag chips and the highlighted script of a snippet (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 5.2; UI 9.3; completes D66).

**Decision:**

- Tags are chips, as in the host panel: Enter or a comma adds the typed tag (with the host tags' rule: trimmed, at most 64 characters, no commas; one that differs only in case is not added twice); while something is typed, the matching tags of the vault's other snippets are offered, and "Create tag" when none is that tag. × removes a chip; Backspace in the empty field removes the last one. A tag still in the field when the snippet is saved is saved too.
- The script field is GPUI Kit's code editor (`Editor`) with the app's own shell highlighting (`snippets/shell_highlight.rs`) through its parser-independent `InputHighlighter`: comments, quoted strings, `$variables` and `${...}`, the snippet's `{{name}}` variables (also inside double quotes, in the theme's "embedded" color), shell keywords (`if`, `then`, `for`, `do`, `case`...), numbers and operators. No grammar crate: GPUI Kit's `tree-sitter` features are not exposed through `gpui-kit` (depending on `gpui-component` directly would break GPUI Kit's rule that an application depends on `gpui-kit` alone) and would build C grammars. Highlighting only colors the text; the script is stored and run as typed. The editor shows no line numbers and no folding, six lines high.

**Spec impact:** none; UI 9.3 is followed as written.

### 75. Session output recording and its viewer (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 5.3; UI 11.2, 12.6; storage-format 8; D16, D20).

**Decision:**

- The core writes and reads `recordings/<file_id>.shpprec` exactly as storage-format 8.1-8.3 says (`crates/core/src/recording.rs`): the 63-byte header as AAD, 64 KiB chunks sealed with XChaCha20-Poly1305 under `recording_key[file]` (HKDF of the personal vault key and the file id, D16), nonce = the random 19-byte prefix ‖ be32(index) ‖ the last flag. `aead` 0.6 no longer carries its STREAM module, and the construction is fixed by the format, so the core builds the nonces itself over the AEAD it already uses; no crate is added. Tests: round trip with a UTF-8 character cut between two outputs, several chunks, a file without its final chunk (read as incomplete), and a wrong key, a changed byte, a changed header, a cut or dropped chunk (all refused).
- A session is recorded when its host says so ("Record output" On or Off), else when `logs.record_output` is on (off by default). The connection log entry names its `recording_file_id` first, then the file is created (0600 on Unix, in the data directory). The session layer gives the decoded output, and only output, to the writer as it reaches the terminal; the writer keeps its own key, so output that arrives while the app is locked is still recorded; the file is closed (its final chunk written) when the session's output ends. A file that cannot be written stops the recording, not the session.
- Recordings go with their entry: retention and "Clear the log" remove the entries first and their files second.
- Settings → Logs: "Record session output" is a working switch; turning it on asks first ("Record session output?", naming passwords, keys and tokens a program prints); turning it off does not ask.
- Logs: the details of a recorded entry say so and offer "Open recording". The viewer is a dialog over the screen: the output as plain text (escape sequences and control characters removed), a search over its lines with the number that match, "Incomplete recording" for a file that was not closed, and "Export to .txt", which warns that the file is not encrypted before it asks where to save it. A recording that is not on this device says so. The file is read and decrypted off the app's thread.
- Not built: the listing of recordings without a log entry (storage-format 8.4), re-encryption at sign-in (stage 7), the background scan for files whose entry is a tombstone (no tombstones are kept in local mode yet: entries are removed outright, and their files with them).

**Spec impact:** none; UI 11.2 and 12.6 and storage-format 8 are followed as written, with the parts above left for their stages.

### 76. The sets of keys and the terminal's keys in the Shortcuts (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 5.4; UI 12.5; completes D67).

**Decision:**

- The Shortcuts section starts with the set of keys, PC or Mac (`shortcuts.preset`; Mac on macOS, PC elsewhere by default). Each action has its keys in both sets: PC as UI 12.5's table, Mac with Cmd in place of Ctrl and the macOS habits for the terminal and the tabs (Cmd+C/V/A, Cmd+D and Cmd+Shift+D, Cmd+F, Cmd+W, Cmd+Shift+] and [). On Windows the Mac set's Cmd is the Windows key, which the system keeps for many keys; it is offered for people used to it.
- The terminal's keys are in the table under "Terminal": copy, paste, select all, the two splits, the four focus moves between panes and the search in the terminal. They are recorded, unassigned and reset like the app's, and conflicts are checked among all of them (the fixed list of D67 is gone). Tab and Shift+Tab to the shell are not in the table: a key without a modifier cannot be recorded.
- The user's own keys (`shortcuts.overrides`) stay when the set changes. Every change, reset and change of set rebinds every action at once: for each action, the keys either set gives it and the keys bound last are unbound for that action only in its key context, then its current keys are bound after them.

**Spec impact:** `docs/ui-behavior-spec.md` 12.5 (the Mac set; and `docs/ru`).

### 77. The user's keyword highlighting rules and category colors (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 5.5; UI 12.4; data-model 14.2; completes D67).

**Decision:**

- The core keeps `highlight_rule` records (type `0x0902`, class appearance, personal vault): a custom rule (`pattern`, `case_insensitive`, colors, `enabled`) or a category's colors and on/off. A pattern must compile in the terminal's own regex engine (the `regex` crate: linear time, no backtracking, so no rule can stall the terminal; its syntax is close to ECMAScript's, which data-model 14.2 names, without look-around and backreferences), within a 256 KiB compiled size, and must not match the empty text; 512 characters at most; 64 custom rules at most. Colors are `#RRGGBB`. Export Data writes the records and Import Data takes them back (D34's rules).
- The terminal applies the custom rules first, in creation order, then the categories that are on, each in its own colors for a dark or a light terminal background; where two match the same text, the earlier wins. As before, only text in the terminal's default colors is recolored, never in the alternate screen. The rules are one set for the app; a new set has a new generation, which is part of the row cache's key, so rows are laid out again at once.
- Settings → Terminal, under "Keyword highlighting" (shown while it is on): each category with its color and on/off, then "Your rules": the pattern, "Any case" and the color, on/off and remove for each, and a field with "Add rule" (Enter adds too). Colors are chosen from a palette of eight pairs (a shade for dark and one for light backgrounds: the six categories' own, violet and orange), not typed, so a rule is never unreadable on one of the two backgrounds. An expression the checks refuse leaves its reason under the field.

**Spec impact:** none; UI 12.4 and data-model 14.2 are followed as written (the palette is the "color selection").

### 78. Interface zoom waits for the owner's choice (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 5.6; UI 1.4): blocked, with the options for the owner.

**Decision:**

- Zoom In, Zoom Out and Actual Size stay "In development". GPUI has no window zoom: the window's scale factor comes from the display, and its override (`Window::set_scale_factor`) exists only in test builds. The app's sizes are the design system's absolute pixels (`theme::metrics`, 47 values used at about 1 400 places, besides the `px()` of each view), so a zoom cannot be put on top of them.
- The options, for the owner: (a) the design system in rems: `theme::metrics` becomes rem-based and `Window::set_rem_size` zooms the whole window (GPUI Kit's own components follow rems already); a refactor of every view, its tests and screenshots, done in one stage with the design system's review; (b) a patch of GPUI's window scale (a fork of `gpui-pre`, against the one-dependency rule of GPUI Kit and the license and update path of D4); (c) zoom of text only (the metrics of font sizes), which leaves controls and spacing as they are and breaks the layout's proportions. Recommendation: (a), in its own stage.

**Spec impact:** none until the choice. (Run 6: the owner chose (a) for the run's plan, section 7; closed by D96.)

### 79. "If time remains" of run 3 against the owner's section 9 (run 4)

Status: run 4 decision, 2026-10-22 (plan, section 7; the owner added section 9 during the run, after section 7, and named 9.1 the most important). Disputable: listed for the owner.

**Decision:**

- 7.1 (a compact scrollback: the emulator keeps the screen and about 1 000 lines, older lines go to a journal of text and attribute runs that the renderer, the search and the selection read by absolute line) changes the terminal's rendering, search and selection together; estimated at four to six hours with its tests and measurements, it is not started tonight. 7.2 (serial connections) and 7.3 (Telnet) are SPEC stage 9 features of their own (a new session kind each, its connection dialog, its tests without real hardware or servers); about two hours each. Started under the 45-minute rule of section 1.4, each would end as a `wip/*` branch; so they are left whole for their stage, and the night's time goes to section 9.
- 7.4 (keyboard navigation and visible focus on every screen) is done as far as an audit of the screens and its fixes take within the hour.

**Spec impact:** none.

### 80. Legacy algorithms per host and the errors of a mismatch (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 9.1 [decision of the owner]; closes the open item "per-host opt-in for legacy SSH algorithms" of docs/security.md, section 18).

**Decision:**

- A host setting `ssh.allow_legacy_algorithms` (bool, off by default), "Allow legacy algorithms" in Show more of the host panel, with a warning that stays shown while it is on. On, the connection also offers the SHA-1 key exchanges (`diffie-hellman-group14-sha1`, `diffie-hellman-group-exchange-sha1`), the SHA-1 MACs (`hmac-sha1-etm@openssh.com`, `hmac-sha1`), `ssh-rsa` (SHA-1) host key signatures and the AES-CBC ciphers, each after all modern ones of its list, so a server that has a modern one still gets it; an RSA user key signs with SHA-1 only for a server that accepts nothing else (its `server-sig-algs` has no `rsa-sha2-*`, or it sends no list, as OpenSSH before 7.2). A jump host follows its own setting. Quick connect has no host, so it never offers them. 3DES, RC4, Blowfish, `ssh-dss`, the 1024-bit `diffie-hellman-group1-sha1` and the MD5 MACs are not offered at all.
- When a negotiation finds nothing in common, the failure is `no_common_algorithm` with the negotiation (key exchange, host key, cipher, MAC, compression) and what the server offered. Only the names of a fixed table of known algorithms are kept (as bits of a `Copy` value), the others are counted: as for every failure, no server text is kept or shown (docs/security.md, section 9.1). The failure screen shows the reason, "The server offers: …", "Shell++ supports: …" (with the legacy names when the connection offered them) and, when the server offers one of the legacy algorithms that were not offered, the way to turn the host setting on (of the jump host when the failure names one; Quick connect never offers them). Show more opens by itself for a host with the setting on, so its warning is in view.
- The compatibility matrix (docs/compatibility.md) runs in Docker: the latest OpenSSH, OpenSSH 7.4 (CentOS 7), Dropbear (Alpine), and the OpenSSH variants legacy-sha1, kbd-2fa, no-sftp and motd-banner of tests/openssh, each in its own container (`crates/ssh/tests/compat.rs`, also in CI).

**Spec impact:** none; docs/security.md (section 11.2 and the open item) and docs/data-model.md (the host's `ssh` fields) are updated.

### 81. Broadcast input and workspaces (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 9.3; UI 4.4, 4.7; D68 item 2). Disputable: listed for the owner.

**Decision:**

- Broadcast input works: the tab menu item of a tab with splits turns it on ("Stop Broadcast Input" while on). It takes the panes whose sessions run at that moment (with fewer than two, a toast says it needs two connected panes); a pane whose session stops running (a drop: a reconnect is a fresh shell) leaves for good, also in a tab in the background, a new pane does not join, and fewer than two members end it. What is typed in a member (keys, Tab and Shift+Tab, pastes, IME text) also goes to the other members, each encoding it with its own terminal modes and Backspace setting (a paste of several lines is left out where the shell takes no bracketed paste); the receiving members have a `--warning` border while the active pane is a member. From the moment the text before the cursor of the pane typed in ends in a password prompt (the prompt pattern of security.md 10.1 and a `:` last: `sudo`, `su`, `mysql -p`) until Enter or Ctrl+C is typed there or the shell is back at its own prompt, nothing is mirrored, that Enter included; a prompt that echoes `*` stays private too. While the shell edits its own line (bracketed paste on: bash, zsh, fish) nothing is checked, so a command that names a secret (`kubectl get secret`) or a typed `TOKEN:` is mirrored; in a shell without bracketed paste a typed `password:` pauses too (it fails closed). Only the end of the line before the cursor is read. Mirrored input arrives as injected input, so the receiving pane never records that line in its command history. Replies to terminal queries, mouse reports and input the app injects stay in their own pane.
- Ctrl+Shift+B (Settings → Shortcuts, "Broadcast input on or off") does what the tab menu item does in the active tab. A paste of several lines asks first while Broadcast input is on, also with the multi-line confirmation setting off, and the dialog says the other panes get it too.
- Not done: Broadcast input to panes of other tabs (plan 9.3 asks for panes and tabs): which tabs join (every tab of the window, tabs picked in a list, a tab group) is for the owner; the members, the pause and the marks would carry over as they are.
- Workspaces, first part: the core keeps `workspace` records (0x0A01, data-model 12: a name and the tabs in the shape of Restore's, checked by the same rules; host links and local terminals only, so a quick connection is left out). "Create a workspace" on the new tab page saves the window's tabs under the name a dialog asks (disabled while no tab can be kept); a "Workspaces" block on the page lists them by name: a click opens the tabs in place of the page (hosts removed since are left out, as in Restore), × removes one.
- A snippet run on two to four hosts (D68, item 2): the confirmation has "Run in split tab" beside Run. The tab has one pane per host (two side by side, three in a row, four in a grid); each pane types the script and Enter once its session first runs and the shell waits at its prompt (it turned bracketed paste on; at most 3 s), as the terminal sidebar's Run does (a reconnect does not type it again). Fewer than two hosts left: a toast says so. Run keeps the results table.
- Add to Workspace ▸ in the host menu lists the workspaces by name, then New Workspace (its name is asked): each chosen host becomes a tab with one pane after the workspace's tabs (32 tabs at most).
- Ctrl+K finds workspaces by name ("Workspaces", after the open tabs); × asks before it removes one (the hosts and open tabs stay). The names are read with the records (no read of the tabs per frame).
- A pencil beside a workspace renames it (the tabs are kept).
- Not done tonight: workspaces in Export and Import Data (their host links would need the import's id mapping). Done in run 5 (D84).

**Spec impact:** `docs/ui-behavior-spec.md` 0 and 4.7 (and `docs/ru`): Broadcast input and Create a workspace are no longer disabled.

### 82. The Windows release build, and the MSI waits for an installer tool (run 4)

Status: run 4 decision, 2026-10-22 (plan, item 9.5). The installer tool is listed for the owner.

**Decision:**

- The artifact is built with a `dist` profile (inherits `release`): fat LTO, one codegen unit, line tables in the PDB (kept beside the artifact for crash reports, not in the zip). `--release` keeps the default profile, so tests, the measurement scripts and other CI builds are unchanged. `panic = "abort"` is not used: two startup paths catch a panic of the OS keychain library on a helper thread and go on to the unlock screen (`main.rs` `try_device_unlock`, `platform::desktop`'s probe); with abort, such a panic would end the app.
- `shellpp.exe` carries a version resource made from the package version of `Cargo.toml` by `apps/desktop/build.rs` (`embed-resource`, MIT, already in the dependency tree through GPUI).
- The artifact is a portable zip (`scripts/package-windows.ps1`: `shellpp.exe`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `README-portable.txt`, and a `.sha256`), built by the CI job "Windows release artifact" on pushes to main and kept for 14 days with the PDB. No signing, no publishing.
- The MSI is not made: the tools that build one do not fit SPEC.md's license list, even for build-only tools (BlueOak-1.0.0 and CC-BY-4.0 are the only additions there). The options, for the owner: (a) WiX Toolset (MS-RL, the common MSI builder; the MSI it produces holds no WiX code unless custom actions are used): an exception to the list for a build-only tool; (b) an NSIS installer (`.exe`, zlib license, which the list allows) instead of an MSI, without MSI's group policy and Intune deployment; (c) the portable zip only until signing exists. Recommendation: (a), because organizations deploy MSIs.
- The three icon variants in `docs/assets/` are drafts for the owner's choice; the executable gets an icon once one is chosen.

**Spec impact:** none; README gains "Installing on Windows".

### 83. The owner's answers to the questions of run 4

Status: accepted, 2026-10-22 (owner's choice; the questions in section 3 of the run 4 report). Amends D70, D78, D81 and D82 where they differ; the plan of run 5.

**Decision:**

1. Interface scale (D78): option (a), the design system in rem with `set_rem_size`, in a separate run later. Not now; the menu items stay placeholders.
2. Installer (D82): the portable zip only for now. Once code signing exists, an NSIS installer. No MSI.
3. App icon (D82): variant B, `docs/assets/icon-b-window.svg` (an accent tile with a terminal window, its prompt and a plus), becomes an `.ico` with every size from 16 to 256 px, embedded in the executable and set on the window; 16 and 32 px must stay legible. The other drafts are then removed. (The owner named the variant after the first answer, which had left it open.)
4. Windows app identity (D70): the process sets `Shellpp.Desktop` with `SetCurrentProcessExplicitAppUserModelID` at every start; its registration under HKCU happens at the first start, not at the first notification. A test run (`SHELLPP_TEST=1`) and a temporary profile (`--demo`) still set and register nothing, as in D70.
5. No allocator that wipes freed memory. The copies of secrets the code cannot wipe are listed in `docs/security.md` (section 9.5, RR11) as a known limitation.
6. Broadcast input to panes of other tabs (D81): option (b), the tabs ticked in a list.
7. Test images: the programs needed only for the screenshots of terminal compatibility (emacs, vttest and the others of plan 9.2) move to an image of their own; the main OpenSSH test image returns to its earlier size.

**Spec impact:** `docs/security.md` 9.5 and RR11 (item 5) now; README "Installing on Windows" (item 2) now; with the implementation in run 5: UI 4.4 and 4.7 (item 6, and `docs/ru`), D70's notification identity (item 4), `tests/openssh` (item 7). Item 1 changes nothing until its run.

## Run 5 of 2026-10-22

The decisions below were taken during the owner's fifth long run (its plan, section 1.1), marked "run 5"; the disputable ones are listed for the owner in the run report.

### 84. Workspaces in Export and Import Data (run 5)

Status: run 5 decision, 2026-10-22 (plan, section 5; D81's "not done").

**Decision:**

- Export Data already wrote workspaces (every record but the recovery key); Import Data now takes them, after the hosts, by the rules of D34 and D63 like snippets and forwarding rules: a new id is added under that id; the same content is "already exists"; other content under an id here is a conflict, imported as a copy with " (imported)" on its name and the `import_key` `json:<exported id>`, which the next import finds.
- A workspace's host links are references like any other: the import rewrites them with the same id map, so they point to the host that stays here or to the imported copy of a conflicting host, and a link to the exported personal vault names the personal vault here. A link to a host that is neither here nor in the file stays, as every reference (D63); opening the workspace leaves that pane out (D81).
- Each workspace is checked by the rules of a save: at most 32 tabs, the shape of Restore (splits, sizes, leaves, depth), no quick-connect target; one that breaks them is refused with its errors. `import_key` is now allowed in a workspace (data-model 3.3), managed like the other types: a save never takes it from its input, an edit keeps it.

**Spec impact:** `docs/export-format.md` section 4, `docs/data-model.md` 3.3, `docs/ui-behavior-spec.md` 1.4 (and `docs/ru`).

### 85. A compact journal for the old part of the scrollback (run 5)

Status: run 5 decision, 2026-10-22 (plan, section 6; night 1 report, section 2.4).

**Context.** Alacritty keeps every scrollback row as wide as the window, 24 bytes a cell: a full scrollback of 10 000 lines took 39 MB in night 1. The plan asks for at most 5 MB per session, search and selection over the whole history, and output not slower.

**Decision:**

- The grid keeps the screen and the newest 500 scrollback lines as alacritty rows. Older rows move into a journal (`terminal/journal.rs`): each row as its text without trailing blanks (every cell's character, combining marks after it) and its attributes as runs, plus its wrap. 10 000 typical shell lines in a 200-column window take about 4 MB together with the grid's part (test `ten_thousand_shell_lines_take_a_few_megabytes`).
- Rows move in batches, on the parser thread: when it has nothing queued, or at most every 250 ms (run 6, D99: 2 s) while output keeps coming and the grid is 200 lines over its part. A flood pays nothing per line (encoding each row as it leaves would cost about 0.3 µs a line, more than the parsing); the grid holds at most the rows of one interval over its part, and gives them back at the next compaction or when the flood pauses.
- The scrollback limit (10 000 lines) holds for the journal and the grid together: when output needs room, the journal's oldest lines go first, so the buffer keeps its order and positions counted from the oldest line stay valid. ED 3 and RIS clear the journal with the scrollback.
- Drawing, search and selection read the journal and the grid as one buffer of lines. The view's scroll offset covers the grid's scrollback first, then the journal; while the view is in the journal, new output keeps it on its text.
- The selection is now the app's own, in buffer lines counted with the lines that dropped off (as the search's positions), so it reaches into the journal and stays on its text. Its part on the screen is mirrored into alacritty's selection, which alacritty drops when output clears it (an erased line or screen, a reset) and moves with text that scrolls inside the screen (the alternate screen, a scroll region): the app's selection ends or moves with it, as before. A new width ends it. A double click on a bracket no longer selects up to its matching bracket. Copying keeps alacritty's rules (trailing blanks left out, a line break where a line ends, wide characters whole).
- Not kept in the journal: OSC 8 links and underline colors. Journal rows keep the width they had when they moved: a later resize does not reflow them (a wider row is cut, a narrower one padded).

**Spec impact:** none (UI 4.2's search and 4.3's selection already cover the whole scrollback).

### 86. Broadcast input to ticked tabs (run 5)

Status: run 5 decision, 2026-10-22 (plan, section 7; D83 item 6: "the tabs ticked in a list").

**Decision:**

- The tab menu's "Broadcast Input" item shows on a tab with splits and on any session tab while other tabs are open. With other tabs open it opens a list of the open session tabs (by their names), each with a check box, the tab of the menu ticked (and the tabs that broadcast now); Start begins Broadcast input over the ticked tabs. With one tab it starts at once, as before. On a tab that broadcasts, the item stops it everywhere.
- The members are the panes whose sessions run when it starts, in all ticked tabs; what is typed in one goes to all the others, with every rule of D81 (the password pause, Tab, the confirmed paste and its count). A pane that stops running leaves, a tab that closes leaves with its panes, and fewer than two members in all end it, in every tab.
- Each tab that takes part shows a tower in `--warning` by its name in the tab strip; in a tab, the receiving panes have the border, also when the active pane of that tab is not a member (the other tabs send to it).
- Ctrl+Shift+B turns Broadcast input off, or on for the current tab only (the quick case); the list is the menu's.
- The window keeps the set of tabs; each tab's pane group keeps its members and the other tabs' members as peers, and tells the window when its members change.

**Spec impact:** `docs/ui-behavior-spec.md` 4.4 item 7 and 4.7 (and `docs/ru`).

### 87. Telnet sessions (run 5)

Status: run 5 decision, 2026-10-22 (plan, section 9.1; UI 3.1 "Add Telnet", data-model 7.4; stage 9 of SPEC.md).

**Decision:**

- A saved host's Telnet block opens a Telnet session tab: Connect ▸ via Telnet (enabled when every chosen host has a Telnet block), and Quick Connect, Enter and a double click for a host whose `primary_protocol` is `telnet`. The tab is the same terminal as SSH; Duplicate, split panes, Restore and workspaces keep it (`{"kind": "telnet", "host": host_link}`, data-model 12).
- The connection: the host's address and `telnet.port` (23 by default), the same name resolution and 15 s connect timeout as SSH; RFC 854 with ECHO and SUPPRESS-GO-AHEAD from the server and TERMINAL-TYPE (`terminal.term`) and NAWS from the client, sent again on every resize; any other option is refused, and an option is answered only when its state changes. Input doubles 0xFF and sends CR as `CR NUL`.
- The host's `charset` and `backspace` apply, as for SSH (data-model 7.4).
- Login is interactive: the host's saved username and password are not sent (`telnet.username` stays a note for now).
- The server closing the connection is a drop "closed by the server", a network error one "connection lost": the output stays and Reconnect connects again. Telnet has no exit status, so a tab never closes by itself. There is no automatic reconnect for Telnet.
- The host panel's Telnet block says that Telnet is not encrypted (the login and password cross the network as plain text).
- Connect from the host panel, the new tab page, Ctrl+K and a split pane's picker follow the primary protocol too; a split from a Telnet pane opens Telnet to the same host.
- A server that sends and never reads cannot grow the client's memory: replies and input wait in a bounded queue, and the output side stops reading while it is full; closing the tab ends the writer even when the socket does not drain.
- Not in this run: Telnet in the connection log and in Recent connections, and quick connect by `telnet://`.

**Spec impact:** `docs/ui-behavior-spec.md` 2.4 (Connect ▸ via Telnet) and 3.1 item 6 (and `docs/ru`).

### 88. Serial connections (run 5)

Status: run 5 decision, 2026-10-22 (plan, section 9.2; UI 1.4 item 6, 2.2 "Serial"; stage 9 of SPEC.md).

**Decision:**

- The Serial button, Ctrl+Alt+S and New Serial Connection open a dialog: the port (the ports the system lists, sorted by name, Refresh), the baud rate (300 to 921600, 9600 by default: the console of most network devices), data bits, parity (None, Odd, Even), stop bits and flow control (None, XON/XOFF, RTS/CTS). 8N1 without flow control by default. Mark and space parity are not offered (the `serialport` crate has none). A port the system does not list cannot be chosen (run 6, D95: it can be typed).
- Connect opens the port in a session tab with the same terminal; the title is the port name. Bytes go to the port as typed and its output is UTF-8. A serial line has no window size and no exit status.
- A port that does not open fails with the reason: not found, in use or access denied (Windows reports both the same way: a listed port that does not open is "in use"), settings refused, or any other failure. An unplugged device drops the tab; Reconnect opens the port again with the same settings.
- A port opens once: Duplicate and Duplicate in a new window are disabled on a serial tab; a split still works (the new pane chooses a host). Serial panes are never saved (data-model 12): snapshots for Restore and workspaces leave them out. The dialog offers the settings of the last Connect again during the app run (run 6, D95: also after a restart).
- The ports and the connection live in `shellpp-term-local` (SPEC.md, crates): `serialport` 4.10 (MPL-2.0) without its default `libudev` feature, so that Linux builds need no system library (ports are found through sysfs). One thread owns the port (one handle: Windows serializes the I/O of a synchronous handle, and a duplicated handle would be inherited by programs the app starts and keep the port busy): it writes the queued input in 64-byte chunks for at most 100 ms, then reads with a 20 ms timeout. It looks at the close flag between chunks and reads, and ends when the session is closed or dropped, so a closed tab lets go of the port at once, also in the middle of a long paste or while the port was still opening. Cancel while the port opens fails the tab as cancelled; Reconnect then opens it again. Tests use a fake port behind the `SerialBackend` trait; no hardware is needed.
- Not in this run: saved serial profiles, a typed port name, local echo, line endings other than CR, break signals and the charset of a serial tab. (Run 6, D95: the last settings are kept between runs and a port name can be typed; profiles stay out, D92.)

**Spec impact:** `docs/ui-behavior-spec.md` 1.4 item 6, 2.2 "Serial" and 4.4 (and `docs/ru`).

### 89. The app icon, the app identity and the programs image (run 5)

Status: run 5 decision, 2026-10-22 (plan, section 10; how D83, items 3, 4 and 7, is built). Amends D70 on when the identity is registered.

**Decision:**

- **Icon.** Two masters of variant B: `docs/assets/icon-b-window.svg` from 32 px up, and `icon-b-window-small.svg` for 16 to 24 px, the same tile and window on the 16 px grid without the title bar's dots and the plus (at 16 px they turned into noise; the prompt is drawn thicker). `node scripts/gen-icon.mjs` renders them with the `resvg` command line tool (`cargo install resvg --version 0.46.0 --locked`, a development tool) into `apps/desktop/assets/icons/`: `icon.ico` with 16, 20, 24, 32, 40, 48, 64, 96, 128 and 256 px (PNG entries), `icon.icns` (16 to 1024 px) and the sized PNGs. `build.rs` embeds `icon.ico` as icon 1 next to the version resource: Explorer shows it for `shellpp.exe`, and GPUI gives the window icon 1 of the module, so the title bar and the taskbar show it as well. Drafts A and C are removed.
- **Identity.** At every start on the user's profile the process sets `Shellpp.Desktop` (GPUI's `set_app_identity`: `SetCurrentProcessExplicitAppUserModelID`, and the identity its notifications use). When `HKCU\Software\Classes\AppUserModelId\Shellpp.Desktop` has no `DisplayName`, the start registers it ("Shell++"): the first start, and again after the key was removed. A refused registration is logged and tried again at the next start. A test run and a temporary profile do neither. The calls go through the `IdentitySystem` trait (`app/identity.rs`), whose fake records them in tests. No `IconUri` is registered: the portable zip has no image file outside the executable; the installer (D83, item 2) can add one.
- **Test images.** The main OpenSSH test image is back to openssh-server, htop, mc, procps and busybox (179 MB, as before run 4). `tests/openssh/programs.Dockerfile` adds tmux, vim, nano, less, emacs-nox and vttest on top of it (658 MB, sharing its layers); `start.sh` builds and runs it with `SHELLPP_OPENSSH_PROGRAMS=1`, and `scripts/ui-screens.ps1` asks for it when a run includes a program state. The session tests and the server matrix keep the main image.

**Spec impact:** none in the UI specs; `tests/openssh/README.md`, `docs/compatibility.md` section 2, `docs/assets/README.md`.

### 90. Run 4 tails: emoji sequences, DECCOLM, the Keychain's focus ring, Undo of a workspace removal (run 5)

Status: run 5 decision, 2026-10-22 (plan, section 11).

**Decision:**

- **Emoji sequences.** The grid keeps one character per cell as the shell's `wcwidth` counts them (a skin tone modifier is two more columns, a ZWJ family six in all): changing the columns would move the shell's cursor out of step with the screen. Drawing joins the cells of a ZWJ sequence, an emoji with a skin tone modifier, and a flag's two regional indicators into one glyph two cells wide at the first cell; the other cells stay blank. Selection and copy take the cells as before. VS16 presentation (`❤️`) is unchanged.
- **DECCOLM.** As in xterm: DECCOLM (mode 3) switches to 132 or 80 columns only after the program allowed it with DECSET 40, which vttest does and `tput reset` does not. The grid takes the width at once, in the middle of the output, and clears with the scrolling region and the cursor home; the remote side gets the new size. The width stays until the pane's own size changes, RIS or DECRST 40 (which a reconnect sends with its mode reset, so a new shell does not inherit it); the pane draws the narrower grid at its left. The resize keeps the scrollback as a view resize does.
- **Keychain.** The keyboard focus ring marks the first key on display while nothing is selected, where the arrows start, as on Hosts and Snippets.
- **Workspaces.** × on a workspace of the new tab page removes it at once, with a toast that offers Undo for 5 s (the undo of UI 2.4): Undo brings it back under its id with its tabs. The question before the removal is gone, as for hosts.

**Spec impact:** `docs/ui-behavior-spec.md` 4.5 (and `docs/ru`); `docs/compatibility.md` section 2.

### 91. Long-running fixes (run 5)

Status: run 5 decision, 2026-10-22 (plan, section 12: a review of what grows or stays alive over hours and days).

**Decision:**

- **Shortcuts.** GPUI's keymap only grows and is searched on every keystroke: applying the shortcuts binds only the actions whose keys changed since the last time, so a new window binds nothing (before: about 90 bindings per window and per edit).
- **Forwarding.** A rule an autostart (on launch, on connect) cannot start is stopped as the Forwarding screen stops a failed rule, so its host's connection does not stay up and reconnect after every drop with nothing on it; its failure is still shown until the rule is started or stopped.
- **SFTP Open.** The copies of an SFTP session that ended (the panel went to another host or closed) leave the list checked every 2 s; a copy without changes is removed at once, a saved one stays for the question at exit or at the next start. A program started for "Program for Open" is waited for on a thread, so no zombie process stays per opened file on macOS and Linux.
- **History.** The core forgets a session's previous command (the "same as the previous entry" rule) when the session ends, instead of keeping the plain text of every session's last command until the lock.
- **Recordings.** A recording stops at 4 096 chunks (256 MiB of text) with a marker event (storage-format 8.3); the rest of that session's output is not recorded.
- Not changed: the SFTP Back history and the finished jobs of the transfer queue (until "Clear finished") grow with the user's own actions; the forwarding maps are bounded by the number of hosts and rules.

**Spec impact:** `docs/storage-format.md` 8.3.

## The owner's answers to run 5

### 92. The owner's answers to the questions of run 5

Status: accepted, 2026-10-22 (owner's choice; the five questions in section 3 of the run 5 report). Amends D84, D88 and D90 where they differ; items 1 and 5 go into the plan of run 6.

**Decision:**

1. A workspace's link to a host that does not exist stays (D63, D84): an import keeps it and opening the workspace leaves that pane out (D81). In addition, the Workspaces block of the new tab page marks such a workspace "N hosts not found", and its tooltip names the missing hosts as the workspace saved them (run 6).
2. No mode 2027 (grapheme clusters negotiated with the shell). The blank cells after an emoji sequence drawn as one glyph (D90) stay a known limitation, recorded in `docs/compatibility.md`, section 2.
3. DECCOLM stays as D90 has it: 80 or 132 columns only after DECSET 40, until the pane's size changes, RIS or DECRST 40.
4. Removing a workspace stays as D90 has it: at once, with Undo in a toast for 5 seconds, without a question.
5. Serial (D88): no saved port profiles. The settings of the last Connect are kept between app runs (a per-device setting: port names belong to one device). The port field also takes a name typed by hand when the port is not in the system's list (run 6).

**Spec impact:** `docs/compatibility.md` section 2 (item 2) now; with the implementation (run 6): `docs/ui-behavior-spec.md` 2.2 "Serial" and the new tab page's Workspaces block (6), and `docs/ru`.

## The owner's answers to the sync design draft

### 93. The owner's answers to the sync design draft

Status: accepted, 2026-10-22 (owner's choice; the 18 questions of `docs/sync-design-draft.md`, section 11, and a new requirement: the server will run in the owner's homelab). The owner gave the number 93; 92 holds the owner's answers to the questions of run 5, recorded after it. Amends D11 (a server whose personal epoch is below the device's), D12 (records missing from a snapshot) and D14 (server restores) where they differ, and the stage 0 decisions that left custom CA certificates for sync servers out of the MVP (architecture.md 1.3) and that a received tombstone removes the local copy (storage-format.md 4.6). Closes the draft. SPEC stage 7 is still not started: this changes documents only.

**Decision** (the owner's answers; each option as the draft worded it):

1. The stage 0 design of `docs/sync-protocol.md` and `docs/security.md` (draft, section 2.3) is the accepted basis for stages 7 and 8; only the choices below were open.
2. Storage quotas (Choice 1): (b), optional per-account limits (records, bytes) set by the operator, off by default; a push item status `quota_exceeded` marks the record "not synced". Registration stays invite-only by default; the operator turns open registration on in the server's configuration (anyone can run a server of their own).
3. Records missing from a full snapshot (Choice 2): (c), they are pushed back and listed for review, with "Delete again" per record.
4. A server whose key epoch is below the device's (Choice 3): (b), the device that holds the newer epochs re-posts them with the same keys, one by one, through the existing `key-epochs` endpoint (personal vault: after password re-entry; team vault: an owner, under the interactive recipient rules, removing again the users its roster marks `removed`), then pushes the records back. Every time only after the user's explicit confirmation, never automatically.
5. Team vault archives of an earlier account (Choice 4): (a), `account_user_id` and `server_url` are stored on each `vaults` row (an additive migration).
6. A pulled record whose key epoch goes down (Choice 5): yes, (b): a pulled version whose `key_epoch` is below that of the local synced version is refused, with the integrity warning.
7. The same object created on two devices (Choice 6): (c), a duplicate hint with Merge, extending the merge of identical keys to the other types by natural key.
8. Conflict copies of types without a view (Choice 7): (a), copies for all types, and a generic comparison (fields, secrets masked) for types without a card; plus (c), a winner rule per type where the type needs one, as for `roster_entry`.
9. Concurrent SSH ID regeneration: yes, the device whose `ssh_id` version wins re-publishes its public key when the server's differs.
10. Deletions by other devices and members (Choice 8): (b), and for every vault, the personal one included, not only team vaults. "Recently deleted" (the trash): every deleted record, deleted on this device or arriving from another one, is kept on this device for 90 days, encrypted, and never synced. Actions: Restore, Export, Delete forever and "Empty trash". Its place in the interface is ours to propose (below). Undo for 5 seconds stays.
11. A lost or stolen device (Choice 9): (b), Revoke asks "Lost or stolen?"; yes continues with a password change that rotates the personal vault and offers to rotate the team vaults the user owns; for the other team vaults it says to ask an owner.
12. Account deletion (Choice 10): (a), immediate, with the dialog of (c), which offers Export and says what the members lose.
13. The account's other devices (Choice 11): yes, (b): the server keeps the account's refresh token hashes and the date for 30 days and answers `401 account_deleted`; the client says so, and that the local copy is kept.
14. The user's published connection log entries in team vaults (Choice 12): (c), they stay, and the deletion dialog says so.
15. Server backups (Choice 13): (c), tested recipes: a dump piped to an encrypting tool, a scheduled compose service; the recommended retention is 30 days. A recipe for a homelab: Docker Compose with a scheduled encrypted dump.
16. Detecting a restore (Choice 14): yes, (b): `GET /v1/server` returns a random `server_generation`; every restore ends with `shellpp-server admin restored`, which replaces it; a client that sees a new value runs the restore procedure for every vault.
17. What a restore brings back (Choice 15): (c) for now, documented manual steps: the server logs carry user ids; `admin user delete` for each account deleted after the backup.
18. A lost response to a token refresh: the server accepts the previous refresh token once, within 60 seconds, while its successor is unused (instead of signing the device out).
19. The server will run in the owner's homelab:
    - deployment documentation for three ways of access: (1) an own domain, a reverse proxy and Let's Encrypt (Caddy); (2) Cloudflare Tunnel; (3) only through a VPN (Tailscale or WireGuard) with an internal certificate;
    - the client: for a server whose certificate the system does not trust, an explicit choice: "Trust this certificate" with its fingerprint shown (a pin, like TOFU for SSH), or adding one's own CA. A change of the pinned certificate blocks sync until an explicit confirmation. An invalid certificate is never accepted silently. HTTP without TLS only for localhost.

Details we chose while carrying the answers into the documents (marked [decision] there; the owner can change them):

- Quotas: an account's usage is its live records (`created_by`) and their envelope bytes in every vault. A push item counts against the pushing account only for a new record or a record it created that grows, so a member is never stopped by another account's quota, and deletions, re-encryption and writes that do not grow are never refused. `SHELLPP_QUOTA_MAX_RECORDS` and `SHELLPP_QUOTA_MAX_BYTES` (unset: no limit), `admin user quota` per account; the account bundle reports the usage.
- Brought back: a local list of the records a full resync pushed back, in Account & Sync, with "Delete again" and "Keep"; types the user cannot delete are only counted. A server restore's push-back is not listed.
- Re-post: personal vault after the password the server holds now; team vault from an owner device with the interactive recipient dialog, the removals in its first step; non-owners and devices that do not hold the newer epochs only show "server state rolled back". Members added after the backup are invited again.
- The epoch rule also covers `current` of a push conflict.
- Duplicates: a pair is raised when a pull inserts a record whose natural key equals that of a live record of the same type in the same vault (not for history and log entries). The user picks the record to keep; references are rewritten to it; "Keep both" is remembered on this device. Screen hints and a comparison dialog, as for conflicts.
- Winner rules: `known_host` (a `revoked` marker wins) and `member_pin` (with the same key, `verified` wins), besides `roster_entry`; both get a security review before stage 7 ships.
- SSH ID: the device that wrote the winning version checks after each pull, conflict and sign-in.
- Recently deleted: a sidebar section below Logs and a screen of its own. It keeps the last live envelope of every user-visible record; not command history (its deletion is the remedy for a secret the detector missed), not log entries removed by retention, not moves made on this device, not conflict copies or bookkeeping types. Restore writes the record back under its id, or copies it to the personal vault where the user cannot write.
- Lost device: a Devices block in Account & Sync (names from `device_info`). A thief also holds the user's X25519 key (RR17), so the team vault rotation protects only while the server refuses the stolen device.
- `account_deleted`: other devices then do what the deleting device did (local-only personal vault, team archives).
- Backups: `age` to an operator's public key; the private key never on the server; tested in CI with the server.
- Manual restore steps also use `admin device revoke` and `admin vault delete`; the server logs each irreversible event with ids only.
- Token grace: the unused successor is marked used, so presenting it later is a reuse that revokes the family; the grace is granted once per token.
- From the review of the documents: a token refresh times out after 20 s and one without a response is retried at once, so the retry arrives within the 60 s; tombstones written by retention stay out of Recently deleted only on the device that writes them; an added CA counts together with the system roots; binding stores `server_generation` and the vaults' `account_user_id` and `server_url`.
- Certificates: a pin is the SHA-256 of the server certificate, shown as `openssl x509 -fingerprint -sha256` prints it; only an unknown issuer can be pinned or covered by an added CA, while an expired certificate or one for another name is refused; an added CA counts for that server only; any certificate other than the pinned one blocks sync, also one the system trusts; trust choices are kept in a plaintext table, since recovery runs while the app is locked. Cloudflare Tunnel puts Cloudflare in the position of an active server for the transport.

**Spec impact:** `docs/sync-protocol.md` (conventions; 2, 2.1, 2.2 new; 3.1; 3.4; 4.4; 4.6; 4.7; 5.1; 5.6; 5.8; 6.4; 6.5; 6.6; 6.7; 6.8; 6.9; 6.10; 6.11; 7.7; 7.11; 8; 9; 10.2; 10.3; 11; 12; 13; 14), `docs/security.md` (conventions; 1; 3; 4; 5.1; 5.9; 6; 9.1; 9.3; 14.1–14.4; 15; 17; 18), `docs/data-model.md` (conventions; 2.1; 6.1; 6.6; 13.3; 15; 16.6–16.8 new; 18.2; 18.3), `docs/storage-format.md` (conventions; 4.2–4.6; 6.2; 6.6; 12; 14.5), `docs/ui-behavior-spec.md` and `docs/ru/ui-behavior-spec.md` (1.2; 2.4; 7.2; 12.2; 12.3; 13.2; 13.5; 13.6; 13.7 and 14 new); `docs/sync-design-draft.md` is marked closed. SPEC.md is unchanged. `docs/architecture.md` 1.3 (custom CA certificates and pins for sync servers are in the MVP) and 10.2 (the admin subcommands, the backup service, the three ways of access, certificate trust, the quota variables). `server/README.md` follows with the server in stage 7.

## Run 6 of 2026-10-22

The decisions below were taken during the owner's sixth long run (its plan, section 1.1), marked "run 6"; the disputable ones are listed for the owner in the run report.

### 94. Workspaces name the hosts that are gone (run 6)

Status: run 6 decision, 2026-10-22 (plan, section 5; D92, item 1).

**Decision:**

- A workspace's links to hosts that do not exist stay (D63, D92). The new tab page's Workspaces block marks such a workspace "N hosts not found" in the warning color, with the hosts in the tooltip; Ctrl+K shows the count as the workspace's detail. "Do not exist" is: no live host record with that id on this device (deleted, not imported, or in a vault the device does not hold); Telnet panes count like SSH panes.
- The record had only host links, so a host deleted since could not be named. The workspace gets `host_names` (data-model 12): the name (label, else address) of each linked host, written by the core on every save; a host that is gone keeps the name saved before. The desktop's three save paths (create, Add to Workspace, rename) need no change. An import keeps the file's names, and its id remap rewrites the links inside them like the tabs' links; a file without the member reads as before.
- Removing a host names it in every workspace that links it, by the name it has then, in the same transaction (the workspaces are rewritten). So a workspace saved before this run, or imported without names, names a host removed here; a host renamed after the save is named by its new name. A host that never was on this device and has no saved name is "an unknown host".
- The names are advisory: a bad entry (no host link, no name, a host twice, more than 512) is dropped on save and import, never the workspace; an entry's members this version does not know stay (data-model 1). The size and depth checks of the tabs run before the names read them.
- Privacy: a name lives as long as the workspace links the host (D63), like the link; removing the pane or the workspace removes it. An export writes the names of the hosts the file carries only, so the label or address of a host that is gone never leaves the device in a file (D41, D50). Recently deleted (D93) is not built yet, so the names cannot come from there.
- The core lists each workspace with its missing hosts (`workspace_summaries`); the app reloads it with the records, so the mark follows deletions, Undo and imports at once.

**Spec impact:** `docs/ui-behavior-spec.md` 4.5 (and `docs/ru`), `docs/data-model.md` 12.

### 95. The Serial dialog keeps its settings and takes a typed port (run 6)

Status: run 6 decision, 2026-10-22 (plan, section 6; D92, item 5). Amends D88 on the settings of the last Connect and on ports the system does not list.

**Decision:**

- The settings of the last Connect (port, baud rate, data bits, parity, stop bits, flow control) are settings of this device: `serial.port`, `serial.baud`, `serial.data_bits`, `serial.parity`, `serial.stop_bits` and `serial.flow` (data-model 16.1), written at Connect and offered by the dialog of every window, also after a restart. A port name belongs to one device: Import Data never applies them (like the settings of D41 that run or point somewhere). No serial profiles (D92).
- The port is a text field with the list of ports found beside it. A name typed by hand (`COM7`, `/dev/ttyUSB3`, `\\.\COM12`) opens like a listed one; the spaces around it are dropped; an empty field, or a name longer than 256 bytes ("The port name is too long"), disables Connect. An empty field takes the first port found; a name in it stays when the ports are listed again, also one the system does not list, which is marked "Not found now" in the warning color (a saved port of an unplugged adapter is not replaced by another port). On Windows a name is compared as the system does: not case-sensitive, with or without the `\\.\` prefix (`com3` is the listed COM3), also when an open error tells "in use" from "not found". Open errors stay those of D88.
- The settings are written at Connect; when that fails, the tab still opens and a toast says the settings were not kept.
- The `serial.*` settings never leave the device in a file: an export leaves them out, and no import applies them, not even "Full restore from my backup" (D50), which applies every other setting of the file.

**Spec impact:** `docs/ui-behavior-spec.md` 2.2 "Serial" (and `docs/ru`), `docs/data-model.md` 16.1, `docs/export-format.md` 1 and 4.

### 96. Interface zoom with the design system in rems (run 6)

Status: run 6 decision, 2026-10-22 (plan, section 7; closes D78 with its option (a), the owner's choice).

**Decision:**

- Every size of the design system is a rem size: `theme::metrics` holds `Rems` (`rpx(px)`, 1 rem = 14 px at 100 %), and so do the views' own sizes (column widths, dialog fields, tiles, gaps). GPUI Kit takes the theme's base font size as each window's rem, so the zoom sets that size (`font.size` and `mono_font.size` of both themes) and every window, the kit's own components included, scales at once.
- View > Zoom In / Zoom Out / Actual Size and Ctrl+= / Ctrl+- / Ctrl+0 (Cmd on the Mac set; rows of the shortcuts table, UI 12.5) go by the steps 80, 90, 100, 110, 125, 150, 175 and 200 %; the zoom is kept in `appearance.ui_zoom` (now 0.8 to 2.0), applied before the first window of the next launch, the lock screen included, and at once when the setting changes otherwise (an import). The keys are bound at the outermost key context of a window (the kit's root), so they work over Settings, dialogs and the lock screen too, and a deeper context wins: in a terminal the three keys go to the program (like Ctrl+letter). The menu items work everywhere.
- The kit's own radii (`radius`, `radius.lg`: pixels in the kit) and our shadows follow the zoom with the font size; the kit input's padding stays its own pixels, which the layouts planned in design units divide by the zoom.
- Not zoomed: the terminal's font and grid (`terminal.font_size`, its own setting), the terminal's 8 px padding, the 8 px hit zone of the pane dividers and the offset of a new window (screen pixels). The zoom comes on top of the display's scale factor.
- Layouts planned in numbers (the host grid and its columns, the action bars, the details panel width and the sidebar collapse) plan in design units, px at 100 %: measured widths and pointer positions are divided by the zoom, the result is drawn in rems. So a zoomed window plans like a window as many times narrower at 100 % (960 × 600 at 150 % like 640 × 400). The APIs that take pixels only (the kit's dialog and menu widths, button radius, box shadows, the item sizes of virtual lists) get the pixels at the zoom; a menu built without a context keeps a 200 px floor.
- Tests compare measured pixels with `metrics::at_100(...)`; a test at 150 % checks the Hosts screen in the minimum window (nothing clipped or overlapping, with and without the details panel), one at 175 and 200 % its bars without the panel, another the steps, the setting, the keys in a terminal and over Settings, and the terminal font. `ui-screens.ps1 -Zoom <percent>` takes the screenshots at a zoom, named `<W>x<H>-<theme>-<zoom>-<state>.png`.
- Known limit: GPUI cannot change a window's minimum size after it opens, so the minimum stays 960 × 600 px. At 175 and 200 % that window is 548 and 480 px wide at 100 %: with the details panel open (320 px at 100 % at least) the main area gets narrower than its 360 px and the action bar of a screen may be cut; a wider window, or 150 % and less, has none of this. (Run 7, D100: there the details panel opens over the main area instead, UI 1.3.)

**Spec impact:** `docs/ui-behavior-spec.md` 1.4 item 8 and 12.5 (and `docs/ru`), `docs/design-system.md` 6 (and `docs/ru`), `docs/data-model.md` 16.1.

### 97. The update check only notifies (run 6)

Status: run 6 decision, 2026-10-22 (plan, section 10; SPEC stage 9). Amends D7 and UI 12.7 for the GPUI app: no download, no install.

**Decision:**

- Help ▸ Check for updates and Settings ▸ Updates ▸ Check Now always work (one at a time; Help says "Checking for updates…" at once); the check at launch only when "Check for updates at launch" (`updates.auto_check`) is on (off by default, principle 2), at most once a day (`updates.last_check`, the time of the last check, failed or not; a time in the future does not stop it). `updates.last_check` belongs to the device: an export leaves it out and no import applies it.
- The source is the release manifest, by default `https://github.com/proxyboy228/shellpp/releases/latest/download/latest.json` (architecture.md 7; the GitHub releases API was the other choice: it would need a parser of the API's JSON and its rate limits for no gain). A setting changes it (`updates.manifest_url`, `https://` only, no spaces, at most 2048 bytes). One GET, through redirects that stay on `https://`, 15 s at most, 256 KiB at most, with `User-Agent: shellpp/<version>` and nothing else of ours (no cookies, no ids; a source with a user part, `user@host`, is refused, so no `Authorization` is sent). Plain HTTP only to this machine's loopback (the host parsed), for the tests, never through a proxy.
- The manifest's `version` (a leading `v` taken off) is compared with this build's by semver precedence (a pre-release is older than its release; build metadata does not count). A newer one is a toast with "Open Release Page" and, for the rest of the run, an item "Open the Shell++ X.Y.Z Release Page" in Help; the page is the manifest's `url` when it is `https://`, else the release tag's page of the repository. Nothing is downloaded or installed; the top bar's update button of UI 1.1 is not built.
- A manual check says its result (an update, the latest version, or why it failed); the check at launch shows only an update.
- The HTTPS client is `ureq` 3.5 (MIT OR Apache-2.0) with rustls and webpki-roots (CDLA-Permissive-2.0), and `semver` (MIT OR Apache-2.0); `cargo deny check licenses` passes.
- Tests run against a server of the test on this machine (no network): newer, not newer, a broken answer, no answer, only https or this machine, and no request at all while the check is off or a day has not passed.

**Spec impact:** `docs/ui-behavior-spec.md` 1.1, 1.4 item 10 and 12.7 (and `docs/ru`), `docs/data-model.md` 16.1, `docs/architecture.md` 10.1 and the identifiers table, `docs/security.md` (the update check), `docs/export-format.md` 1 and 4.

### 98. A forwarded connection lets the server close its channel (run 6)

Status: run 6 decision, 2026-10-22 (plan, section 8: the soak test found it).

**Decision:**

- The soak run (`scripts/soak.ps1`) grew about 4 MB a minute with forwarding traffic alone: about 560 forwarded connections a minute through a local and a dynamic rule, a few KB each. Cause: russh 0.64.1 (the latest) keeps the client's entry of a channel the client closes first. Our `ChannelStream` closed each channel on drop, right after the copy; the server's CLOSE then found the channel gone from the session's list and returned before removing the client's entry (its sender and queue).
- A forwarded connection (local, dynamic, remote) now copies through the channel's halves, reading the channel message by message (russh's reader closes the channel's queue at the EOF, so the server's CLOSE could not be seen after it), sends its EOF when the socket ends or fails, and, when both directions are done, waits up to 5 s for the server's CLOSE; the client sends its own CLOSE only when the server stays silent, or when the connection is dropped mid-copy (a stopped rule; the Remote rule's relay starts before the stop is looked at), as before. OpenSSH and the russh test server close a channel once both sides sent EOF.
- Left (for the owner, run report): the same close-first pattern remains in agent forwarding (one channel per use of the agent on the server) and exec channels, at low volume; a one-line patch of russh (remove the client's entry before the early return) would cover every channel type, as a `[patch]` of the dependency and a pull request upstream.
- Checked by the soak's load of forwards alone: before, private bytes 133 to 194 MB in 15 minutes; after, flat at 133 MB with the same traffic. The forwarding tests of `shellpp-ssh` and of the session layer pass unchanged.
- Not changed: session, SFTP and exec channels open once per tab or task, not per connection.
- Review (run 6, three rounds): the channel is read until the server's CLOSE whatever else waits: the upload after the server's EOF, and our EOF. russh's session loop hands each message to the channel's queue (100 places) and waits while it is full, taking nothing else meanwhile; a window adjustment is queued at each WINDOW_ADJUST (dropped when full), and our EOF needs a place in the session's own queue (10), which that loop empties. An unread channel could so stop every tab of the connection: after the server's EOF once it closed mid-upload, and when our EOF went out unread after a failed write. Our EOF, the upload's end and the wait for the server's CLOSE are now one loop that reads the channel. A socket that fails (a reset, a write that cannot go through) ends the connection at once: our EOF, the wait of 5 s, our CLOSE, as `copy_bidirectional` did; before, the relay waited for the server's EOF, which a destination that ignores the half-close never sends. The upload owns the channel's writer, so a stopped upload keeps no place in russh's queue and not the window's lock. The server's CLOSE ends the connection without waiting for the local side's end. A dynamic rule whose reply to the SOCKS client fails sends its EOF and waits for the server's CLOSE the same way instead of closing first. A Remote rule connects to its destination here before it accepts the server's channel, and refuses the channel when the destination is unreachable (connect failed, as OpenSSH does) or the rule stops meanwhile (administratively prohibited): before, the accepted channel stayed unread for up to 10 s of connecting. Tests: an upload after the destination's end, the server's CLOSE with the local side still open, a reset local connection with the destination still open, a reset local connection that stopped reading a flood while it kept sending (the SSH connection jams first; it failed on the second round's code), a Remote connection to a closed port (the second and third failed before the first round). Left: a local reset goes unseen while the upload waits for window with its buffer full and the server sends nothing (tokio's copy reads only with room in its buffer), until the server ends the channel; and a local side that stops reading stops the whole SSH connection once 100 messages wait for it, as russh grows the window on receipt.

**Spec impact:** none.

### 99. The journal compacts every 2 s during a flood (run 6)

Status: run 6 decision, 2026-10-22 (plan, section 9). Amends D85.

**Decision:**

- Measured (run 6, plan 9): `seq 1 3000000` typed into an SSH session of the russh test server, release builds, 5 to 7 runs each, interleaved; medians of the runs whose window stayed active: 613ac41 (run 4) 0.58 s with no custom highlighting rules and 0.55 s with 10; 51b9994 (run 5) 0.62 and 0.67 s; the run 6 head 0.63 and 0.61 s. Run 5's 0.68 s against run 4's 0.55 s was a regression, not noise: 886b295 (just before the journal) 0.54 and 0.54 s, a83b47f (the journal and its review fixes) 0.67 and 0.69 s; a83b47f with the journal off (`LIVE_HISTORY` of a million lines) 0.53 to 0.59 s.
- Cause: during a flood the journal compacted every 250 ms. Each compaction moved about 9 500 rows into the journal and freed them in the grid (alacritty frees spare rows over a thousand, and the compaction truncates the rest); the flood's next 9 500 lines then allocated and filled a row of the grid each, while the journal gave up a line per new line. About a quarter of the run went to that.
- Fix: while output keeps coming, the grid gives its rows to the journal at most every 2 s (`COMPACT_INTERVAL`); it still does 100 ms after the output pauses. Measured on the head: 0.52 to 0.58 s, as before the journal. During a long run of output the grid holds up to the whole scrollback for 2 s at most, as it did for the whole flood before D85.

**Spec impact:** none.

## The owner's answers to run 6

### 100. The owner's answers to the questions of run 6

Status: accepted, 2026-10-23 (owner's choice; the six questions in section 3 of the run 6 report, recorded as section 2 of the plan of run 7). Amends D96 (its known limit at 175 and 200 %), D97 (the top bar's update button) and D98 (the russh patch left for the owner) where they differ; items 2 to 6 go into the plan of run 7.

**Decision:**

1. The names of hosts that are gone stay in a workspace as D94 has them: kept while the workspace links the host, and an export writes only the names of the hosts the file carries.
2. Zoom of 175 to 200 % in a narrow window (D96): when the details panel does not fit beside the main area, it opens over the main area (as the sidebar collapses in a narrow window), with a shadow, and Esc closes it. A screenshot at 960 × 600 and 200 %.
3. The channel leak of russh (D98):
   - the workaround in forwarding stays;
   - the one-line patch of russh goes into `third_party/russh`, used through `[patch.crates-io]` by path, with a leak test for exec channels and agent forwarding; the license is kept, and `THIRD_PARTY_NOTICES.md` notes the patch;
   - the text of an issue and a pull request for upstream (description, reproduction, patch) goes into `docs/upstream/russh-channel-leak.md`; the owner sends them;
   - once upstream releases a fix, the patch is removed (a note in `CLAUDE.md`).
4. The update check (D97): an update button in the top bar (UI 1.1), shown only when a check found a newer version.
5. WSL:
   - `wsl.exe --list` runs in a job object, and the timeout kills its whole process tree;
   - the shells are detected lazily, only when a local terminal or the shell settings are opened, not at every app start; the result is kept until the next start.
6. The soak test on OpenSSH (`scripts\soak.ps1 -Minutes 120`, with SFTP and `htop`) runs at the end of run 7 if Docker works and more than 2.5 hours remain before the deadline; otherwise it goes into the report.

**Spec impact:** none now. With the implementation (run 7): `docs/ui-behavior-spec.md` 1.1 (item 4), 1.2 and 1.3 (item 2) and 12.4 (item 5), and `docs/ru`; `THIRD_PARTY_NOTICES.md`, `CLAUDE.md` and `docs/upstream/russh-channel-leak.md` (item 3).

## Run 7 of 2026-10-23

The decisions below were taken during the owner's seventh long run (its plan, section 1), marked "run 7"; the disputable ones are listed for the owner in the run report.

### 101. The server's databases without sqlx (run 7)

Status: run 7 decision, 2026-10-23 (plan, section 3.1). Amends the stage 0 choice of `sqlx` in docs/sync-protocol.md, section 14, and docs/architecture.md, section 8.2.

**Context.** The sync end-to-end tests run two clients of `shellpp-core` against the server in one process (plan, section 4; docs/architecture.md, section 9.1), so the server's test build links the client's `rusqlite` 0.40, which links `libsqlite3-sys` 0.38. `sqlx-sqlite` needs `libsqlite3-sys` below 0.38 (0.8 and 0.9 alike), and Cargo allows one crate linking `sqlite3` per build.

**Decision:**

- SQLite through `rusqlite` 0.40 (bundled, the client's version) and PostgreSQL through the synchronous `postgres` client 0.19 (MIT OR Apache-2.0), behind one SQL layer (`server/src/db.rs`): statements written once with `?` placeholders (numbered for PostgreSQL), every value typed by the query, each store operation a closure over one transaction on a blocking thread. Write transactions take the database lock in SQLite (`BEGIN IMMEDIATE`) and the vault row in PostgreSQL (`SELECT … FOR UPDATE`); reads are one snapshot (`REPEATABLE READ`).
- In SQLite, UUIDs are 16-byte blobs, times RFC 3339 UTC text with milliseconds (which sorts in time order), booleans 0 or 1 and JSON text; the two migrations (`server/migrations/{postgres,sqlite}`) hold the same schema.
- `shellpp-server migrate` applies them in order (one transaction per step, a `schema_migrations` row each); the first creates the server generation. `serve` refuses another schema version.

**Spec impact:** docs/sync-protocol.md, section 14 (crates), docs/architecture.md, section 8.2 (server row).

### 102. Sync client rules settled while building stage 7 (run 7)

Status: run 7 decision, 2026-10-23 (plan, sections 3.2-3.4 and 5). Clarifies docs/sync-protocol.md, sections 4.4, 5.1, 6.5-6.9, and docs/data-model.md, section 18.5; found by the run's reviews.

**Decision:**

- **A stale server version never wins.** A server version (pulled, or `current` of a push conflict) whose revision is below the revision this device synced, or below the revision of the base envelope a pushed-back record keeps, is stale: the local version is re-sealed above it and pushed again, with no conflict copy, whatever rules 2-5 of section 6.7 would say. Otherwise a server that forces a resync could bring back an old version of a revoked `known_host` or a member pin through rule 4.
- **Only authenticated versions count as received.** The list of received records of a full resync and the server revisions of a restore are built only from versions that opened; the restore applies the snapshot first, then marks the synced revisions (section 6.9, steps 1 then 2), and refuses a snapshot the page limit cut off. A `conflict` without `current` is refused as an integrity failure.
- **The merge at sign-in keeps near-duplicates.** A local record is dropped in favour of an account record only when its data, with the references already mapped to account records, equals the account record's; a record that only shares the natural key (for example a host in another group) is kept with a new id and raised as a duplicate pair for "Merge / Keep both" (UI 13.7). The pre-merge backup of the database is written in `backups/`.
- **Signing in to a rolled-back server.** When the account bundle reports a personal epoch below the local one and its `kek_wrap` opens to the key this device holds for that epoch, the device signs in (the account row and tokens are stored) but no key row changes, and it stops with "server state rolled back"; the re-post (section 5.1) is then possible without a second sign-in. A wrap that opens to another key changes nothing.
- **Push batches.** A cycle pushes every waiting batch (500 items each); only the rounds that re-push after conflicts are capped at three.
- **Recovery key reissue of a bound vault** shows the new key after the server committed it (the local flow of stage 1 did the same); the "I saved it" confirmation before sending (section 5.3, step 4) is left to the interface's next pass.
- **Rotations through the server** rewrap the `device` row when "remember on this device" is on (the keychain is read before, under the `remember` lock).

**Spec impact:** docs/sync-protocol.md, sections 6.7 (a rule 0 before rule 1), 6.9 (step order, authenticated lists), 5.1 (sign-in to a rolled-back server), 5.3 (key shown after commit); docs/data-model.md, section 18.5 (near-duplicates).

### 103. Server limits and a resync after the run 7 reviews (run 7)

Status: run 7 decision, 2026-10-23 (plan, section 5: findings of the authentication review handed over first, fixed after the run's goal check). Clarifies docs/sync-protocol.md, sections 6.9, 10.1 and 10.2.

**Decision:**

- **Client IP.** An `X-Forwarded-For` element may carry a port (`ip:port`, `[ipv6]:port`, as some proxies write it); an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`, an IPv4 client of a server bound to `[::]`) counts as the IPv4 address, for the trusted-proxy CIDRs and for the rate-limit key. Before, a proxy that added the port made the client's forged element the key. The walk from the right stops at the first element that is not a trusted proxy; one that does not parse (`unknown`, a zone id) ends it at the last trusted hop instead of reaching the elements the client wrote.
- **The auth limit before the body.** The per-IP limit of the unauthenticated auth endpoints runs before the body is read, and these endpoints take a body of at most 1 MiB (`limits::MAX_AUTH_BODY_BYTES`); before, a request over the limit still cost the parse of up to 16 MiB.
- **Bogus bearer tokens.** A bearer token the server never issued (unknown or malformed) counts a failure for the client IP, an IPv6 client by its /64; after 60 in 15 minutes (`SHELLPP_RATE_LIMIT_FAILED_TOKENS`) the next authenticated request from that address is `429` without a database lookup, which is a write transaction (the global lock on SQLite). An expired token, or one of a revoked device or a disabled account, is the ordinary `401` and does not count: every client meets it when its access token runs out, so counting it locked out many devices behind one address (found by the review of this change).
- **A full resync repairs a server rollback.** After a complete pull, a full resync marks the synced revisions as a restore does (section 6.9, step 2): a record the server holds at a revision below the one this device synced (a server restored from a backup while the device was signed out, when the rebind had no generation to compare) is pushed again, an edit or a deletion alike. The mark (here and in a restore) only ever lowers a synced revision: a server revision above it is a version the pull received without applying it (a type that is off, a refused key epoch, a payload that did not open), and raising the mark would take a pending local edit out of the push for good (found by the review of this change).

- **Badges in list rows.** A conflict badge in a list row sits in the name cell; where the title would be cut below 56 px the badges (conflict, and a known host's "revoked") take a compact 20 px form, and the title truncates first, so nothing reaches the next column (found by the review of the badges).

**Spec impact:** docs/sync-protocol.md, sections 10.1 (client IP, the auth limit before the body, the bearer token row) and 10.2 (the auth body limit); section 6.9 is unchanged in substance (the mark of step 2 now also follows a full resync); docs/ui-behavior-spec.md, section 13.6 (badges in list rows, snippet packages).

## Run 8 of 2026-10-23, agent B

The decisions below were taken by agent B of the owner's eighth run (import and new features; its plan, section "Agent B"), marked "run 8"; the disputable ones are listed for the owner in `docs/overnight/REPORT-B.md`.

### 140. PuTTY sessions are imported through a session source (run 8)

Status: run 8 decision, 2026-10-23 (plan, item B1). Settles the PuTTY fields of D21 and UI 2.5.

**Context.** UI 2.5 lists PuTTY sessions as a source read from the Windows registry, and docs/architecture.md, section 3, has the desktop app read the registry and pass the values to the core. Which values are imported, what happens to settings the host model cannot hold, and how the registry is kept out of tests were not settled.

**Decision:**

- The core reads sessions through a `SessionSource` trait (`shellpp_core::import::putty`): a session is its registry key name (with PuTTY's `%XX` escapes) and its `REG_SZ` and `REG_DWORD` values. `shellpp-desktop` implements it over `HKCU\Software\SimonTatham\PuTTY\Sessions` on Windows; tests, the demo, a test run and other systems use a fixed list. A missing key is "no sessions"; a key that cannot be opened is listed as not read.
- The preview and the summary are those of the `~/.ssh/config` import (a session is a host whose alias is its name, its `PublicKeyFile` a key file with the same passphrase field). `Default Settings` is a template and never a host. `import_key` is `putty:<session name>`, matched case-insensitively as registry key names are.
- Mapping: `HostName` (as in PuTTY, a `user@` in it wins over `UserName`, and a single `:port` after the name is dropped), `PortNumber`, `UserName`; `Protocol` `ssh` or `telnet` (a Telnet host with `telnet.port` and `telnet.username`); `PublicKeyFile` (imported into the Keychain and linked); `ProxyMethod` SOCKS 5 and HTTP (with `ProxyUsername` and `ProxyPassword`, the password stored as the host's secret; the `ProxyType` of PuTTY before 0.71 is read too); `PortForwardings` as forwarding rules that start on connect, except one that listens beyond loopback (PuTTY's "accept connections from other hosts" makes a bare port listen on all interfaces); `AgentFwd` as forwarding of the system agent (Pageant, D60); `PingInterval` (minutes) plus `PingIntervalSecs` as the keepalive when it is not 0 (0 leaves the app's default); `Environment`; `BackspaceIsDelete 0` as `^H`; `LineCodePage` as the WHATWG label of the same character set ("Use font encoding" and code pages without a label stay UTF-8, noted).
- The proxy follows PuTTY's own choice: a host PuTTY reaches without it (a local name, `127.*` or `::1` unless `ProxyLocalhost`, or a match in `ProxyExcludeList` with PuTTY's matching) is imported without the proxy, with a note.
- Not imported, with the reason: the `raw`, `rlogin`, `serial`, `supdup` and `bare` protocols, a session without a host name, a SOCKS 4, Telnet or SSH proxy or a local proxy command (never run or stored, counted with the commands of the summary), and a Telnet session with a proxy (Telnet connections of this app have no proxy). Such a host is refused rather than imported without its proxy, so it never connects directly (the rule of D44 for jump hosts). A tunnel the forwarding rules cannot hold (a host name as the listen address) is noted, never dropped silently. X11 forwarding, a remote command, a detached certificate and a user name taken from the local user are listed in the host's notes.
- Session names are UTF-8 in recent PuTTY versions; older ones escaped the bytes of the Windows ANSI code page, which the registry source reports (`HKLM\SYSTEM\CurrentControlSet\Control\Nls\CodePage\ACP`), so such names are read in that code page.

**Spec impact:** docs/ui-behavior-spec.md, section 2.5 (PuTTY item, CSV alone stays disabled), and `docs/ru`.
### 141. CSV import with a column mapping (run 8)

Status: run 8 decision, 2026-10-23 (plan, item B2). Extends the CSV columns of D21 and UI 2.5.

**Context.** UI 2.5 fixes the CSV columns (`label, address, port, username, group, tags`) and data-model 18.4 the matching by (address, port, username). The run plan asks for a mapping dialog (which column holds what, with `notes` too), detection of the delimiter and the character set with a way to change them, a warning for passwords and a template to save. Real files come from spreadsheets and other managers: other names, other orders, semicolons, Windows-1251.

**Decision:**

- After the file step, a mapping step shows the character set and the delimiter (both detected, each a menu), "First row is a header", and each column with its role and its first values. Roles: not imported, label, address, port, username, group, tags, notes, password; each role at most one column (choosing it moves it), and Next needs an address column.
- Detection: a byte order mark (UTF-8, UTF-16 LE or BE), else UTF-8 when the bytes are valid UTF-8, else Windows-1251 when at least 70 % of the bytes above ASCII are in 0xC0-0xFF and at least two thirds of them stand next to another such byte (Cyrillic words), else Windows-1252 (its accented letters stand alone: "Müller"). The delimiter is the one of `,` `;` tab `|` that splits the first 20 lines into the same number of fields (more than one), the most fields winning. The first row is a header when it names a known column (English and Russian names and common synonyms: name, host, hostname, ip, user, login, folder, description, имя, адрес, порт, пользователь, пароль…). A first row that names none but looks like a header (no port number, IP address, `.` or `@` in it) is a header whose columns are all "not imported" until the user maps them, so a password column is never taken for another one by position; otherwise the columns take the SPEC's order. Fields follow RFC 4180 (quotes, doubled quotes, line breaks inside quotes). A file that ends inside a quote is refused (its last field would hold the rest of the file), and so is one over 16 MiB or 100,000 rows (never cut silently).
- `group` is a path with ` / ` between levels (data-model 18.4): missing groups are created, existing ones matched by parent and exact name. `tags` are separated by `,` or `;`. `password` is stored as the host's `ssh.password` (a secret). The mapping and the preview show a warning while a password column is mapped or rows carry one ("the file holds passwords in clear text; delete it after the import"), the summary repeats it, and the mapping masks that column's values.
- Matching: a row is "already exists" when a live host has the same address (as stored: brackets removed, case-insensitively), effective port and effective username: the host's own, else its identity's username, else inherited from its groups (data-model 18.4). A row whose group path is new from some level on inherits from the deepest group that exists (the new groups have no settings). Duplicates inside the file are reduced to the first row. Existing hosts are never overwritten.
- "Save template…" in the file step writes `shellpp-hosts.csv` with the header `label,address,port,username,group,tags,notes` and one example row.
- The file's bytes, its records, the mapping's sample and the host data built for the preview are wiped from memory when dropped (they may hold passwords); nothing logs the file's content.

**Spec impact:** docs/ui-behavior-spec.md, section 2.5 (CSV columns and steps), docs/data-model.md, section 18.4 (CSV `notes` and `password`), and `docs/ru`.
### 142. Autocomplete from the host's history (run 8)

Status: run 8 decision, 2026-10-23 (plan, item B4). Settles UI 4.2 and 12.4 "Autocomplete (beta)" and the `terminal.autocomplete` setting (data-model 16.1).

**Context.** UI 4.2 has "Autocomplete: a dropdown of modes, off by default", built from the local history of this host, hidden until its stage (section 0, D28). The run plan asks for it as a beta: Tab or → accepts; it never suggests what the history filter takes for a secret; it does not work in the alternate screen or at a password prompt. How a suggestion is shown, which line it reads and how it is kept away from passwords were not settled.

**Decision:**

- `terminal.autocomplete` takes `off` (default) or `history`. Settings > Terminal (first row of the Terminal block) and the terminal sidebar's Tools tab show the same dropdown, "Off" or "This host's history".
- A suggestion is the rest of the newest command of the host's local history that starts with the typed text and is longer; it is drawn after the cursor in the theme's foreground at 45 % opacity, cut at the end of the row, only while the cursor can be seen (the view at the bottom, the cursor shown, the session live, no IME composition). Tab or → (without modifiers, not a held key) types the rest as typed input (the line is recorded as any typed line) only when the last frame drew that very suggestion; otherwise both keys go to the shell as before. Between a key and the next output (the echo) nothing is suggested or accepted, so a second Tab before the echo is the shell's, never the rest typed twice.
- The typed text is what the history capture reads (security.md 10.1): the text on screen from the line's boundary to the cursor, so a key the server does not echo never counts. A suggestion needs a session to a saved host (only they keep history and have a capture); the shell's line editor active (it turned bracketed paste on at its prompt: bash 5.1 and later, zsh, fish; a pager's search or a program reading a line has it off, and an older shell without it gets no suggestions); a line the capture would record (known boundary, not pasted, not pending, started outside the alternate screen, printable keys typed); the cursor at the end of the line's text and not in the last column; and neither the text before the cursor, nor the line, nor the capture's prompt text (with nothing before the boundary: the last line above) looking like a password prompt.
- Suggestions come from `history_list` of the host (at most 5,000 entries, newest first), read and filtered off the UI thread when the first suggestion is wanted, and grow with the commands the session records (also those recorded while the list loads). A command that starts with a space, spans lines or that the secret detector (security.md 10.2) flags is left out, also when it is in the history (an older entry, an import). The list is dropped and wiped when the app locks, when records change (history may have been deleted) and when the terminal leaves its session; it is read again when next needed.

**Spec impact:** docs/ui-behavior-spec.md, sections 0, 4.2 and 12.4; docs/data-model.md, section 16.1; `docs/ru`.


### 143. FIDO2 security keys sign and are made through the system's authenticator (run 8)

Status: run 8 decision, 2026-10-23 (plan, item B3). Implements item 8 of D50 and the signing left open in D37.

**Context.** D37 imports `sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com` keys without signing; D50 chose the WebAuthn API on Windows and `ctap-hid-fido2` on macOS and Linux. No security key is at hand during the run: the work is checked at the level of formats, against a software authenticator and the test server, and with a key in the morning check list.

**Decision:**

- `shellpp-ssh` signs through an `Authenticator` trait (`sk.rs`): the request carries the key's application (relying party id, `ssh:`), key handle, flags and the data to sign; the authenticator signs its authenticator data and SHA-256 of the data (its client data hash), as OpenSSH's PROTOCOL.u2f describes. The SSH signature is the Ed25519 signature, or the ECDSA one converted from DER to `mpint r, mpint s`, followed by the flags and the counter; it is checked against the key's public key before it is sent. Without an authenticator, or when it is not ready for the key (no device, several devices over HID, a PIN key over HID), a security key is skipped before it is offered and the next method runs, as before. Once the server accepted the key a failure cannot be passed over (the server waits for that signature): a cancelled touch ends the attempt (`Cancelled`), another failure as a key that cannot be used. An encrypted key file's passphrase is offered for saving as for any key. Connections without a tab (forwarding at launch, snippets on many hosts) get no authenticator, so no dialog comes up unasked. The default key files `~/.ssh/*_sk` stay out (each would ask for a touch at every connection).
- Windows: the WebAuthn API of `webauthn.dll` (Windows 10 1903 and later), loaded at run time from System32 (`LoadLibraryExW` and `GetProcAddress`, `windows-sys` types; a load-time import would keep the app from starting without the DLL); `unsafe` only in `apps/desktop/src/platform/security_key.rs` with SAFETY comments, listed in CONTRIBUTING.md. The data to sign is passed as the "client data", which the API hashes with SHA-256, the credential as the only allowed one, user verification as the key's flag asks; the system shows its own Windows Security dialog over the active window. The calls that show the dialog (sign and make) are tested only when `SHELLPP_TEST_WEBAUTHN=1` (a person with a key); loading the DLL, the API version and the error mapping are tested always. The dialog belongs to the active window; it cannot be cancelled from the app (that needs options version 3), so after a closed tab it stays up to its 60 s timeout.
- macOS and Linux: CTAP2 over USB HID through `ctap-hid-fido2` 3.6 (MIT, ring backend) behind the desktop's `fido2-hid` feature, **off by default**: it builds `hidapi`, which needs libudev on Linux (added to the CI packages for the `--all-features` steps), and nobody could check it with a key. It asks the one FIDO device plugged in; a key that requires a PIN is skipped (no PIN prompt yet). The owner turns the feature on for release builds after a check.
- Key generation: "Create on the security key" in the FIDO2 form, where an authenticator is there: WebAuthn MakeCredential, or CTAP2 makeCredential over HID, with EdDSA first and ES256 second, a non-discoverable credential, user presence required; the COSE public key is read from the attested credential data and the OpenSSH key file (application, flags, key handle) fills the form, imported as any key file.
- A test run never reaches an authenticator. Nothing logs key handles, data or signatures.

**Spec impact:** docs/ui-behavior-spec.md, section 7 (FIDO2, the tooltip), and `docs/ru`.


### 144. Script generation through an OpenAI-compatible API (run 8)

Status: run 8 decision, 2026-10-23 (plan, item B5). Settles UI 9.5.

**Context.** UI 9.5 asks for script generation off by default, with the user's own API key or a local model, the description sent only on the button, the destination shown, and the result only inserted into the Script field. The run plan adds: the provider is configurable (an API key, the address of an OpenAI-compatible API, or a local model), tests on a fake provider, no real requests. Settings are plaintext and must not hold secrets (data-model 16.1).

**Decision:**

- One provider kind: the OpenAI-compatible `POST <address>/chat/completions` (OpenAI and most hosted services, Ollama, llama.cpp and LM Studio locally). The settings are `snippets.generation` (`off` by default), `snippets.generation_url` (empty: `https://api.openai.com/v1`) and `snippets.generation_model`. The address must be `https://`, or plain `http://` to this computer's loopback (a local model, never through a proxy), with no user part; no redirects are followed.
- The API key is kept in the OS credential store (Windows Credential Manager, the macOS keychain, the Secret Service) under one entry per host (`api-key:<host>`), never in a setting, the vault or an export; it is sent only as the `Authorization: Bearer` header to that host, so a key saved for one service never goes to another after the address changes. It is read and written off the UI thread. A test run and `--demo` keep keys in memory; an unsigned macOS build has no store for them (D7).
- The generation settings belong to the device: Import Data never applies them, not even a full restore (D41, D50): a file never chooses where descriptions go.
- The snippet form reads the settings again when Generate is pressed: a row left open while Settings turned generation off sends nothing.
- The request holds a fixed system instruction (a shell script only, `{{name}}` for values to fill, never passwords or keys) and the description (at most 2,000 characters), temperature 0.2; the answer's first choice is the script, a Markdown fence around it removed. Answers over 256 KiB are refused; the timeout is 120 s (local models are slow).
- The form shows the row only while generation is on with a model; the caption names the host the description goes to. Generate runs off the UI thread; the script goes after what the Script field holds and is never run or saved by itself.
- Tests use a fake provider through an override; no test sends a request.

**Spec impact:** docs/ui-behavior-spec.md, section 9.5; docs/data-model.md, section 16.1; docs/security.md (A3, TB2); `docs/ru`.

## Run 8 of 2026-10-23, agent C

The decisions below were taken during the owner's eighth long run, by its agent C (numbers 170-199), marked "run 8"; the disputable ones are listed for the owner in the run report.

### 170. Signed releases and a signed update manifest (run 8)

Status: run 8 decision, 2026-10-23 (plan, C4). Amends D7 and D97 for the GPUI app: the manifest the update check reads is signed.

**Context.** D97 made the update check a notice that reads `latest.json`; anyone who can change that file (a taken-over release page, a changed update source, a hostile network path to a custom source) could announce a "new version" with a page of their choice. The plan asks for signed release files, the key generated by the owner, and the app checking the manifest's signature before it shows anything.

**Decision:**

- **minisign** (Ed25519, ISC license) signs every release file, `latest.json` and `SHA256SUMS` included, as `<file>.minisig` with the trusted comment `shellpp <version> <file>`. The signature is the prehashed kind (BLAKE2b-512 of the file), the default of minisign 0.10 and later; the legacy unhashed kind is refused.
- The key pair is the owner's (docs/release.md): the secret half only in the secrets `MINISIGN_SECRET_KEY` and `MINISIGN_PASSWORD` of the GitHub environment `release`, the public half in `release/minisign.pub`. The release workflow verifies every signature with that file before it creates the draft, so a secret that does not match it stops the release.
- `.github/workflows/release.yml`, on a `v*` tag: the tag must equal the app's and the server's version; the Windows portable zip (scripts/package-windows.ps1), the server image (`docker save`) and its static Linux binary (copied out of the image), `latest.json` (`version`, `url`, `published`, `files` with SHA-256 and size), `SHA256SUMS`, the signatures, and a **draft** release (`gh release create --draft`); no build cache; nothing is published or pushed to a registry.
- The app builds `release/minisign.pub` in. The update check fetches the manifest and `<manifest URL>.minisig` (before a query string), verifies the signature and its trusted comment with ed25519-dalek and BLAKE2b-512 (RustCrypto `blake2`, both already in the tree), requires the trusted comment to be the workflow's `shellpp <version> latest.json` with the manifest's version (another signed file of a release is not a manifest), and only then compares versions. A mismatch asks for both files once more (a release published between the two requests); a signature answered with 5xx or 429 is "did not answer". The key file has minisign's own layout (the comment on line 1), checked by the workflow and the app alike. A missing or wrong signature is "the update is not signed with the Shell++ release key"; a build whose `release/minisign.pub` holds no key asks nothing and says "this build has no key to check updates with". A custom update source changes where the manifest comes from, never the key.
- Tests: vectors made by minisign 0.12 itself (a throwaway key whose secret half was deleted, `apps/desktop/src/updates/testdata`), a round trip with a key generated by the test, and refusals of a changed manifest, a changed trusted comment, flipped signature bits, another key (and another key under the same key id), the legacy kind, a replayed signature of another manifest, a missing or oversized signature.

**Spec impact:** `docs/security.md` 13.4, `docs/architecture.md` 10.1, `docs/release.md` (new). Owner's action: generate the key and fill in `release/minisign.pub` and the two secrets (docs/release.md, "The signing key"); until then the release workflow stops at its first job and the app reports that it cannot check updates.

### 171. Limits of the files an import reads (run 8)

Status: run 8 decision, 2026-10-23 (plan, C5: the security review of the local core). Adds to D35 (`~/.ssh/config`) and docs/export-format.md, section 4 (Import Data).

**Context.** The review found two imports that a file from elsewhere (security.md, A10) could make run out of memory or never finish: an Import Data file of tiny values (`[0,0,…]`: gigabytes as a tree, and every broken record kept as a refusal without a limit), and an `~/.ssh/config` whose `Include` lines fan out (one argument per missing file, a file included under another spelling, no limit across files) or that lists so many hosts that matching each host against every block takes hours.

**Decision:**

- Import Data counts the JSON values of the file before it parses it (`,`, `:`, `[` and `{` outside strings, plus one): more than 8,000,000 is refused as "too many" (a real export spends well over 8 bytes of text per value, so 64 MiB of export stays within it). Every element of a vault's `records` counts against the 500,000-record limit, a refused or repeated one too.
- `~/.ssh/config`: a file is never read twice in one `Include` chain, recognized by its canonical path (so `../.ssh/config`, `CONFIG` or `config.` on Windows is the same file: "Include loops or nests too deep"); the same file included from two `Host` blocks is read for each, as `ssh` does. Limits over the whole import: 1,000 file reads, 16 MiB read, 256 `Include` arguments, 10,000 folder entries looked at by `Include` patterns (names are matched before the file system is asked), 10,000 `Host`/`Match` blocks, words of 4,096 bytes, `Host` patterns and expanded `HostName`s of 1,025 bytes (`NI_MAXHOST`), 100 values of a keyword that adds up per host (`ssh` reads at most 100 identity files), and 1,000 problems; past a limit the reading stops with "the file is too large", which is always kept.
- Matching is indexed and budgeted: a `Host` block whose patterns all name hosts is looked at only for those names, so a generated config costs one block per host; every other block (patterns, `Match`, global) is looked at for every host. Each block looked at costs its patterns' length and its options; past 50,000,000 the remaining hosts are left out of the preview and the import, with the same problem. A pattern without `*` or `?` is compared without copies.
- A key file named by `IdentityFile` is read only when it is a regular file (a FIFO or a device would block the read).

**Spec impact:** docs/export-format.md, section 4; the D35 limits (this entry).

### 172. A host from a file does not forward the agent (run 8)

Status: run 8 decision, 2026-10-23 (plan, C5). Extends D63 (imported forwarding rules do not start by themselves) to Agent Forwarding (D60).

**Context.** With Agent Forwarding on, a host can sign with the user's keys elsewhere while a session is open (security.md, RR19); the mitigation is that it is off by default and the user's choice per host. An Import Data file from someone else could turn it on for a host it defines.

**Decision:** Import Data turns Agent Forwarding (and "Also offer the system agent's keys") off on every host of the file, and counts the hosts it adds that way (a new host or an imported copy) in the summary ("Hosts imported with Agent Forwarding turned off"), unless the user chose "Full restore from my backup". Such a host is compared with the records here without these flags, like the autostart flags of a rule (D63): a host that is the same here stays "already here" with the user's own setting, and the next import of the same file finds the host or its copy again instead of adding another. The `~/.ssh/config` import keeps mapping `ForwardAgent` (D60): that file is the user's own config.

**Spec impact:** docs/export-format.md, section 4 (the import rules); the user guide, chapter 7.

## Run 9 of 2026-10-24

The decisions below were taken during the owner's ninth long run (its plan, section 1), marked "run 9". D173 records the owner's answers given before the run; the disputable decisions of the run are listed for the owner in the run report.

### 173. The owner's answers before run 9 (run 9)

Status: accepted, 2026-10-24 (owner's choice; the plan of run 9, section 2: the findings of the run 7 report, section 10.1, the questions of ROADMAP section 4 and those of run 8's agent C). Amends D102 (the order in which a reissued recovery key is shown) and the stage 0 default of "My old password may be compromised" (docs/sync-protocol.md, section 5.1). The parts marked "run 10" or "stage 8" are carried out there.

**Decision:**

1. Run 7 review findings (run 7 report, section 10.1):
   1. The Argon2 verification moves out of the database transaction now.
   2. TOTP: instead of the fixed window of SP 10.1 (10 failures in 15 minutes per account), a growing delay per account and client IP, and a soft limit per account that never locks sign-in out entirely. The delay is an answer "retry after N seconds", never a wait inside the request.
   3. The per-user lock in the quota step: stage 8.
   4. `B64` and `Secret` of `crates/sync-api` are wiped on drop now; copies inside reqwest and hyper are a known limitation (security.md).
   5. Prelogin on a bound, signed-out device with weaker KDF parameters (still above the floor) than the local ones: refused with a warning about a possible downgrade attack; going on only after an explicit confirmation in a dialog of its own.
   6. The merge at sign-in compares in two passes and leaves no extra duplicate pairs.
   7. A forged `key_epoch`: an error with the action "Reset sync state of this vault" (a full resync that leaves local data untouched, after a confirmation); the forged epoch is never accepted.
   8. The `roster_entry` type rule: stage 8.
   9. The server secret is read from a file (`SHELLPP_SERVER_SECRET_FILE`, as Docker secrets provide it); the environment variable stays for compatibility, with a warning in the log; copies inside `totp-rs` are a known limitation.
   10. The empty `CHANNEL_DATA` of russh: a second patch in `third_party/russh` with a test, and the text for upstream in `docs/upstream/`.
   11. A device back after a long absence: the server version that the device pushes its own over is never lost; it is kept as a conflict record on every device.
   12. The duplicates hint hidden for viewers and archives: stage 8.
   13. The duplicates hint on Keychain, Port Forwarding, workspaces and themes: run 10.
   14. `keys_merge` in two transactions: accepted (a failure of the second step leaves both keys; nothing is lost).
   15. f6d74c5 (run 7) is already in main, as is f3e1c2b (run 8).
   16. Natural keys as data-model 18.6 defines them: the effective port and username, inherited from groups, and references compared after they are rewritten.
   17. The server's `recovery_wrap` is checked in every cycle until the check passes, also after a recovery key reissue.
   18. An abandoned sign-in merge: the extra vault is removed when the user gives up the sign-in.
   19. The `failed` set and `stale_key_epoch` follow the table of SP 6.5 exactly.
2. The recovery key and the password change:
   - Keeping and showing the key stay as the specifications have them (the `recovery_key` record; Show and Reissue in UI 13.4; the key at registration, UI 13.5, step 4); only the code is missing.
   - A reissued key is shown and confirmed ("I saved it") before anything is sent (SP 5.3, step 4). This replaces the interim order of D102, which showed the key after the server committed it.
   - A password change, the rotation of the personal vault key and a recovery key reissue go to the server as one request applied in one transaction, with everything SP 5.1 and 5.3 list for them: the new `recovery_wrap`, the new verifier of `recovery_auth_key` and the new revision of the `recovery_key` record checked against its `base_revision`. A failed request changes nothing on the server, and the user is told that the old password and key still work. `/account/password` and `/account/recovery-key` stay for their own cases.
   - "Lost or stolen" changes the password and reissues the recovery key through that request.
   - "My old password may be compromised" now also reissues the recovery key, through the same request, and is on by default (the stage 0 default of SP 5.1 was off). Turned off, the change rewraps without a rotation and the recovery key stays.
   - Without an account (local mode) the option is on by default too and does the same without a server: the rotation of SP 5.1 with the recovery wrap of its step 5, and a new `recovery_key` record whose key is not shown. Whoever knew the old password could read the old record, which would become the account's key at registration.
   - The other devices are signed out as in SP 5.1; after signing in with the new password they go on syncing.
   - "Remember on this device" after a rotation made on another device: the `device` wrap is made again with the device key from the OS key store; without that key the option is turned off with a clear note; never on a revoked device.
3. The server certificate behaves as SP 2.1, UI 12.2 and UI 13.5 have it: the trust choice when the issuer is unknown, a CA added in that dialog, the changed-certificate dialog with all its buttons, a confirmation that goes on without signing out. Beyond the specification only a row with the current trust in Account & Sync (UI 12.2). An added CA counts for the sync server only, never for SSH or the system.
4. Run 8, agent B (ROADMAP 4.3): `fido2-hid` stays off on macOS and Linux until a check with a real key; CSV keeps importing passwords (the reminder to delete the file exists, D141); autocomplete only in shells with bracketed paste, and script generation with the API key in the OS store and one OpenAI-compatible provider type, are confirmed.
5. Other:
   - Signing in or registering on a device whose personal vault is bound to an earlier account makes the archive of data-model 18.5 instead of refusing. The archive is exported like any vault (storage-format 13.1); viewing it in the interface comes with stage 8, together with the archives of removed members.
   - Publishing the SSH ID again and changing the email are postponed.
   - The behaviour questions of run 8's agent C (its report, section 8) are decided in run 9 by "the safest default; the user can turn it on explicitly"; the test infrastructure questions of that section go to run 10; only the owner makes the minisign signing key.
   - The `wip/salvage-*` branches are kept.

**Spec impact:** with the implementation (run 9): docs/sync-protocol.md, sections 2.1, 5.1, 5.3, 6.5, 6.7 and 10.1; docs/security.md; docs/data-model.md, sections 18.5 and 18.6; docs/ui-behavior-spec.md, sections 12.2, 13.4 and 13.5; `docs/ru`.

### 174. Argon2id checks outside the database transaction (run 9)

Status: run 9 decision, 2026-10-24 (D173, item 1.1; the run 7 report, section 10.1, finding 1).

**Context.** Every sign-in ran its Argon2id check (m = 19 MiB, t = 2) inside its database transaction. On SQLite the server has one connection, so a burst of unauthenticated sign-ins, even with wrong keys for unknown emails, held the database for every other request (a request behind 64 such sign-ins waited for all of them); on PostgreSQL it held pool connections. The fix has to keep the order of the checks, the dummy check of unknown emails and the failure limits, and must not act on a key whose verifier changed meanwhile.

**Decision:**

- `login`, `recover/begin` and every signed-in endpoint that checks `current_auth_key` (email change, TOTP setup and disable, account deletion, a personal key epoch) check the key in two steps, through one helper (`accounts::check_sign_in`, `accounts::check_current_auth_key`, which return a `CheckedKey`): the user row is read in a read transaction of its own, then the failure limit and the Argon2id check run outside any transaction; an unknown email or a key of the wrong length is still checked against the dummy verifier. The write transaction then takes the user row with `CheckedKey::user` (`CheckedKey::recovery_user` for recovery; locked on PostgreSQL), which goes on only while the verifier the key matched (`auth_verifier`, or `recovery_verifier`) is still the stored one. A personal key epoch for a vault that is not the account's personal vault still answers `403 not_member` before the key is checked. A password change, email change, recovery or reissue committed in between answers `401 invalid_credentials` without counting a failure; a user deleted in between answers what it answered before (`unauthenticated` for a signed-in request, `invalid_credentials` for a sign-in).
- A registration makes its two verifiers the same way: its checks (the code on an invite-only server, the request's shape, the email, the vault and the device) run first in a read transaction, so a request that will be refused costs no hash; the write transaction checks everything again and stores the verifiers made before.
- The failure limit per email (section 10.1) counts an attempt before its check and takes it back when the check passes, so checks of one email that run at once never pass the limit together (with checks inside the transaction, SQLite ran them one at a time).
- At most half the CPUs, 1 to 8, Argon2id checks run at once (a semaphore of the server); the others wait without holding the database. The blocking task of a check holds its slot until the check ends, also when the client closed the connection meanwhile. Before, SQLite let only one run at a time and PostgreSQL as many as its pool: without the bound, a burst would allocate 19 MiB per request on as many threads as the blocking pool has.
- Cross review of run 9 (memory and sessions):
  - An email longer than 320 bytes once normalized (a text over 1,280 bytes is not even normalized) is refused at sign-in and recovery like an unknown email, after the dummy check, and is not counted; pre-login answers `422 validation_failed`. No account can have one (registration allows 320).
  - The failure windows keep an email as its SHA-256 (32 bytes, and no email in memory) and hold at most 100,000 keys: past that, ended windows go first, then the oldest down to half. The per-address limit of the auth endpoints keys an IPv6 client by its /64, as the bearer-token window does.
  - At most 64 requests wait for an Argon2id slot, each at most 20 seconds; one more, or one that waited too long, is answered `429 rate_limited` with `Retry-After: 5` instead of holding its body in memory while it waits.
  - The write of a signed-in request checks its session again: an access token of its family on its device, the device not revoked and the account not disabled; else `401 unauthenticated` and nothing changes. A device revoked or an account disabled between the check and the write no longer lets the request through.
- `/account/password` and `/account/recovery-key` kept their check inside the transaction until the merge of the run; then they moved to the same helper, as did `/account/credentials` (D178), and they count failures with the client network (D185). The new verifiers of password changes, email changes, recoveries and reissues are still made inside their transactions: those requests need a valid token or ticket.

**Spec impact:** docs/sync-protocol.md, section 4.4 (server rules).

### 175. TOTP: a growing delay per account and network, and a soft limit per account (run 9)

Status: run 9 decision, 2026-10-24 (D173, item 1.2; the run 7 report, section 10.1, finding 2). Replaces the TOTP row of docs/sync-protocol.md, section 10.1 (stage 0 decision: 10 failures per 15 minutes per account).

**Context.** The fixed window let anyone who knows the password try 40 codes an hour for ever (about 65 % success in a year against a 6-digit code with ±1 step), and, keyed by the account alone, let anyone who knows the email and password lock the owner out for 15 minutes at a time. The owner chose a growing delay per account and client address, and a soft limit per account that never locks sign-in out entirely; a delay is an answer "retry after N seconds", never a wait inside the request.

**Decision:**

- Every TOTP check (login, recovery, enable, disable, account deletion) first asks two counters, both in memory: the account with the client's network (an IPv4 address, the /64 of an IPv6 one, as for bearer tokens, D103) and the account alone. While either delay runs, the answer is `429 rate_limited` with `Retry-After` in whole seconds rounded up, also for the right code, and the ticket's 5 attempts are not used.
- Per account and network: 3 failures free, then 2 seconds, doubled by each further failure, up to 15 minutes (`SHELLPP_RATE_LIMIT_FREE_TOTP`, default 3). A right code from the network starts it over.
- The soft limit per account, from every network together: 10 failures free, then 2 seconds doubling, up to 4 minutes (`SHELLPP_RATE_LIMIT_FAILED_TOTP`, default 10; it named the failures of the fixed window before). 4 minutes, not more, so a login ticket (5 minutes) outlives one wait. A right code does not reset it, so a sign-in of the owner gives an attacker elsewhere no new tries.
- At sign-in (login, recovery) the soft limit does not hold a network that a right code of the account came from in the last 30 days (the 8 most recent per account). Only a right code marks a network: neither a token nor a failure does, so an attacker without the code cannot make one. The owner's networks with a recent code are therefore never locked out, even when an attacker takes every turn of the soft limit. A network without one waits at most 4 minutes per try, but the wait runs from anyone's last failure: an attacker that tries the moment each wait ends can keep taking the turn, so on a new network sign-in may need several tries or the recovery (named for the owner in the run 9 report). Codes of signed-in requests (enable, disable, account deletion) are held by the soft limit on every network: a stolen token gets no way around it.
- Each delay runs from the last failure that counts for it; failures are forgotten 24 hours after the last one. Past 100,000 entries a map drops the forgotten ones, then the oldest down to 50,000 (one prune per 50,000 failures), which only lowers network counters; the account counters stay unless 100,000 accounts fail at once.
- What it gives: from one network 4 codes an hour; from many networks without a recent right code, at most 15 an hour once the soft limit is at its cap (about 33 % success in a year of a lasting attack with the password known, against 65 %), while no try waits longer than 4 minutes because of failures elsewhere. Every TOTP check runs inside the transaction that locks the user row (SQLite: one writer; PostgreSQL: `FOR UPDATE`, also in the recovery now), so two codes of one account are not checked against the counters at the same moment.

**Spec impact:** docs/sync-protocol.md, section 10.1 (the TOTP row and the paragraph after the table).

### 176. The compose deployment: the client address and the backups (run 9)

Status: run 9 decision, 2026-10-24 (the plan of run 9, sections 3.4 and 3.7; run 8's agent C, findings S6 and S7; docs/sync-protocol.md, section 12, "the encrypted backup recipe and a restore from it"). Uses the client address rules of D103.

**Context.** The server README told a homelab with the compose file to set `SHELLPP_TRUSTED_PROXIES=127.0.0.1/32` for a proxy on the host. Docker hands a connection to a published port to the container from the gateway of the container's network (checked on Docker Desktop: the peer is the gateway), so `127.0.0.1` never matched: the server ignored `X-Forwarded-For` and put every client into the gateway's one bucket of the sign-in limits (20 a minute for everyone) and, with D175, of the TOTP delays. `.env.example` had `172.16.0.0/12` instead, which works but believes the header from anything in a /12. The backups: `backup.sh` piped `pg_dump` into `age` without `pipefail`: a dump that failed (database down, wrong password) gave an encrypted empty file, the script said "backup written", and after 30 days of that the good backups were pruned. Reproduced in run 9: exit 0 and a 200-byte `.sql.age` with no database at all. The `backup` service ran a stock `alpine:3.22` that installed `postgresql17-client` and `age` with `apk add` at every start, so the nightly backup depended on the network and Alpine's mirrors. The recipe itself was not tested.

**Decision:**

- The compose file gives its default network a fixed subnet, `172.16.88.0/24` with the gateway `172.16.88.1` (outside the pools Docker hands out by default, 172.17–172.31 and 192.168), and the server's default `SHELLPP_TRUSTED_PROXIES` is `172.16.88.1/32`, as in `.env.example`: exactly the address a proxy on the host (Caddy, `cloudflared`) reaches the server from, and only that. Containers of the project (PostgreSQL, the backup) reach the server from their own addresses and are not believed. A proxy container in the project gets a fixed address on that network and is trusted instead. Both addresses change together when the subnet is taken.
- The README explains this in a section of its own ("Client addresses"), and its Access sections point to it instead of naming `127.0.0.1/32`; the guide (EN and RU), security.md, section 14.4, and architecture.md say the same. The subnet and gateway are `SHELLPP_SUBNET` and `SHELLPP_GATEWAY` in `.env`, set together with `SHELLPP_TRUSTED_PROXIES` when the default is taken. The trust is safe only while the port is published on `127.0.0.1` alone (the README says so); an upgrade from an older `.env` needs `docker compose down` and `up -d` so the network is made again with its subnet (README, "Update").
- The server logs once per process, without addresses, when a request carried `X-Forwarded-For` from a peer it does not trust, so an operator sees that the proxy in front is not trusted.
- The server keeps the rules of D103: the header is read only when the peer is trusted, from the right, past trusted hops; from any other peer it is ignored, so neither the sign-in limits nor the TOTP delays can be dodged by naming another address.
- `backup.sh` runs with `set -o pipefail` (busybox ash has it). A failing dump or encryption removes the `.part`, prints "backup failed" and ends the script with an error, before the prune: no `.sql.age` comes from a failed run and old backups stay. Only a finished file is renamed to `.sql.age`; a `.part` left by a killed run is removed at a later start once it is a day old (a run going on at the same time keeps its own); a failed rename or prune fails the run the same way. In the service (not with `BACKUP_ONCE=1`) a failure waits 5 minutes before it ends, so Docker's restart does not retry in a tight loop; `init: true` lets `docker compose stop` end a running dump. Missing tools are an error that names the image, not a reason to install them.
- The image builds never take a deployment's own files: the root `.dockerignore` leaves out `server/deploy` (the server image needs nothing from it, and its `.env`, secret and backups live there) and any `restore.sql`, and `server/deploy/.dockerignore` sends only `backup.sh`. The restore steps delete the plaintext `restore.sql` after the load. The restore loads the dump with `psql -v ON_ERROR_STOP=1 --single-transaction`, so a partial load fails before `admin restored`.
- The service uses an image built from `server/deploy/backup.Dockerfile` (Alpine 3.22 with `postgresql17-client` and `age`, the script inside), built once by `docker compose up`; the Update steps rebuild it with `docker compose build --pull --no-cache backup`, so the client and `age` get their updates.
- `server/tests/deploy.rs`, under `SHELLPP_TEST_DEPLOY=docker`, runs the recipe: the backup image dumps the server's PostgreSQL through `age`, the dump is decrypted and loaded into a clean PostgreSQL, `admin restored`, the server comes back at the same address; two clients keep their data and notice the restore (`server_generation`), the record made after the backup comes back from the device, and a new device sees everything. A second test runs a failing dump; a third checks the compose file as Compose reads it (trusted gateway, Docker secret, backup image). The server image takes `--build-arg BUILD_JOBS=N` for shared machines.
- The recipe test found that `serve` on PostgreSQL panicked at start ("Cannot start a runtime from within a runtime"): the schema check ran on the async runtime's thread, where the synchronous PostgreSQL client cannot start its own. The start's checks now run on a blocking thread like every other database access. Until then only SQLite had been started with `serve` in tests; the compose deployment (PostgreSQL) could not start.

**Spec impact:** none in the protocol (docs/sync-protocol.md, section 10.1 already says it); server/README.md, server/deploy/docker-compose.yml and .env.example, docs/security.md, section 14.4, docs/architecture.md (environment variables), docs/guide/06-sync.md and docs/guide/ru/06-sync.md. For the backups: docs/security.md, section 14.4 (backups); server/README.md (Backups, Update); docs/guide/06-sync.md and docs/guide/ru/06-sync.md.

### 177. The server secret from a file (run 9)

Status: run 9 decision, 2026-10-24 (D173, item 1.9; the run 7 report, section 10.1, finding 9).

**Context.** `SHELLPP_SERVER_SECRET` in the environment stays in the process environment for the life of the process (removing it from a running multi-threaded process is not safe), shows in `docker inspect` and in the compose file's interpolated configuration. Docker secrets give a file instead.

**Decision:**

- `SHELLPP_SERVER_SECRET_FILE` names a file that holds the base64 secret (surrounding white space and a line break are ignored; at most 4 KiB). The server reads it (a regular file only: a FIFO would block the start) into a buffer allocated once at its largest size and wiped on drop; a byte order mark is skipped; errors name the variable, never the contents.
- `SHELLPP_SERVER_SECRET` keeps working for compatibility, with a warning in the log at `serve` that tells the operator to move the same value into a file.
- Both set is refused at start, even when they agree: no precedence rule, so no one has to guess which of two secrets derives the keys (a wrong one would make every TOTP secret unreadable and change the fake salts). Neither set is refused, naming `SHELLPP_SERVER_SECRET_FILE`.
- The compose file hands the secret to `server` as the Docker secret `server_secret` from `deploy/secrets/server_secret` (ignored by git), readable by the image's user 10001; the README says how to make it and how an existing deployment moves the value the running server uses (`docker compose exec server printenv`, more reliable than `.env`, which the shell environment can override) into it before the upgrade; the guide (EN and RU) points there. Compose refuses to start while the file is missing. `migrate` does not get the variable (it reads no secret).
- `serve` refuses a secret that opens none of the TOTP secrets at rest in the database (it tries up to 5; one that opens is enough), with a message that names the way on: the same secret, or `admin user reset-totp` for each account before a new one. So a regenerated secret never makes every account's TOTP fail silently; a database without TOTP secrets has nothing to check (the fake salts would change, which matters less).
- Copies of the TOTP secret by value inside `totp-rs` (`Builder`, `Totp`) stay a known limitation (security.md, section 14.4).

**Spec impact:** docs/security.md (the key table and section 14.4), docs/architecture.md (environment variables), server/README.md, server/deploy/docker-compose.yml and .env.example, the guide's table of settings (EN and RU).

### 178. One request for a password change with a recovery key reissue (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.2; D173, item 2). Adds docs/sync-protocol.md, section 5.9.

**Context.** D173 asks for the password change, the rotation of the personal vault key and the recovery key reissue in one server request applied in one transaction, with everything SP 5.1 and 5.3 list for them, and leaves the request's design to the run. Run 8's agent C found that "Lost or stolen" rotated without a reissue, so the stolen device's copy of the old recovery key still passed `recover/begin` and could set a new account password (reproduced by `lost_or_stolen_retires_the_old_recovery_key`, `server/tests/e2e_keys.rs`).

**Decision:**

- `POST /v1/account/credentials` (access token; a sensitive operation of SP 4.8) carries the fields of `/v1/account/password` (`current_auth_key`, `kdf`, `kdf_salt`, `auth_key`, `kek_wrap`), the required `rotation` and those of `/v1/account/recovery-key` (`recovery_auth_key`, `record`). The answer is the reissue's `{"tokens", "seq"}`.
- The server checks in the order of SP 5.9 (the proof of password, the KDF bounds and field sizes, the epoch, the record and its `base_revision`) and writes everything in the one transaction; any failure rolls it back, so the old password and recovery key keep working. `current_auth_key` is checked as for the other signed-in requests, before the write transaction (`accounts::check_current_auth_key`, D174); the write takes the user with `CheckedKey::user`, so a password change or reissue committed in between answers `401 invalid_credentials`.
- `/v1/account/password` is sent only with the option off and never with a `rotation` any more (the server still takes one); `/v1/account/recovery-key` is the reissue alone.
- The client prepares the change in memory (`password_change_prepare`, `recovery_key_reissue_prepare`), shows the new key, and sends only after "I saved it" (`key_change_confirm`); "Cancel" (`key_change_cancel`) and a lock drop it. Each prepared change has an id, and confirming, cancelling or copying names it: a second prepare (another window) replaces the first, whose dialog is then refused instead of committing the other key. The one-call `password_change_rotating` and `recovery_key_reissue` stay for callers that show the key themselves (tests).
- The answer's tokens (of this request and of `/v1/account/password`) are stored only while the stored `server_url` is still the origin the request went to; otherwise nothing is stored, so tokens of one server are never sealed and sent to another (D188; found by the cross review).
- "Lost or stolen" revokes the device at once, as before, so the revocation never depends on the rest of the flow (a lock or auto-lock closes the dialog, a request fails); after "I saved it" the `DELETE` is sent once more, right before the change (`key_change_confirm_lost`), because during the minutes the user spends saving the key the thief could sign in again with the old password, which clears a revocation (found by the reviews of this change). A failure of the change itself leaves the device revoked, and the dialog says that the old password and recovery key the device holds still work.

**Spec impact:** docs/sync-protocol.md, sections 3.2, 4.7, 4.8, 5.1, 5.3 and 5.9 (new); docs/security.md, sections 5.9 (key table and key events) and RR6, RR16; docs/ui-behavior-spec.md and docs/ru/ui-behavior-spec.md, sections 12.2 and 13.4.

### 179. "Remember on this device" after a rotation on another device (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.2; D173, item 2; ROADMAP 2.1, run 7 question 3). Extends D102 (rotations made on this device rewrap the `device` row).

**Context.** A device that learnt a new personal epoch at sign-in (a rotation made elsewhere) deleted the old epoch's `device` row and made no new one, so "Remember on this device" went off without a word and its keychain item stayed behind.

**Decision:**

- In the transaction of the rebind that writes the new epochs, the device key read from the OS keychain (before the transaction, under the `remember` lock) wraps the new current key: the new `device` row.
- Without the key (no keychain item, or the keychain cannot be read) the option is off, and a `meta` flag `remember_device_lost` (not secret) makes Settings ▸ Security say why, until the user sets the option again. The same note follows a rotation made on this device without the key.
- A revoked device never makes the wrap: it learns a new epoch only by signing in with the new password, which a lost or stolen device does not have.

**Spec impact:** docs/sync-protocol.md, section 5.1 (effects); docs/security.md, section 5.9 (key events); docs/ui-behavior-spec.md and docs/ru/ui-behavior-spec.md, section 13.4.

### 180. What a failed reissue tells the user (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.2; SP 5.3, step 7). Details of the client's answer to a failed `/v1/account/recovery-key` or `/v1/account/credentials`.

**Context.** SP 5.3, step 7 says the client discards the new key and tells the user it is not valid, and D173 that the user is told the old password and key still work. Both hold only when the server answered: a request whose answer was lost may have committed, and then the key shown is the only valid one; telling the user to discard it could lose the account's recovery (principle 3).

**Decision:**

- An error answer of the server (`4xx`) to the change, or a refusal before anything was sent (the access token is made sure of first): "not accepted". The key is discarded, and the dialog says that nothing changed, that the key shown is not valid, and that after a concurrent change on another device "Show" gives the valid key once this device signs in again.
- A refused token refresh on the way (`401`, the device is signed out): "signed out". A committed change revoked this device's tokens, perhaps this device's own attempt whose answer was lost (a proxy that retried), so the dialog tells the user to keep the key until the next sign-in, after which "Show" gives the valid one.
- No answer (a network error, a timeout, a `5xx`, a malformed answer): "not known". The key is not used again by this device, but the dialog tells the user to keep it (with the old one, for a reissue alone), and both passwords for a password change, until this device signs in again; "Show" then gives the valid key. A later probe cannot prove a rollback: the server goes on with a transaction after the client gave up, so an answer to `GET /v1/account` may come before the commit (found by the review of this change).
- The server accepted the change but this device could not store it (locked meanwhile, a failed write): "in effect". The user keeps the key and signs in again with the new password, as SP 5.1 says for a crash after the commit.

**Spec impact:** docs/sync-protocol.md, sections 4.7 and 5.3 (step 8, new); docs/ui-behavior-spec.md and docs/ru/ui-behavior-spec.md, sections 12.2 and 13.4.

### 181. A forgotten password with "Remember on this device" (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 2.5; run 8, agent C, report section 8). Decided by "the safest default; the user can turn it on explicitly" (D173, item 5).

**Context.** With "Remember on this device" on, the app opens at launch without the password, so a user who forgot it can still work but cannot change it: the change asks for the current password. Agent C asked whether the change should work without it.

**Decision:** no. A change of the master password keeps asking for the current one, also when the vault was opened with the device key: otherwise anyone who starts the app on an unattended computer could replace the password, and with it the lock screen's protection, in seconds; with an account the server would refuse the change anyway, since it needs the proof of the current password (SP 4.8). The ways out stay: with an account, "Restore from sync" with the recovery key (shown at registration and in Settings ▸ Security, SP 5.2); in local mode, Export while the app is open, "Start with a new vault" and Import, as the user guide already says (chapter 1, "If you forgot the master password"). Nothing in the code changes.

**Spec impact:** none (the user guide already describes the ways out).

### 182. Recordings re-encrypted at sign-in keep their key until they are done (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.1; run 8 agent C, finding 5). Clarifies docs/storage-format.md, sections 2.2 and 8.4.

**Context.** The merge at sign-in re-encrypted the recordings after the transaction that deleted the old personal vault and its keys: a recording that failed (a full disk, a file being written) stayed sealed under a key no longer on the device, nothing retried it, and the re-encryption itself rebuilt each file from its parsed events, which dropped the size-limit marker and the missing final chunk of an incomplete file, cut what did not fit the size limit, held the whole file in memory under the core's lock, and lost the output a running session appended after the rename.

**Decision:** Before the merge transaction, every recording sealed under the old vault gets a `<file_id>.<vault_id>.rekey` file with its key wrapped for the new vault (under a key derived from the file's key in that vault, AAD `"shellpp/v1/wrap/recording-rekey"` ‖ the file's head); a failure to write one stops the sign-in before anything changed. After the commit and at every unlock the client re-encrypts each such recording chunk by chunk with the same plaintext and the same chunk boundaries, syncs the new file, renames it over the old one and only then deletes the `.rekey` file; a file a session of this process is still writing is left for a later unlock; until then the recording opens through its `.rekey` file. One `.rekey` file per target vault, so a sign-in that does not commit never replaces the one an earlier merge left.

**Spec impact:** docs/storage-format.md, sections 2.2 and 8.4; docs/security.md, section 5.6 (the wrap blob row).

### 183. A forged key epoch stops sync until the user resets the vault's sync state (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.5; D173, item 1.7; run 7 report, section 10.1, finding 7). Clarifies docs/sync-protocol.md, sections 6.4 (step 2) and 6.9, and docs/ui-behavior-spec.md, section 12.2.

**Context.** A pulled record whose header names a key epoch above the device's stopped the pull with "newer epoch" and left the cursor before it, so every cycle stopped at the same record: a server could stop sync for good with one forged header, and the status asked the user to sign in again, which cannot help.

**Decision:** A header epoch above the device's while the page's `current_epoch` is not above it is a key the server itself does not hold: the cycle stops with `forged_epoch`, again in every cycle, and changes nothing (no key row, nothing after it in the page). Account & Sync shows the row of UI 12.2 with "Reset sync state of this vault…"; after the confirmation the client runs the full resync of section 6.9 from cursor 0 (`Core::sync_reset_vault_state`) in which such a record is refused like a failure of steps 1-3 (not applied, the cursor advances, an integrity warning), until that resync completes, also after an interrupted cycle or a restart (`meta` key `sync_reset_vault`). The epoch is never accepted; local records the snapshot lacks are kept and pushed back as in every full resync, so a refused record's genuine server version comes back through the conflict of its push-back. A page whose `current_epoch` is above the device's is the ordinary newer epoch (sign in again, section 5.1). The same check stops a server restore (section 6.9) and the merge at sign-in with `forged_epoch` (the reset refuses the record in a restore too), and a push conflict whose `current` names an epoch above the device's (the cycle pushes only after a pull that reported none) is refused as damaged and not retried automatically.

**Spec impact:** docs/sync-protocol.md, sections 6.4 and 6.9; docs/ui-behavior-spec.md, section 12.2 (and `docs/ru`).

### 184. The server version a device pushes over is kept as a conflict copy (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.5; D173, item 1.11; run 7 report, section 10.1, finding 11). Amends rule 0 of docs/sync-protocol.md, section 6.7 (D102), and sections 6.4 (step 6) and 6.9.

**Context.** After a server is restored from an older backup, another device may edit a record there at a revision not above the one this device synced. Rule 0 (D102) lets the device's version win over such a "stale" server version, and a pulled version at or below the synced revision was ignored, so the device pushed its version over the other device's edit without a trace (at an equal revision the two even diverged silently).

**Decision:** A server version below the synced revision, or another version (other bytes) at the synced revision, is compared by its stamp `(hlc, device_id)` with the version this device synced (its envelope when synced, its base envelope when pending). When its stamp is greater, or when it is another version at the synced revision (a device never writes two versions at one revision, so such a fork is always another device's write), it was written on the restored server: the device's version still wins (rule 0; at an equal revision it is re-sealed above it and pushed, as rule 0 does), and the server version is kept as a conflict copy, which is pushed and reaches every device like any copy. A stale version below the synced revision whose stamp is not greater (a version this device superseded, or a replay by a hostile server) adds nothing and gets no copy, nor does one equal to the local version, a conflict copy, one under a key epoch below the one this device synced (refused, D93), or one whose copy this device holds already (a repeated resync or restore). A record a restore or resync marked (synced below its base) that meets another version at its base revision is the same fork and is handled the same way (cross review of run 9). In a server restore the device notices (a new `server_generation`, section 6.9), a server version above the revision this device synced replaces its version as usual, and the replaced version, which the restored server may have lost, is kept as a conflict copy too; a device that was only behind gets a copy it did not need, never a loss. Residual, for the owner: (1) an edit made on the restored server by a device whose clock is so far behind that its stamp is below the lost version's is still pushed over without a copy; stamps are the only order the client has; (2) in the full resync of a rebind (the device was signed out during the restore, so no generation tells it a restore happened) a server version above the synced revision replaces the local one without a copy, as in any pull: copying there would copy every record another device changed meanwhile. Run 10: (2) is closed by D197, (1) by D198 wherever the server's `seq` tells the edit apart.

**Spec impact:** docs/sync-protocol.md, sections 6.4 (step 6), 6.7 (rule 0) and 6.9 (server restore, step 3).

### 185. Failed sign-ins: a window per email and network, and a soft limit per email (run 9)

Status: run 9 decision, 2026-10-24 (the cross review of run 9, theme 2, finding 3; D173, item 1.2, which rules full lockouts out for TOTP). Replaces the key of the failed sign-in row of docs/sync-protocol.md, section 10.1 (stage 0 decision: 10 failures per 15 minutes per email). Amends D174.

**Context.** The failure window was keyed by the email alone: ten wrong keys in 15 minutes from one address made `login`, `recover/begin` and every `current_auth_key` check (email change, TOTP, deletion, key epochs, password change, reissue) answer `429` for the owner from any network, for as long as someone who knows only the email keeps going. D175 had removed the same kind of lockout from TOTP.

**Decision:**

- The window counts per email and client network (an IPv4 address, the /64 of an IPv6 one): 10 failures per 15 minutes (`SHELLPP_RATE_LIMIT_FAILED_SIGNINS`), then `429` with `Retry-After` until the window ends, for that network only. An attempt still counts from before its Argon2id check until it passes (D174).
- A soft limit per email over every network, for login and recovery: after 10 failures (the same setting), 2 seconds doubling up to 4 minutes, from the last failure. It does not hold:
  - a network a successful check of the email's key (login, recovery, `current_auth_key`) came from in the last 30 days (8 networks per email; past 100,000 emails the ones seen longest ago go, down to half);
  - a login or recovery with the `device_id` of one of the account's devices that is not revoked (only the account's devices know their ids), so the owner's devices sign in again whatever someone who knows only the email does (review finding);
  - a signed-in request (`current_auth_key`): it is past the sign-in, and its failures count only in its own network's window.
  Failures are forgotten 24 hours after the last one. The answer is the longer of the two waits.
- Known limitations (named for the owner in the report): a new device on a new network can lose its turn to an attacker who tries the moment each wait ends; the soft limit is checked before the Argon2id check and counted after it, so up to the number of Argon2id slots of tries from different networks can share one turn; known networks and counts are in memory, so a restart forgets them; a flood of failures for 50,000 other emails can push an email's soft count out of the map (it needs about 200 Argon2id checks a second).
- Everything is in memory and keyed by the email's SHA-256 (D174). Every check of an auth key outside the transaction now takes the client network, including `/account/password` and `/account/recovery-key`, which moved to `CheckedKey` (the merge of D174, item 4, done).
- What it gives: from one network 40 wrong keys an hour as before (with Argon2id, the 64 MiB client KDF and the email-bound salt behind each); the owner's devices, sessions and usual networks are never held, and a new network waits at most 4 minutes per try. A spread attack from many networks takes about 15 wrong keys an hour once the soft limit is at its cap (more while tries share a turn, see above).
- The account endpoints and key epochs take at most the 1 MiB body of the auth endpoints (section 10.2), so the requests waiting for an Argon2id slot hold at most that much each (review finding).

**Spec impact:** docs/sync-protocol.md, section 10.1 (the failed sign-in row and the paragraph after the table); server/README.md ("Client addresses").

### 187. The certificate dialogs of the sync server (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.3; D173, item 3). Implements the interface of docs/sync-protocol.md, section 2.1, and UI 12.2, 13.2 and 13.5, step 1, over the trust modes of D93.

**Context.** The core had all three trust modes, but the app only pinned: the trust dialog existed only in the connect wizard, which a bound, signed-in device never opens, so a changed pin (every renewal of a pinned certificate) showed "Last error: certificate_changed" and was confirmed only by Sign out → Connect → Trust → Sign in (run 8, agent C, finding 6). "Add a CA" and "Use the system's trust" had no callers. The core itself never needed a new sign-in: the end-to-end test of this run passed on the unchanged core.

**Decision:**

- One certificate panel (`apps/desktop/src/sync_ui/trust.rs`) is the wizard's certificate step and a dialog of its own: the trust choice of an unknown issuer (server, subject, issuer, dates, the reason, the fingerprint as openssl prints it with the command that prints it on the server; "Trust this certificate", "Add a CA certificate…", Cancel), the changed certificate (the pinned and the new fingerprint side by side, whether the system trusts the new one, "Trust the new certificate", "Use the system's trust" only when it does, Cancel) and a refusal (expired, another name, any other TLS failure: the reason, Close).
- "Add a CA certificate…" reads a PEM or DER file of at most 256 KiB and shows the CA's subject and fingerprint for confirmation ("Add this CA", Back).
- Every choice goes through `Core::sync_server_trust_verified`: `GET /v1/server` under the new pin, CA or system trust, and the `server_tls` row is written only after that answer. A CA that did not issue the server's certificate (or one of the same name whose signature does not verify) leaves the choice open with "not issued by this CA"; a certificate for another name or out of its dates under the CA is refused with the reason; a pin of another certificate than the one the server shows is not kept. A kept choice changes only that row: account, tokens and records stay, so sync goes on without a new sign-in.
- Account & Sync: a bound device whose last cycle ended with `certificate_changed` or `certificate_refused` (an added CA no longer validates) shows a warning row with "Review…", which asks the server again and opens the dialog with what it shows now (a problem that went away just syncs). Under the server address, a row shows the current trust ("Trusted by the system", "Pinned certificate" or "Your CA: <the CA's common name>", with the fingerprint); it only shows it (UI 12.2).
- "Restore from sync" of the unlock screen (UI 13.2) opens the same dialog when the recovery stops at the certificate; the recovery runs again with Restore.
- Cancel keeps sync stopped: the automatic cycles keep trying the handshake, which sends no request, so a server that shows the pinned certificate again syncs again by itself. While a choice is being checked, Cancel, Back, Escape and a click outside wait for it, so no choice is kept behind a closed dialog.
- A certificate for another name or out of its dates is refused with that reason before any trust is offered, also when its issuer is unknown (webpki reports the issuer first) and when it replaces a pin: the dialogs never offer a pin the check would refuse. A pin or "the system's trust" the server does not answer under (it changed its certificate after the dialog opened) leaves the dialog on what the server shows now, under the stored trust, with "Nothing was trusted".
- A bound device's review of an unknown issuer (a server trusted by the system or an added CA until now) warns that an unannounced change can mean an attack, as the changed pin does. A review that cannot reach the server says so in a toast. The certificate's subject, issuer and the reason of another failure are shown with control and bidirectional characters made visible (security.md 11.4). Restore from sync stopped at the certificate keeps the local recovery on offer.
- An added CA lives only in the verifier of its origin; nothing is written to the OS store, and SSH host keys never consult it. The system's trust of sync connections is the OS verifier, or a fixed root set in tests (`Core::sync_set_system_roots_for_test`, behind the `test-hooks` feature): no test changes the machine's store, and the tests of the trust choices do not read it (the run 7 tests of the verifier still check one origin through the OS verifier, the path the app takes).

**Spec impact:** docs/sync-protocol.md, section 2.1 (a choice is kept only after the server answers; a kept choice keeps the account signed in); docs/ui-behavior-spec.md, section 12.2 (the trust row only shows the trust; the refused certificate of an added CA; confirming goes on without signing in), and `docs/ru`; the user guide, chapter 6 (en, ru); `server/README.md`, access 3.

### 188. A hostile server: answer sizes, the refresh token's origin, the TOTP URI (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.6; run 8, agent C, findings S3, S4 and S5). Adds to docs/sync-protocol.md, sections 5.5 and 10.2, and docs/storage-format.md, section 6.4.

**Context.** Three ways a server, or someone who can change the local database, could reach further than the protocol allows: the client read any answer whole into memory (a server could send gigabytes); the wrapped refresh token was bound only to the device id, so a `server_account.server_url` changed in the database (with a pin or a CA for the new address) sent the token to another server on the next refresh; the QR code of the TOTP setup encoded whatever URI the server returned, a web link included, for the phone that scans it to open.

**Decision:**

- **Answer size.** Every answer is read through a limit: 16 MiB (`http::MAX_ANSWER_BYTES`), chosen from section 10.2: the largest legitimate answer is a pull page, 8 MiB of envelopes plus the one that crossed it (262 232 bytes), which is under 12 MiB as base64url in JSON with the records' other fields; account bundles, device and vault lists and key epochs are far smaller. A push answer may carry the server's envelope for every item (a conflict's `current`), so its limit is 16 MiB plus 350 155 bytes per item (349 643 base64url characters and its id, seq and status). An announced `Content-Length` above the limit is refused before reading; a body read past it is refused as soon as it crosses it (`answer_too_large`; the automatic cycle tries again a minute later, with no backoff, as for other refusals).
- **Refresh token origin.** The AAD of the refresh token's wrap is `"shellpp/v1/wrap/refresh-token"` ‖ `device_id` ‖ `be16(len(o))` ‖ `o`, with `o` the normalized origin that issued it (the encoding of the auth keys). A token that does not open for the stored `server_url` is never sent: the device is signed out locally (the refresh token column cleared) and signs in again with the master password. The access token kept in memory is used only with a client of the origin that issued it, and a refresh whose client was made for another address than the stored one sends nothing (review of this entry). The test vectors (`crates/core/tests/vectors`) are regenerated by their independent generator. Behaviour change: a device bound before this change cannot open its stored token and signs in once more after the update; no data is lost.
- **TOTP URI.** The client never shows or encodes the server's `otpauth_uri`: it checks the secret (base32 of 16 to 128 characters, padding aside; otherwise the setup is refused with `totp_secret`) and makes the URI itself, `otpauth://totp/Shell%2B%2B:<the account email, percent-encoded>?secret=<secret>&issuer=Shell%2B%2B`. A first version checked the server's URI instead; its review found parameters and fragments that authenticator apps read differently, so nothing of the server's text reaches the code.

**Spec impact:** docs/sync-protocol.md, sections 5.5 and 10.2; docs/storage-format.md, section 6.4; docs/ui-behavior-spec.md, section 13.5, step 3, and `docs/ru`.

### 189. Weaker KDF parameters at pre-login are a possible downgrade (run 9)

Status: run 9 decision, 2026-10-24 (D173, item 1.5; run 7 report, section 10.1, finding 5). Adds to docs/sync-protocol.md, section 4.2.

**Context.** On a bound, signed-out device, a server could answer pre-login with weaker KDF parameters than the ones the device holds for the account, still within the floor; the client derived the auth key under them, sent it, and the rebind stored them. A hostile server, or one that took over the address, then gets an auth key that is cheaper to attack offline than the device's own. A weaker answer is also what an honest server sends after the password was changed on a phone (the mobile preset, 32 MiB × 4 passes, against the desktop's 64 MiB × 3).

**Decision:**

- "Weaker" is less memory, or less memory × passes, than `local_auth.kdf` (lanes change neither); the check runs for every device that is bound to an account or was bound before (a `server_account` row, or a personal vault that keeps its binding in `vaults.account_user_id`, also after Log out or `account_deleted`), whatever address is typed, after the floor check and before anything is derived or sent. It refuses with `weaker_kdf` and keeps both parameters for the dialog (`Core::sync_weaker_kdf`).
- The wizard opens "Weaker key derivation", a dialog of its own: this device's and the server's settings side by side (memory, passes, lanes), why this is expected after a password change on a phone and what else it can mean. Cancel keeps the device signed out; "Sign in with weaker settings", a danger button, signs in through `Core::sync_sign_in_accepting_kdf`, which goes on only when the pre-login answers exactly the parameters the user confirmed.
- Not covered: the first sign-in of a device that was never bound to an account (nothing to compare with).

**Spec impact:** docs/sync-protocol.md, section 4.2; docs/ui-behavior-spec.md, section 13.5, step 2, and `docs/ru`.

### 190. Log out with "delete local copy" sets the copy aside (run 9)

Status: run 9 decision, 2026-10-24 (plan, section 3.6; run 8, agent C, finding 3). Amends docs/storage-format.md, "Backup and restore", and docs/security.md, which had the whole data directory deleted.

**Context.** Sign out with "Also delete the local copy on this device" failed: the device was signed out, then `start_new_vault` refused with `WrongState` because it needs the Locked state, and the dialog said "Something went wrong" (reproduced by `server/tests/sign_out_local.rs` before the fix). The code meant to set the database aside as "Start with a new vault" does, while the specifications said that everything in the data directory, `backups/` and `recordings/` included, is deleted. A deletion would lose what is not on the server: edits not pushed yet, the types that do not sync (command history and the connection log unless turned on, session recordings never), Recently deleted and kept conflict copies.

**Decision:**

- The variant that loses no data and keeps the protection: after the account is unbound, the device key leaves the OS keychain together with its `device` row (so nothing opens the old file without the master password), the vault locks, and the database is set aside exactly as "Start with a new vault" sets it aside (`shellpp.db.<stamp>.old` next to the new empty store, never deleted); the app starts at first run, where "Restore from sync" brings the account back. The data on the server is untouched. `backups/` and `recordings/` stay.
- Order (after the review): the device key and its `device` row go first, before the revoke and the unbinding; when the OS keychain refuses, the sign-out stops with nothing changed. Setting the store aside is one step under the mutex from the unlocked state (no lock or unlock in between); the session is wiped only once the new store holds the directory, so a failed rename keeps the vault open and the error in the dialog (the device is signed out, the copy in place).
- The dialog says so under the option when it is ticked, and how many changes are not on the server yet and how many conflicts are kept only here ("Sync first to keep them in the account"); the user guide says that wiping a device means deleting the data folder by hand.
- For the owner: whether "delete" should really erase the files (the old text) is open; the code now follows this entry.

**Spec impact:** docs/storage-format.md, "Backup and restore"; docs/security.md (the log out row of the key lifecycle stays, the paragraph on deletion changes); docs/data-format.md, section 1, and `docs/ru/data-format.md`; docs/ui-behavior-spec.md, section 12.2 (Log out), and `docs/ru`; the user guide, chapter 6 (en, ru).

### 191. A second russh patch: empty channel data is not the end of a stream (run 9)

Status: run 9 decision, 2026-10-24 (plan, 3.6; D173, item 1.10). Extends D100 (item 3: russh is built from `third_party/russh` with one patch) to two patches.

**Context.** The run 7 review (its report, section 10.1, finding 10) found that russh 0.64.1's `ChannelRx`, the reader behind `Channel::into_stream`, hands an empty `CHANNEL_DATA` to the reader as a read of 0 bytes, which every `AsyncRead` user takes for the end of the stream. RFC 4254 allows empty data, so a server may send it; an SFTP session then closed as if the server had ended it. The app reads two more channels through the same type: the `direct-tcpip` channel that carries a ProxyJump connection (an empty packet from the jump host ended the whole inner connection) and a forwarded agent channel; port forwards and the shell read messages and were not affected. Reproduced before the fix: all three tests below failed on 0.64.1.

**Decision:** a second patch in `third_party/russh`, the smallest that fixes it: in `ChannelRx::poll_read` an empty `Data` (or `ExtendedData` of the stream being read) takes the arm that already skips messages the reader does not want, so the next poll reads the next message (two `!data.is_empty()` guards, marked "Shell++ patch"). `PATCHES.md` lists both patches and how to remove each one alone when a russh release fixes it (CLAUDE.md says the same); the text for the upstream issue and pull request is `docs/upstream/russh-empty-data.md`. Tests (`crates/ssh/tests/empty_channel_data.rs`): a russh server behind a proxy that adds an empty data packet for the same channel (both ends negotiate the `none` cipher and MAC, test only, since russh never sends empty data itself): an SFTP session answers requests after the empty packet, an exec channel's stream reads all of its output, a stderr reader all of its stream.

**Spec impact:** none (a library fix); `third_party/russh/PATCHES.md`, `THIRD_PARTY_NOTICES.md`, CLAUDE.md ("Подводные камни").

### 192. The values of the sync API are wiped on drop (run 9)

Status: run 9 decision, 2026-10-24 (plan, 3.6; D173, item 1.4). `crates/sync-api` gets one dependency, `zeroize`, which only wipes memory: the crate stays without cryptography and I/O (docs/architecture.md, section 2.3).

**Context.** The run 7 reviews (its report, section 10.1, finding 4) found that `B64` and `Secret`, through which `auth_key`, `recovery_auth_key`, tokens, tickets, TOTP codes and the TOTP setup answer travel on both sides, kept their bytes in freed memory, and that the client copied the access token into plain `String`s on the way.

**Decision:**

- `B64` and `Secret` derive `Zeroize` and `ZeroizeOnDrop` (`zeroize`, already in both lock files, MIT OR Apache-2.0); the base64 text a `B64` is decoded from or encoded to is held in `Zeroizing`, and it is decoded into a value that wipes itself, also when the text turns out malformed. Their fields stay public, so building and reading a value does not change; moving the contents out, which a `Drop` type forbids, goes through the new `into_inner()`, which moves the buffer without a copy (the client puts tickets and the TOTP setup answer straight into `Zeroizing` and `SecretString`; the server hands salts and wraps to the database).
- The client: the access token is borrowed or held in `Zeroizing` on the sign-in paths (it was copied into plain `String`s for the first requests of a sign-in and of the merge, and the merge's copy was left unwiped when a step failed); the decrypted refresh token is turned into text without an intermediate copy (a defensive change: only an unreachable error path left one); the TOTP setup dialog keeps the secret and its `otpauth://` text in `Zeroizing` instead of plain `String`s, and its `Debug` shows neither.
- Copies outside these values stay a known limitation (security.md, section 9.5): the JSON text that serde_json writes and reads, and what reqwest, hyper and, on the server, axum keep of the bodies and of the `Authorization` header. Wiping them needs an allocator that wipes freed memory, which D83 left out.
- Tests: both types are `ZeroizeOnDrop` (the compile-time check used for the other wiping types), `zeroize()` empties them, `Debug` of the types, of bodies made of them and of the TOTP setup step shows neither the value nor its base64, the wire form stays a plain string, and `into_inner()` hands the contents over.

**Spec impact:** docs/security.md, section 9.5 (the wiped values; the limitation) and RR11.

### 193. `ForwardAgent` only from the user's own `~/.ssh/config` (run 9)

Status: run 9 decision, 2026-10-24 (plan, 2.5 and 3.6; D173, item 5: "the safest default; the user can turn it on explicitly"; run 8 agent C's report, section 8). Narrows D60 and the last sentence of D172 for the OpenSSH config import. Changes behaviour users are used to: a config imported from another path used to turn Agent Forwarding on.

**Context.** The import maps `ForwardAgent yes` to Agent Forwarding with "Also offer the system agent's keys" (D60), because OpenSSH forwards the user's agent. D172 kept that for `~/.ssh/config` as the user's own file, but the import reads any path the user picks: a config from a colleague, a download or a repository is a file from elsewhere (security.md, A10), and its hosts could get the user's keys while a session is open (RR19).

**Decision:**

- The core's `SshConfigSource` says whether the file is the user's own (`own`, false when left out); only then does `ForwardAgent yes` turn Agent Forwarding on. The app decides it by comparing the chosen file with its suggested `~/.ssh/config` (the home folder's file, or the stand-in that tests set; the demo has none, so no file is its own there) once both are resolved by the file system; any other file, a missing one included, is not the user's own.
- From another file, `ForwardAgent yes` imports the host with Agent Forwarding off, the preview and the host's notes say "ForwardAgent is imported turned off: the file is not your ~/.ssh/config", and the summary counts such hosts ("Hosts imported with Agent Forwarding turned off (see their notes)", `hosts_agent_forwarding_off`, the name of the Import Data count of D172). The user turns forwarding on per host. `ForwardAgent no` needs no note. The import key stays the alias, so importing the user's own config later finds such a host "already exists" and does not change it. Hosts imported earlier from other files keep their setting: nothing is changed in place.
- Files included by the user's own config (`Include`) are part of it. PuTTY sessions (D140) come from the user's own registry and keep mapping agent forwarding.
- Tests: the core imports `ForwardAgent yes` from another file turned off with the note and the count, and from the user's own file turned on (D60, unchanged); the dialog tells the two apart and shows the summary row only for the other file.

**Spec impact:** docs/ui-behavior-spec.md, section 2.5 (and `docs/ru`); docs/security.md, section 13.5; the user guide, chapter 2 (and `docs/guide/ru`).

### 194. While the app is locked, forwarded agents offer no keys (run 9)

Status: run 9 decision, 2026-10-24 (plan, 2.5 and 3.6; D173, item 5; run 8 agent C's report, section 8). Replaces the stage 0 decision of security.md, section 8 ("Proxying of the system agent for open sessions: continues; the system agent applies its own policy"). Changes behaviour users are used to: with "Also offer the system agent's keys" on, a host could ask the system agent for signatures while Shell++ was locked.

**Context.** A lock wipes the Keychain keys of the built-in agent, so a forwarded agent stops offering them (D60), but requests for the system agent's keys were still passed to the system agent. Locking says "I am away"; a host whose root is hostile could then sign in elsewhere with the user's system agent keys for as long as the session stays open (RR19), without the user at the screen to notice a confirmation or to close the session.

**Decision:**

- While the app is locked, a forwarded agent channel lists none of the system agent's keys and refuses a signature with one of them before anything reaches the system agent, as it already did for the Keychain's keys. In `shellpp-ssh` the built-in agent's key source answers `system_agent_allowed()` at every request (true by default); the desktop's answers "unlocked, or the user chose so".
- The explicit way to keep the old behaviour: Settings → Security → "Forward the system agent while locked" (`security.system_agent_while_locked`, a per-device setting, off by default). On, a host that also offers the system agent's keys can still ask the system agent for signatures while the app is locked, and the system agent applies its own policy (a confirming agent still asks). The sessions read it at each lock, before they learn of the lock, so no request sees the lock with an older choice. It belongs to the device, like the serial port (D95): an export leaves it out and no import takes it, a full restore neither.
- Sessions, port forwards and transfers keep running while locked as before (D9); only signatures through forwarding stop, and they come back at unlock without reconnecting.
- Tests: a russh server written for the test asks the forwarded agent for identities and for a signature by a key it learned earlier: while the app refuses, nothing is listed or signed and the system agent receives no request; allowed, it lists and signs; refused again, it stops at once (the test fails when the check is removed). A lock that lands after the request was checked (during a confirmation) keeps the system agent from signing, and one that lands while the system agent signs keeps the signature from the host. The desktop reads the setting at each lock; the Security switch is off by default and stores the choice; an export leaves the setting out and no import, a full restore included, takes it.

**Spec impact:** docs/security.md, sections 8 and 11.2; docs/ui-behavior-spec.md, sections 3.1, 13.3 and 13.4 (and `docs/ru`); docs/data-model.md, section 16.1; the user guide, chapter 3 (and `docs/guide/ru`).

### 195. Key changes and a vault that keeps its account (run 9)

Status: run 9 decision, 2026-10-24 (the cross review of run 9, theme 1). Amends D173 (item 2, local mode) and D180; carries out SP 5.1, re-post step 4.

**Context.** After Log out with the local copy kept, the server account row is gone but the personal vault keeps its account binding, so the next sign-in is a rebind. A change with "My old password may be compromised" took the local branch there: a local rotation and a new recovery key. The rebind then ended in "server state rolled back", and the re-post gave the server the new recovery wrap and record while its recovery verifier stayed the old key's: neither key recovered, without a warning, and Show displayed the new one. D180 also treated every `4xx` as a rollback, a proxy's `408` without an error body included, and a failed second revocation of "Lost or stolen" lost its "signed out" meaning.

**Decision:**

- While the personal vault keeps an account binding and the device is not bound, the option and a recovery key reissue are refused ("sign in to the account and change it there"); a change without the option stays local, as SP 5.1 has it. The safest choice: nothing local runs ahead of the account.
- The re-post checks, after its restore step, whether the local `recovery_key` record is newer than the server's (pending: its synced revision below its revision); then it keeps a `meta` flag that Account & Sync shows with "Reissue…" (shown first, with "I saved it") until a reissue or a change of SP 5.9 passes.
- Only a `4xx` with an error body of the server is a rollback; one without (`http_4xx`) counts as no answer. `409 epoch_conflict` with a `current_epoch` below this device's is "server state rolled back". A refused refresh before the second revocation of "Lost or stolen" stays "signed out".
- The password change dialog compares the new password and its repetition in wiping buffers and empties its fields once a step passed.

**Spec impact:** docs/sync-protocol.md, sections 5.1 (local mode, re-post step 4) and 5.3 (step 8); docs/ui-behavior-spec.md, sections 12.2 and 13.4 (and `docs/ru`).

### 196. "Remember on this device" at a merge, and the recovery row of a rebind (run 9)

Status: run 9 decision, 2026-10-24 (the cross review of run 9, theme 1). Extends D179.

**Context.** The merge at sign-in removed the old vault with its `device` row, so "Remember on this device" went off without a note, and its keychain item stayed: it still opened the pre-merge backup in `backups/` without the password. The bound key change that found the device key missing left the item too, unlike the local one. A rebind whose first pull failed built the local `recovery` row of the new epoch from the record held before, which after a reissue elsewhere is the old key: its holder could open this copy offline, and the server check reported a mismatch that was not there.

**Decision:**

- After a merge with the option on, a new device key replaces the keychain item and wraps the account vault's key; when that fails, the item is deleted and the note of D179 is set.
- A bound key change that finds the device key missing deletes the keychain item, as the local one does.
- A rebind builds the `recovery` row and checks the server's recovery wrap only after its first cycle went through; otherwise a later cycle builds it from the pulled record (`ensure_recovery_row`).

**Spec impact:** none beyond D179 (docs/sync-protocol.md, section 5.1, describes the rewrap).

### 197. The owner's answers to run 9 (run 10)

Status: the owner's decisions, 2026-10-24 (plan of run 10, section 2, and the owner's answer before the run). Revokes D190; settles the questions of ROADMAP 4.1 and 4.2 after run 9.

**Decision:**

- **Release.** Release 1.0 ships without team vaults; stage 8 comes after it. ROADMAP has the path to the release.
- **The previous account's archive** is finished after release 1.0; its keys will then be wrapped under the personal vault key (storage-format 6.2 changes with that work). Until then, signing in or registering on a device whose vault is bound to another account is refused with a message that names the safe path.
- **D188** stays: every bound device signs in once after the update; no migration.
- **D190 is revoked.** "Also delete the local copy" at log out deletes the local copy for real, as the text before D190 had it: the whole data directory as far as the app wrote it (the database, set-aside copies `shellpp.db.*.old`, the backups, the recordings, the temporary directory; a file the user put in the data directory stays) and the device keys in the OS keychain. It needs a confirmation of its own, which lists the set-aside copies and backups, because the server has none of them, and counts what only this device holds (the records sync leaves out, the recordings; cross review of run 10); the warning about unsent changes stays. "Start with a new vault" still sets the copy aside.
- **D194** stays: the setting belongs to the device.
- **D175, D185**: the numbers of the sign-in and TOTP limits are accepted.
- **CI** gets a step with the deployment tests.
- **D184, residual 2**: sign-out keeps the server generation this device synced with; the rebind compares it with the server's and runs the restore procedure of sync-protocol 6.9 when they differ (the server already keeps `server_generation`, so no protocol field). When the rebind does not know the generation of before, every version a newer server version replaces is kept as a conflict copy: not sure, keep a copy.
- **D184, residual 1**: closed if the same rule closes it, otherwise the reason is recorded (D198).
- **D195** is accepted.
- **The `wip/salvage-*` branches** are deleted (what they held is in main as `records::deletable`).
- **Backups**: the rotation never deletes old backups while the new ones are empty or much smaller than before, and keeps at least one weekly and one monthly backup besides the daily ones.

**Spec impact:** docs/sync-protocol.md, section 6.9 (the rebind); the rest with the work of run 10 (storage-format "Backup and restore", security.md, UI 12.2, server/README.md).

### 198. A version written after a restore is told apart by its server sequence (run 10)

Status: run 10 decision, 2026-10-24 (plan of run 10, section 3.2; D197). Amends D184 and rule 0 of docs/sync-protocol.md, section 6.7.

**Context.** D184 keeps as a conflict copy a server version below the synced revision whose stamp is greater than that of the version this device synced: such a version was written on a server restored from an older backup. Residual 1: an edit made there by a device whose clock is behind (by more than the time since the lost version) has a lower stamp and was pushed over without a copy. The rule "not sure, keep a copy" (D197) would copy every older version a restored backup holds: the device cannot tell such a version from a late edit by stamps, so every record the restore rolled back would get a copy of its superseded version on every device.

**Decision:** The server's `seq` tells them apart where it can. A server gives a `seq` only to a later write, so in a restore a version below the synced revision whose `seq` is above the `seq` of the version this device synced was written after the restore, whatever the writer's clock, and is kept as a conflict copy like one with a greater stamp. The pull applies this only inside the restore procedure (sync-protocol 6.9), whose snapshot is where such versions arrive: outside it, a record pushed back after its tombstone was purged also has a lower revision at a higher `seq` (review of run 10), and there the rule would put another device's tombstone over it. Rule 0 at a push conflict applies it always: the local version wins there anyway, so the rule only adds a copy (it covers a late edit pushed between this device's restore procedure and its push).

What stays open: a late edit with a lower stamp that the restored server gave a `seq` still below the synced version's (the restored server has handed out fewer sequences since the backup than it lost). That needs a clock behind by more than the time between the two edits and fewer writes after the restore than were lost, on a record that lost two revisions or more. A hostile server can give an old authenticated version a high `seq` (with a new `server_generation`, so the device runs the restore procedure) and so cause a copy of it, per record and again each time the user deletes the copy: superfluous copies, never a loss.

**Spec impact:** docs/sync-protocol.md, sections 6.4 (step 6), 6.7 (rule 0) and 6.9 (step 3).

### 199. The backup rotation keeps good backups (run 10)

Status: run 10 decision, 2026-10-24 (plan of run 10, section 2.2; D197). Amends D176 and the backup bullets of docs/security.md, section 14.4.

**Context.** The backup service deleted every file older than `BACKUP_RETENTION_DAYS`. After a database was emptied or cut down by mistake, it kept making backups of the empty database, and after 30 nights only those were left.

**Decision:**

- **Size guard.** The measure is the size of the largest backup since the last accepted shrink (all of them when none was accepted), the newest one left out; while the newest backup is smaller than `BACKUP_MIN_PERCENT` percent of it (default 50), nothing is deleted and the log says "pruning paused". The largest, not the last good one, so a database cut down in steps, each half of the one before, is caught as well (review of run 10). The size is that of the encrypted file (age adds a near-constant overhead). Half is the threshold: a real database shrinks by half only after a mass deletion, and a pause costs one small file a day. On a very small database the schema's fixed part of the dump can keep an emptied database above half; the README says to set the percentage higher there.
- **A deliberate shrink** is accepted by the operator: `BACKUP_ACCEPT_SMALLER=1` (with `BACKUP_PRUNE_ONLY=1`) writes the newest backup's time and size to `.accepted-size`; from then on that size and the backups after it are the measure, and older, larger ones no longer count, also after the accepted backup itself is pruned (review of run 10). There is no automatic acceptance: a database emptied by mistake looks the same to the script, however long it stays empty.
- **Weekly and monthly backups.** Besides the backups of the retention, the first backup of each of the `BACKUP_KEEP_WEEKS` calendar weeks (ISO, UTC; default 4) and of each of the `BACKUP_KEEP_MONTHS` calendar months (default 3) before the end of the retention stays, and so does the newest backup. Counted from the end of the retention, so they add to the dailies: with the defaults, weekly backups up to about two months back and monthly ones up to four. The first backup of a period, not the newest, so a backup once kept stays kept.
- A backup's time is the UTC stamp in its name, else its modification time, so a backup copied back by hand keeps its place. The settings are checked before anything else: a bad value stops the service before it dumps, instead of a restart loop that dumps every five minutes (review of run 10).

**Spec impact:** docs/security.md, section 14.4; server/README.md, "Backups"; docs/guide/06-sync.md (and `docs/guide/ru`).

### 200. Changed key derivation settings at sign-in ask for the current password first (run 10)

Status: run 10 decision, 2026-10-24 (plan of run 10, sections 2.3 and 3.4). Carries out the stage 0 decision of docs/sync-protocol.md, section 4.2, which was not built.

**Context.** On a device bound now or before, a pre-login answer whose `kdf` or `kdf_salt` differs from `local_auth` means the password or the email changed on another device. The user then most likely types the old password, which the client derived and sent under the new settings: a failed sign-in that counts against the limits, and an auth key of the old password for the server.

**Decision:** The sign-in stops with `kdf_changed` before anything is derived or sent and keeps the settings and salt it saw. The wizard says "The password or email of this account was changed on another device. Enter the account's current password and sign in again." and empties the password field. The next sign-in (`sync_sign_in_after_kdf_change`) goes on only when the server answers the very settings and salt seen; otherwise it stops again. The settings seen stay until a sign-in with them goes through, so a mistyped current password does not ask again; Back or another address in the wizard forgets them, and the weaker settings confirmed with them (review of run 10). It applies to every device bound before, at any address, as the weaker-settings check of D189 does, and after it: weaker settings get their dialog first, and the confirmed ones are carried to the next sign-in. Another account's email also differs in salt and gets the same message; the binding check then refuses it as before (D197). A local password change or an offline recovery of a vault bound before also writes a new salt, so the message names that case too (cross review of run 10).

**Spec impact:** docs/sync-protocol.md, section 4.2; docs/ui-behavior-spec.md, section 13.5 (and `docs/ru`).

### 201. A FIDO2 key is linked to a host as a key (run 10)

Status: run 10 decision, 2026-10-24 (plan of run 10, section 3.5; ROADMAP 5, "FIDO2 in the host panel"). Amends UI 3.1 ("a FIDO2 key joins the list with FIDO2 support").

**Context.** Since D143 FIDO2 security keys are Keychain keys that sign. The host panel's "+" never offered FIDO2 (it waited for D5), and a FIDO2 entry would fill `ssh.fido2_key_id`, which no connection reads: a connection authenticates with `ssh.key_id` (data-model 8.3), where a FIDO2 key already works. The only "In development" left was a branch no menu reached.

**Decision:** A FIDO2 key is linked through "Key"; the "+" has no FIDO2 entry, and the FIDO2 form opens the key form. A `fido2_key_id` written by an older version still shows as its own row and can be unlinked. The field stays in the data model for compatibility.

**Spec impact:** docs/ui-behavior-spec.md, section 3.1 (and `docs/ru`).

### 202. The server's tests run in CI; superfluous copies after a sign-out before run 10 are accepted (run 11)

Status: owner decision, 2026-10-24 (plan of run 11, section 2; ROADMAP 5, the two questions after run 10). One decision for both answers, as the plan asks.

**Context.** (1) Section 9.2 placed "sync end-to-end with PostgreSQL" in the Linux job, but no workflow ran the server workspace's tests: CI ran only the deployment tests (`server-deploy.yml`, run 10), and the end-to-end, hostile server, property, fuzzing and fault tests ran only in the local full criterion (the load test runs only with `SHELLPP_SYNC_LOAD=1`, in neither). (2) A device that signed out before run 10 keeps no server generation (D197), so its next rebind keeps a conflict copy of every version a newer server version replaces, once: copies it may not need.

**Decision:**

- A workflow of its own, `server.yml`, runs the server workspace's tests on SQLite and then on PostgreSQL in Docker (`SHELLPP_TEST_POSTGRES=docker`), with `--locked` and `--no-fail-fast` as in the full criterion. It runs on the changes `server-deploy.yml` runs on: `server/`, `crates/sync-api`, `crates/core`, `crates/private-file`, the root `Cargo.toml`, `rust-toolchain.toml` and the workflow itself. The plan named `server/` and `crates/sync-api` only; `crates/core` is in because the end-to-end tests drive the client's sync engine (a dev-dependency of the server), so a change to it must run them (owner's answer before run 11). For the same reason `crates/private-file`, which the core uses, joins both workflows (review of run 11). The deployment tests stay in `server-deploy.yml`.
- The superfluous copies of (2) are accepted: the owner has no bound devices yet, so no device signed out before this version.

**Spec impact:** docs/architecture.md, sections 9.1 and 9.2; docs/security.md, section 14.4; ROADMAP 5.

### 203. False positives of the secret scan in pushed commits are ignored by fingerprint

Status: owner request, 2026-10-24 (the Secret scan workflow red on GitHub since the push of f92b48e).

**Context.** The Secret scan workflow (gitleaks 8.30.1, `.gitleaks.toml`, the full history) passed last on 5425f9c and failed on every push since f92b48e. gitleaks 8.30.1 in Docker with the same configuration finds the same 9 findings (8 fingerprints) in commits after 5425f9c, over 5425f9c..main and over the whole history alike, and nothing before. None is a secret:

| File | Commit | Rule | What it is |
|---|---|---|---|
| `apps/desktop/src/sync_ui/key_tests.rs` | 53ac243 | generic-api-key | the line-break test of the recovery key dialog: a random key in the format of a real one, made like the screenshots' keys (run 9), bound to no account |
| `apps/desktop/src/terminal/autocomplete.rs`, `apps/desktop/src/terminal/tests.rs` | 0aafa82 | github-pat | the tests that a token is never suggested: `ghp_` and a run of the alphabet, whose GitHub checksum does not hold |
| `server/src/config.rs` | 73870cf | generic-api-key (twice, one line) | `SHELLPP_SERVER_SECRET` in the configuration unit test: the base64 of a run of ASCII digits and letters |
| `crates/core/tests/fuzz_parsers.rs` | 0a35571 | private-key (2) | parser fuzzing seeds: the dictionary of PEM armor lines, and a PEM block of 48 bytes cut from the start of an RSA key (the version and part of the public modulus) |
| `crates/core/src/keys/convert.rs` | 3d4df05 | private-key (2) | doc comments naming the PEM armor lines of the key formats |

The commits are in main, which moves fast-forward only, so they stay in the history the release scans. An allowlist entry in `.gitleaks.toml` would need either the values themselves in its regexes or a path alone, which allows every future finding in the file.

**Decision:**

- The 8 fingerprints (`<commit>:<file>:<rule>:<line>`) go to `.gitleaksignore` at the root, which gitleaks reads by itself, each group under a comment saying what the value is; the values are written nowhere. A fingerprint covers that commit's line only: the same text added again in a later commit is a new finding and is reviewed again.
- `.gitleaksignore` is for false positives in commits already pushed. A finding in a change not yet pushed is fixed in the change (a test secret made at run time, ROADMAP 7.11) or gets a narrow allowlist entry in `.gitleaks.toml`, as before. A real secret is never ignored: it is removed and rotated.

**Spec impact:** CONTRIBUTING.md, "If the hook flags something"; docs/security.md, section 16.

### 204. Owner decisions for run 12: Mosh, Android, the 1.0 platforms, local checks, the sync server's domain

Status: owner decision, 2026-10-24 (plan of run 12, section 2). One decision for all of them, as the plan asks.

**Decision:**

- **Mosh.** For release 1.0 the desktop hides the Mosh toggle: today it only saves its value (D29). The field stays in the data model. Mosh is built together with the mobile client.
- **The mobile client is Android** (the owner's phone is a Samsung S25). The case: now and then, away from home, open SSH to one's own servers and do what is needed; full sync is wanted from day one. No iPhone app. Run 12 only studies what the Rust core needs to build for Android (`docs/android-feasibility.md`); no app code.
- **The platforms of release 1.0** are Windows and Linux. macOS cannot be checked without a Mac; it is decided in the release preparation run.
- **GitHub Actions** are turned off by the owner. Checks are local only (the full criterion of `CLAUDE.md`, `scripts\verify.ps1`); the workflows stay in the repository.
- **The findings of the verification run** (ROADMAP 2.4) are all fixed in run 12.
- **The sync server** goes on the owner's public domain: Let's Encrypt through Caddy, and the client trusts it through the system's roots.

**Spec impact:** docs/ui-behavior-spec.md (the Mosh toggle); the user guide; ROADMAP 1, 3 and a line for the mobile client.

### 205. The full criterion scans for secrets and checks the libraries and binaries alone (run 12)

Status: run 12 (plan, section 3.0).

**Context.** With GitHub Actions off (D204) the Secret scan workflow no longer runs, and the full criterion did not scan. A release build of the desktop warned about an unused import in `crates/core/src/service/sync_trust.rs` that the criterion's clippy missed: `--all-targets` builds the tests, whose dev-dependencies turn on `shellpp-core`'s `test-hooks` for the whole workspace (feature unification), so code only the hooks use looks used.

**Decision:**

- `scripts\verify.ps1` runs gitleaks with the workflow's version (the Docker image `ghcr.io/gitleaks/gitleaks:v8.30.1`, pinned by digest, with no network), `.gitleaks.toml` and `.gitleaksignore`, `--redact`, over the whole committed history of every ref of the repository; a finding fails the step. The git directory is mounted on its own, because in a worktree `.git` is a file with a Windows path the container cannot follow; the script finds it from the files git writes, not from git's output, which Windows PowerShell decodes with the console's code page. A step that cannot run (no git directory) fails, and a script that ran no step is never green (review of run 12). The step needs Docker, as the PostgreSQL tests already do.
- Both workspaces also run `cargo clippy --locked --workspace -- -D warnings`, without `--all-targets`: the libraries and binaries with the features a release build has. On a warm build it takes seconds.

**Spec impact:** CLAUDE.md ("Тесты"); CONTRIBUTING.md, "Secret scanning"; docs/security.md, section 16.

### 206. VS16 emoji take two cells; emoji sequences keep their font's placement; flags on Windows wait for a font (run 12)

Status: run 12 (plan, section 3.1). Amends D90 for VS16; D92 (no mode 2027, blank cells after a sequence) stays.

**Context.** The plan asks for emoji with VS16 two cells wide, ZWJ sequences as one group, flags as flags and vttest at 80/132 columns. Before run 12 an emoji with VS16 (`❤️`) kept the one cell `wcwidth` gives its base and was drawn two cells wide over the next cell (D90 left it). A ZWJ family on Windows 10 fell apart into two pictures: Segoe UI Emoji draws it from several glyphs it places together, and the renderer forced one glyph per two cells, as for a wide character. Flags are drawn as two letters: Segoe UI Emoji has no flag glyphs. DECCOLM has worked since run 5 (D90); only the 132-column screen had no screenshot.

**Decision:**

- **VS16.** When VS16 follows a narrow character that has an emoji presentation (the base and VS16 are two columns wide by `unicode-width`'s string rules), with nothing joined to it yet and room for a second cell in the row, the cell becomes a wide character: the selector joins it and a spacer is written after it the way the emulator writes a wide character's (insert mode, attributes and the wrap at the row's end as for any character). At the last column the emoji stays narrow, and so it does in insert mode (the spacer would push the row's last cell, which can be another wide character's half) and inside a ZWJ sequence: VS16 on a part after a joiner does not widen it, and a joiner right after a widened emoji makes it narrow again, so a sequence keeps the columns `wcwidth` counts (D92). Programs that count such an emoji as one column (bash's readline with glibc's `wcwidth`, a pager filling the line) can put the cursor one column off; the plan asks for two cells, so that is accepted and recorded in `docs/compatibility.md`. When such a program writes into the emoji's second cell, the emoji goes back to one cell instead of being erased with its broken half (review of run 12).
- **Sequences.** A ZWJ sequence, a skin tone modifier and a flag stay one picture at their first cell (D90), and the grid keeps the columns `wcwidth` counts (D92). The picture keeps the font's own placement of its glyphs when that fits the sequence's cells; otherwise one glyph per two cells, as before.
- **Flags on Windows** are postponed: they need a font with flag glyphs shipped with the app under a license the clean room allows (OFL) and in a format Windows 10 draws in color; downloading and choosing one is the owner's call (run 12 report).
- **vttest at 132 columns** gets its screenshot state, `terminal-vttest-132`: test 1 again after Return, at 132 columns.

**Spec impact:** docs/compatibility.md, section 2.

### 207. The soak compares windows and counts the Rust heap; the SFTP engine holds little memory (run 12)

Status: run 12 (plan, section 3.2; ROADMAP 2.4).

**Context.** Run 11 saw private bytes swing by 30–110 MB for 5–15 seconds under the soak's full load on OpenSSH, not on the russh soak (no SFTP there), took SFTP for the cause, and `scripts\soak.ps1` read such a swing in its last sample as growth (minute 15 against the last sample: 199.8 → 251 MB, +26 %). A spike of 498 MB came once at the start of a russh soak.

**Decision:**

- `crates/ssh/tests/sftp_memory.rs` measures the heap an SFTP transfer holds with a counting allocator, against OpenSSH: files of the soak's sizes (16 KB, 1 MB, 8 MB) up and down hold at most 2.5 MiB above what was live before (the 2 MiB in flight and the copy buffer); the test fails above 16 MiB. With `SHELLPP_SFTP_MEMORY=1` it prints where allocations of 512 KiB and more are made.
- Builds with `dev-tools` count the Rust heap through their global allocator (`apps/desktop/src/dev/heap.rs`), switched on only for `--soak`; the soak prints the live bytes and their peak every 5 seconds, and `scripts\soak.ps1` records them beside the process's numbers (`-SampleSeconds`), so a swing can be told to be the heap's or not. `SHELLPP_HEAP_TRACE=1` notes the places of allocations of 1 MiB and more. `-Load` runs one part of the load (`tabs`, `forward`, `sftp`).
- The summary compares windows, not samples: the first window after the warm-up with the last window of the run; memory by each window's lowest value, handles and threads by the median. A window needs at least 10 samples, so the soak samples every 15 seconds by default; with fewer there is no verdict (exit 3). On run 11's one-minute samples it reports −1.2 % (no false alarm); a real growth of 15 % is caught. An unknown `-Load` value stops the run (review of run 12).
- What the soak of run 12 found (25 minutes of the SFTP load alone, then 5 minutes of the whole load, samples every 5 seconds): with SFTP alone private bytes stay at 177.7–180.5 MB and the heap at 35–36 MB, so SFTP is not the saw-tooth's source (run 11 concluded it by elimination). Under the whole load the swings of private bytes (219 → 423, 260–316, 328–349 MB) are the heap's own (57 → 249, 91–130, 167 MB live), and no allocation of 1 MiB or more makes them. The code points to the scrollback design of D85 and D99 (a run with `-Load tabs` would show it directly; the 1 MiB trace cannot see rows of a few kilobytes): while output floods a tab, its grid keeps full-width rows (24 bytes a cell, about 3.6 KB a row at 150 columns) and gives them to the compact journal only after a pause of 100 ms or every 2 seconds, up to the 10 000 lines of the scrollback (about 36 MB a tab). Seven tabs running `seq 1 20000` in a loop reach +100–190 MB for seconds, which goes back at each compaction; nothing grows. The spike at the start of a russh soak (498 MB) is the same, larger: that run types `seq 1 20000` into all ten tabs at once. Not changed: compacting by row count during a flood brings back the churn that made a flood a quarter slower (D99); a bound on the full rows, measured with `scripts\measure-terminal.ps1`, is for the owner to weigh (run 12 report).

**Spec impact:** none (test and measurement tools); ROADMAP 2.4.

### 208. The red CI of runs 9–11: a dependency advisory, a shell lint, and tests that assumed the machine (run 12)

Status: run 12 (plan, section 3.4). GitHub Actions are off (D204); the causes come from the logs of the last runs (`gh run view --log-failed`, nothing run on GitHub).

**Context.** The workflow "CI" failed on every push since f92b48e (run 40093211710 on 560ea7b read):

- Licenses and advisories: RUSTSEC-2026-0387 (`crypto-bigint` 0.7.5: branches in operations meant to be constant-time) and RUSTSEC-2026-0385 (`ctutils` 0.4.3 in the root lock file, 0.4.4 in the server's: non-constant-time mask generation and `u128` comparisons). Both are in the path of RSA, ECDSA and the SSH keys (`rsa`, `elliptic-curve`, `russh`, `ssh-key`).
- Lint: shellcheck SC2015 in `tests/openssh/lab.sh`.
- macOS: `openssl rsa -traditional` does not exist in macOS's LibreSSL.
- Windows: the runner has PowerShell 7, so the local tab is "PowerShell 7", and the test expected "PowerShell".
- Linux: the PuTTY row and its error are shown on Windows only; dates are shown in UTC outside Windows (the local zone is read on Windows only, `crate::dates`); one session test got `Cancelled` instead of `Locked` when a lock met an open password prompt; its `select!` is biased toward the lock, it passed in every local run and on Linux in Docker (run 12), and it is left as a CI flake to watch.
- SSH integration: the puttygen container writes its files as root with mode 0600; the test, as the runner's user, could not read them.

**Decision:**

- `crypto-bigint` 0.7.7 and `ctutils` 0.4.5 in both lock files (semver-compatible); `cargo deny check advisories` is clean in both workspaces. The update moved one more edge in the root lock file: `tempfile` now uses `getrandom` 0.3.4, which was in the tree already, instead of 0.4.3.
- The script, the tests and the puttygen image are fixed as listed: an `if` in `lab.sh`; the OpenSSL test runs with OpenSSL 3 or later only; the local tab test expects the name of the machine's default shell; the PuTTY error is visible on Windows only and the date says UTC outside Windows; `generate.sh` gives its files to the owner of the mounted folder.
- `scripts\verify-linux.ps1` builds the client and runs the workspace's tests on Linux in Docker (`tests/linux/Dockerfile`, the CI's GPUI packages), outside the full criterion.
- On Linux in Docker (run 12) the client builds (`cargo build -p shellpp-desktop`, 2 minutes) and the workspace's 1 666 tests pass (the desktop's 780, 2 ignored), with the fixes above.

**Spec impact:** none; ROADMAP 3 and 6.

### 209. A sign-in the merge refuses as another account's revokes its session (run 12)

Status: run 12 (plan, section 3.6; ROADMAP 2.2, "Sign-in to another account", D200). Reviewed: an earlier version refused before pre-login whenever the email differed from `local_auth.kdf_email`, which also refused an account whose email changed on another device (the server has `POST /v1/account/email`, SP 5.4); it was dropped.

**Context.** A device bound to an account, signed out, that signs in to another account goes through pre-login (the other salt reads as changed settings, SP 4.2), the user types the password and the login succeeds; only the merge then refuses (`previous_account`, D197), and the session the login opened stayed live. On the same server the login itself is refused, because the device id belongs to the first account's user (SP 4.7); the case is another server.

**Decision:** When the merge refuses `previous_account`, the client revokes the refresh token of the login it just made (`POST /v1/auth/token/revoke`) before it reports the refusal; revoking the token family deletes its access tokens too (`server/src/model.rs`). A failed revocation is logged and does not change the refusal. The email is not compared before pre-login: a changed email and another account look the same until the account bundle comes back. Not covered (review of run 12): the other refusals after a login (a rebind's `rolled_back` or `integrity`, a merge's `forged_epoch` or `newer_epoch`) still leave their session live, and the login's `devices` row stays under the other account; ROADMAP 2.2.

**Spec impact:** docs/sync-protocol.md, section 4.4 (sign-in); ROADMAP 2.2.

### 210. The owner's answers to run 12's report: `cargo deny` in the local criterion, dates on Linux, flags stay letters (run 13)

Status: run 13 (plan, section 2.1; ROADMAP 5, the questions after run 12).

**Context.** Run 12's report left six questions (ROADMAP 5) and two notes (GitHub Actions, the contact address of `CODE_OF_CONDUCT.md`).

**Decision:**

- **Flags on Windows**: no font ships. Windows has no flag glyphs and Windows Terminal draws them as letters too; a known limitation (`docs/compatibility.md`, the guide). Supersedes the open part of D206.
- **Flood rows** (D207): not bounded. A flooding tab's full rows are a short burst, not a growth, and the throughput matters more.
- **`cargo deny` in the local criterion**: `scripts\verify.ps1` fetches RustSec's database (`cargo deny fetch db`) and runs `cargo deny --locked --offline check` for both workspaces with the configs of CI's "Licenses and advisories" job and the same cargo-deny (0.20.2; another version fails the steps). The yanked-crate check reads cargo's cached index, which a resolve never refreshes for locked crates, so `cargo update --dry-run` refreshes it first for both workspaces (the lock file is not written), as CI's fresh index is (review of run 13). Without the network the fetch and index steps only warn (WARNING, named in the summary) and the checks use the copy already downloaded; with no copy they fail, as a step that cannot run fails (D205). A new advisory stops main until it is handled: a dependency update, or an exception in the config with its reason and a review date, a decision here and a line to the owner.
- **Dates on Linux**: the local zone is read there before release 1.0. Run 13 reads it on Linux and macOS through the C library already linked (`tzset` and `localtime_r` from `libc`: `TZ` as a zone name, a zone file or a POSIX rule, otherwise `/etc/localtime`, with each date's daylight saving rule); no new dependency. A zone with no offset at a time and half a year from it counts as UTC, which is also the library's answer for a zone it could not read, and the time then says "UTC" as before. The format stays ISO (`2026-10-21 18:04`): the locale's format (`LC_TIME`) is not read. The tests run each zone in a child process of the test binary, since `TZ` belongs to the process.
- **VS16 emoji take two cells** (D206): accepted.
- **Linux on a real desktop** (the window, Wayland or X11, the Secret Service keychain, the fonts): checked in the release preparation run.
- **GitHub Actions** stay off until the repository is published; public repositories get the standard runners free, and then they are turned on.
- **The contact address** in `CODE_OF_CONDUCT.md` is replaced with a project address the owner will give in the release preparation run; whether the git history that holds it (b4aab0de, fd654149) is rewritten before publishing is decided there. Not touched now.

**Spec impact:** CLAUDE.md (the criterion), `scripts\verify.ps1`, docs/security.md (the supply-chain table), docs/compatibility.md and the guide (flags), ROADMAP 2.4 and 5.


### 211. The mobile app: the owner's description and choices (run 13)

Status: run 13 (plan, section 2.2). Supersedes the Mosh part of D204.

**Context.** D204 set the mobile client's line (Android only, a Samsung S25, full sync of the personal vault from day one, Mosh with it); run 12 studied the Rust core for it (`docs/android-feasibility.md`). The design run needed the owner's own description of the case, the requirements of the terminal and the scope of a first version.

**Decision:** The owner's words are recorded in `docs/mobile/owner-notes.md` (Russian as said, with an English translation), and they are the only source of the mobile app's behaviour besides the accepted desktop specifications; the mobile specifications mark everything else [decision], and nothing comes from another mobile client (clean room).

- The case: now and then, away from home, SSH to one's own servers (logs, restarting services, nano, `htop`, downloading files); full sync with the owner's server (a public domain, Let's Encrypt) from the first day.
- The terminal's main requirements: putting the cursor on the wanted character, and selecting the wanted text (for example a piece of a configuration file, to delete it); every extra key (Esc, Tab, Ctrl, Alt, the arrows, `|`, `/`, `~`, `-`).
- Entry: the master password.
- The first version: SSH, host groups, the Keychain, SFTP, known hosts. Snippets, history, autocomplete and the rest come in later versions.
- No iPhone app; Mosh comes later (not with the first version, as D204 had it); fingerprint unlock is an option, off by default [decision].

**Spec impact:** `docs/mobile/owner-notes.md` (new), the mobile specifications of run 13 (`docs/mobile/`), ROADMAP 3.1.


### 212. The Rust core for Android: an added CA beside the platform verifier, the "mobile-ready" check in the local criterion, the session layer and the terminal model as crates of their own (run 13)

Status: run 13 (plan, section 3.3; `docs/android-feasibility.md`).

**Context.** Run 12 found that `shellpp-core` does not build for Android: `crates/core/src/sync/tls.rs` gives the added CA of an origin (`TrustMode::Ca`, D187) to `rustls_platform_verifier::Verifier::new_with_extra_roots`, which the crate does not offer on Android, where the platform verifier asks the system's trust manager and takes no extra roots. The "mobile-ready" check of `docs/architecture.md` 2.3 ran only in CI, which is off (D204). The session layer and the terminal model, which a phone client needs, lived inside the desktop app.

**Decision:**

- **An added CA on Android** is checked beside the platform verifier (`CaBesideSystem` in `tls.rs`): the chain against the CA alone first (webpki, as the tests' fixed roots are), the platform verifier when the CA does not vouch for it. When both refuse, the CA's reason wins unless it is only "unknown issuer", so a certificate the CA issued for another name or out of its dates is refused for that, as with both roots in one store. Windows, Linux and macOS keep `new_with_extra_roots` (no change); the combination is unit-tested on every platform with a fixed "system" root.
- **The session layer and the terminal model are crates of their own**, `shellpp-sessions` and `shellpp-terminal`, without GPUI, and build for Android; the desktop re-exports them under their old paths (`crate::sessions`, `crate::terminal::{model, journal, ...}`) and keeps its GPUI glue, drawing and input. The session layer's ties to the desktop became options (`ManagerOptions::local_title`, `::authenticator`, the latter set by the app to the system's WebAuthn) and the autocomplete limit a core constant (`shellpp_core::history::AUTOCOMPLETE_COMMANDS`). The model's palette takes plain RGB (`PaletteColors`); the desktop makes it from its terminal themes. The autocomplete slot's loading protocol is methods of the slot (`list_state`, `finish_load`), so a view on any toolkit drives it. Two wrappers in the desktop (`ViewTerminal`, `SessionSink`) join the two crates' types, as the orphan rule asks. `shellpp-sessions` links `shellpp-term-local` on every target (it builds for Android); a phone client never opens a local or serial session; leaving it out of phone builds is the mobile runs' to decide. Tests move with their code; those that need the desktop's themes or its test profile stay in the desktop. Behaviour is unchanged.
- **The "mobile-ready" check** is a step of `scripts\verify.ps1`: `cargo check --locked --target aarch64-linux-android` of the crates a phone client shares, with the NDK's clang (API level 26) for the C parts. The NDK is found from `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT` or the newest one of the Android SDK; without one the step says SKIPPED with the reason and the summary names it, never a green mark; a missing Rust target fails the step with the command that adds it.

**Spec impact:** docs/architecture.md 2.1-2.3 (the crates, the dependency graph, the check), docs/android-feasibility.md 6, CLAUDE.md.


### 213. The Android app's stack: Kotlin screens and terminal view over the Rust core through a small JNI layer (run 13)

Status: run 13 (plan, section 3.4); `docs/mobile/architecture.md`.

**Context.** `docs/android-feasibility.md` 5 listed the ways to join the Rust core to an Android interface (UniFFI, JNI by hand, a command protocol, an interface in Rust, Kotlin Multiplatform) and left the choice to the design run. The owner's main requirements of the terminal are putting the cursor on the wanted character and selecting the wanted text (D211).

**Decision:** A Kotlin app: Jetpack Compose screens, the terminal a custom Android `View` drawn on a `Canvas`. The Rust core (`shellpp-core`, `shellpp-ssh`, `shellpp-sessions`, `shellpp-terminal`) is one library, `libshellpp_android.so` from the crate `shellpp-android` (`apps/android/native`), with a small hand-written JNI layer (the `jni` crate already in the graph): JSON commands for the core's service API, whose DTOs are already `serde`; direct calls for sessions, the terminal's hot path (a binary screen snapshot, cursor, modes, input bytes) and file transfers (SFTP writes to a file descriptor the Storage Access Framework gave). Secrets cross as wiped byte arrays, never as `String`s, and the dispatcher logs only command names and error codes. Kotlin waits on Rust (blocking "await wake" and "next event" calls on its own threads) instead of Rust calling back into the JVM. Handles are table indices, never pointers; no panic crosses JNI. Live sessions (dropped ones waiting to reconnect, and SFTP transfers, included) stay in a foreground service (`specialUse`); `Core::tick` runs every second while the process lives; sync runs after unlock, on resume, after changes and when a network comes back, without a periodic cycle. Fingerprint unlock is "remember on this device" behind the biometric prompt: the device key is wrapped by a non-exportable `AndroidKeyStore` key without an authentication binding (the core reads it during rotations), used at a cold start only, as the core's rule RR8 says; after a lock the password is asked. With the default (master password) nothing is stored. A tap walks the cursor with the arrow keys with feedback from the model, in full-screen programs too; "Cut in nano" is built on the same walks. `rustls-platform-verifier` is initialized in `Application.onCreate`, and its Kotlin component is vendored (its Maven repository is on GitHub, not an allowed source). Reviewed in run 13: the nano case, the device key rule, the clipboard in the background, the input method and SFTP were taken in. Rejected: UniFFI (a generator and JNA for little gain at this size; to revisit if the command set outgrows JSON), JNI typed by hand for every call, a command protocol only (terminal frames), an interface in Rust (input methods, selection handles and accessibility are young where the owner's pains are), Kotlin Multiplatform (no iPhone app).

**Spec impact:** `docs/mobile/architecture.md` (new), ROADMAP 3.1.


### 214. The Android app's screens, and how its terminal puts the cursor and selects (run 13)

Status: run 13 (plan, section 3.5); `docs/mobile/ui-spec.md`.

**Context.** The first version's screens had to come from the owner's words (D211) and the accepted desktop specifications only, with the terminal's two pains (the cursor on the wanted character, selecting the wanted text, deleting a selected piece in an editor) worked out in detail.

**Decision:** `docs/mobile/ui-spec.md`: four places (Hosts, Sessions, Keychain, More), the desktop's tokens in dp and sp with a 16 sp body and 48 dp touch targets, first run (a new vault or "Restore from sync"), unlock by the master password (fingerprint an option, off), auto-lock 15 minutes by default on the phone, fingerprint unlock at a cold start only (the core's device key rule), the desktop's data and rules everywhere. The terminal:

- **The cursor**: when the program asked for mouse reports, a tap is a click at that cell; otherwise the terminal walks the cursor there with the arrow keys with feedback from the model (one arrow, wait for the program, read the cursor, then a few arrows at a time): along the typed line on the shell's command line, rows then columns in a full-screen program (nano as it comes, vim); a pager that scrolls with arrows gets one. For precision a handle under the cursor can be dragged with the magnifier after a tap, as in Android's text fields; a gesture table says what each gesture does on the shell's screen and in a full-screen program.
- **Selection**: long press selects a word (the desktop's double-click rule), handles extend it cell by cell into the scrollback, a floating toolbar has Copy, Paste, Select line, Select screen; "Select mode" makes the next drag a free selection.
- **Deleting a selected piece**: on the shell's command line, Delete walks the cursor just past the selection and sends one Backspace per character; while nano's shortcut rows are on the screen, "Cut in nano" walks to the selection's start, sets nano's mark (Alt+A), walks just past its end and cuts (Ctrl+K). Nothing is deleted unless every walk landed exactly. The app does not know which program runs, and says so.
- **The extra keys**: two rows without scrolling, Esc, Tab, Ctrl, Alt and the arrows, then a keyboard toggle, `|`, `/`, `~`, `-`, Home, End, PgUp, PgDn; Ctrl and Alt sticky (a tap arms, a double tap locks); excluded from the system's edge gestures.
- Reviewed in run 13: the conflicting gestures of a scrolling keys row, the missing precise placement and keyboard toggle, deleting after a short walk, the desktop's per-field inheritance, autosave and key deletion, the sign-in steps (TOTP, D189, D200, the mobile KDF ceiling), reconnects waiting for unlock, auto-lock counting key input, Recently deleted, and the missing states were taken in.
- Pinch zooms the font (8-28 sp); the size goes to the server when a change settles (150 ms); sessions live in a foreground service, and the desktop's reconnect policy handles a lost network.

**Spec impact:** `docs/mobile/ui-spec.md` (new), ROADMAP 3.1.


### 215. The Android prototype: quick connect on the real core, its terminal, and Android 17's local network permission (run 13)

Status: run 13 (plan, sections 3.7 and 3.8); `apps/android`, `scripts\verify-android.ps1`.

**Context.** The plan asked for an APK that connects over SSH and gives a terminal with the extra keys, cursor placement by tap and selection at least in a basic form, surviving the app going to the background; without the vault and sync.

**Decision:**

- **The core on a throwaway vault**: the prototype opens `shellpp-core` on a vault in the app's files that is made again at every start with a random password nobody knows, so a quick connect runs the real session layer (`shellpp-sessions`): the host key question, prompts, the reconnect policy. Known hosts last until the app restarts. The first version replaces it with the user's vault (M1).
- **The binding** is the crate `shellpp-android` (`apps/android/native`): an engine (sessions and their terminals by handle), the screen snapshot of architecture 6.1, the input mapping (a copy of the desktop's sequences until it moves into `shellpp-terminal`), the cursor walk with feedback (ui-spec 9.3), the JNI exports inside `EnvUnowned::with_env`. It is a member of the client workspace and its logic and an end-to-end test against the russh test server run on the computer.
- **The app is plain Android views without AndroidX** [decision for the prototype]: the quick connect form, the terminal view, two rows of extra keys, the foreground service. Compose comes with the first version's screens (architecture 1). Back closes the prototype's session (it has no Sessions screen yet); leaving the app keeps the session in the foreground service.
- **Android 17's local network permission**: an app that targets API 37 reaches no address of the local network (10.x, 172.16.x, 192.168.x; the emulator's host at 10.0.2.2) without `android.permission.ACCESS_LOCAL_NETWORK`; the connection simply times out. The prototype declares it and asks for it with the notification permission. The owner's servers at home need it too; servers at public addresses do not.
- **Checks**: `scripts\verify-android.ps1` runs the binding's Rust tests, builds the debug APK (both ABIs; Gradle runs cargo with the NDK's clang) and the Kotlin unit tests; `-Emulator` adds the end-to-end test on the emulator against OpenSSH in Docker. Outside the full criterion, which keeps its "mobile-ready" step.

**Spec impact:** `docs/mobile/architecture.md` (the local network permission), `docs/mobile/README.md` (new), CLAUDE.md, ROADMAP 3.1.


### 216. A sign-in refused after its login revokes that login's device, for every refusal (run 13)

Status: run 13 (plan, section 3.9; ROADMAP 2.2, "Sessions after a refused sign-in"; extends D209).

**Context.** D209 revoked the session of a login whose merge refused `previous_account`. The other refusals after a login (a rebind's `rolled_back` or `integrity`, a merge's `forged_epoch` or `newer_epoch`, and a personal `kek_wrap` that opens to nothing) still left that session live, and the login's device row stayed active in the account's list.

**Decision:** Every refusal after the login and before the device is bound revokes what the login opened, in one place (`finish_sign_in`): `DELETE /v1/devices/{device_id}` with the login's access token, which deletes every token of the device in that account and marks its row revoked, and, when that fails, `POST /v1/auth/token/revoke` with the login's refresh token. Best effort: the refusal stands either way. "Before the device is bound" is read from the core itself: every binding holds its login's access token right after it is written, so a refusal while that token is not held revokes; the one refusal after a binding (a rebind to a rolled-back server whose key matches, which keeps its session for the re-post of the epochs, section 5.1 of the protocol) does not. The protocol deletes no device row: the row stays in the account's list, marked revoked, which is what a sign-out leaves too. Tests through a hostile proxy: `forged_epoch` and `newer_epoch` at the merge and a `kek_wrap` that does not open, each with no live token of the login, its device row revoked, nothing local changed and the account's two devices still syncing; D209's test now also checks the device row; the rolled-back server's test (a binding kept) still re-posts its epochs. A rebind's `rolled_back` before binding (a server whose chain does not reach the device's key) goes through the same path and was not built as a scenario of its own.

**Amended after the review of run 13:**

- Any failure after the login and before the binding revokes, not only a refusal: a lock during the merge, a server error (`5xx`) of the key chain or the merge's pull, a malformed answer, a store error; at a first run every retry makes a new device id, so each failed restore left another active row. A failure with no answer at all (no network, a timeout) sends no revocation, which would wait as long again. A login whose answer has tokens but no account bundle revokes its refresh token family.
- A rebind (the account's personal vault is the device's own) revokes only the login's refresh token family: the device's row was there before this login and stays active, so D185 still exempts it from the email's soft limit and the other devices do not list it as revoked. A row this login made (a first run, a merge into the account) is revoked as before. A later login with a revoked device id restores the row (`revoked_at` cleared), so a merge interrupted this way can be retried.
- "Before the binding" is a count of the bindings sign-ins wrote, which a lock does not clear (the held access token is cleared by a lock, so a lock right after a binding would have revoked a device just bound).
- The TOTP step uses the device id the login was made with (kept with the pending sign-in); a first run made a second id there, so the tokens were another row's than the device's and a refusal revoked a row that did not exist.
- Tests: a refused first run (its row revoked, a second sign-in works), a refused rebind of a signed-out device (its row stays active, only the login's family goes, it signs in again), a `503` of the merge's pull (the login revoked), no access token of a revoked login left, and the TOTP sign-in's current device equal to the device's own id.

**Spec impact:** docs/sync-protocol.md 4.4 (sign-in), ROADMAP 2.2.


### 217. The core refuses the recovery key in local mode; a new vault after a bound one keeps the weaker-settings check (run 13)

Status: run 13 (plan, section 3.9; ROADMAP 2.2, "Keys").

**Context.** Two gaps the review of run 12 left: the core still showed and reissued the recovery key in local mode, where the app never offers either (D29; only the interface hid them), and after "Start with a new vault" the next first sign-in had nothing to compare the server's KDF settings with, so D189's check of weaker settings did not run although the set-aside database was bound to an account.

**Decision:**

- `recovery_key_show` and `recovery_key_reissue_prepare` (and `recovery_key_reissue` through it) refuse with `wrong_state` on a device without a server account, before the password is checked (no attempt is counted, nothing is derived). A device out of its account still gets `account_binding` first for a reissue, as before. Tests of the local recovery paths turn the refusal off on their thread with `shellpp_core::service::local_recovery_key::allow` (`test-hooks` only, never in a release build): five core tests, one server test and the desktop's key dialog tests, whose core stays in local mode.
- A sign-in on a device whose own database has nothing to compare with looks at the set-aside copies of its data folder (`shellpp.db.<stamp>[-<n>].old`, never a `-failed` one) and takes the KDF settings of the newest one that was bound to an account (a `server_account` row or a vault with its account binding), reading it without opening it for writing. It is the reference of the weaker-settings check only: its salt and settings are another vault's, so the check of changed settings (D200) does not use it. Tested at the first run after "Start with a new vault" and in the new local vault after it, against a server that answers the mobile preset (refused, no login sent) and one that answers the same settings (not refused).

**Amended after the review of run 13:** the key in the grant a first run opens (`recovery_key_granted`) is refused in local mode too. The set-aside copies are ordered by stamp and number (`-2` after the first copy of that second, `-10` after `-2`), read with the core's lock released, and a copy without a `-wal` beside it is opened immutable, so reading leaves no `-wal` or `-shm` that a read-only connection could not remove. A device out of its account still gets `wrong_state` for Show and `account_binding` for a reissue (not unified: the interface hides both in local mode).

**Spec impact:** docs/security.md (unlock and the recovery key), docs/sync-protocol.md 4.2, ROADMAP 2.2.
