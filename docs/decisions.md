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
