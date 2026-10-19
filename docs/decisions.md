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
