# Roadmap

State of the project on 2026-10-23, after run 8 and the cleanup that followed
it. What each stage is comes from `SPEC.md` ("Work order"); the reasons behind
the choices are in `docs/decisions.md` (D-numbers below). "Done" means the code
is in main with the full test criterion of `CLAUDE.md` green; the owner's
acceptance of a stage (SPEC, "Work order") is a separate step.

## 1. Stages

| Stage | Scope (SPEC) | Status | Notes |
|---|---|---|---|
| 0 | Design documents, theme and base components | done | `docs/architecture.md`, `data-model.md`, `storage-format.md`, `sync-protocol.md`, `security.md`, `design-system.md` |
| 1 | Skeleton, encrypted storage, master password, Hosts screen and host panel | done | UI rebuilt from Tauri + React on GPUI Kit (D27) |
| 2 | SSH terminal (2a), local terminal, agent, splits, new tab page (2b) | done | |
| 3 | Keychain, import, built-in agent, export | done | ssh config, native JSON, known_hosts, PuTTY sessions (D140), CSV (D141); FIDO2 signing and making keys on the device (D143); see 5 for the FIDO2 gaps |
| 4 | SFTP | done | names that are not UTF-8 on the server are a known limitation (D50, item 12) |
| 5 | Port forwarding, jump hosts, proxy, snippets | done | |
| 6 | Logs, settings, shortcuts, highlighting | done | interface scale (run 6) |
| 7 | Sync server and client, several devices | personal vault done | team parts belong to stage 8; what is left is in section 2 |
| 8 | Team vaults, roles, invitations, sharing | not started in code | schema and push checks are ready (`docs/stage8-readiness.md`); section 3 |
| 9 | Polish | mostly done | workspaces (in Export and Import Data since run 5), broadcast, Telnet (D87), serial (D88, D95), update check (D97) with a signed manifest (D170); releases need the signing key (section 4.4) |
| 10 | Later stages | partly, ahead of order | autocomplete (D142) and script generation (D144) are built; mobile apps, shared sessions, cloud provider import and Mosh are not started (Mosh's toggle only saves its value, D29) |

## 2. Stage 7: what is left

The personal vault syncs between devices through the owner's own server:
server on SQLite and PostgreSQL, registration, sign-in with merge, the sync
cycle, conflicts, Recently deleted, server restore, rotations, recovery through
the server, the interface and the Docker deployment, all covered by tests
(two clients and a server on both backends, a hostile server, the
"no data is lost" property, fuzzing, load).

### 2.1 Not built

- Registering or signing in on a device whose vault is bound to another
  account (the previous account's archive, data-model 18.5): refused with an
  error.
- Publishing the SSH ID again; changing the email.
- Re-issuing the recovery key on a bound device shows the key after the server
  commits it, not before sending (D102).
- "Remember on this device" is turned off when a rotation comes from another
  device (question 4.1, item 3).
- The duplicates hint (UI 13.7) is on Hosts, Known Hosts and Snippets only;
  Keychain, Port Forwarding, workspaces and themes show their pairs only in
  Review.
- Export from Recently deleted: the "OpenSSH files" format exports the vault's
  keys, not the trash.

### 2.2 Review findings left open in run 7

Each needs the owner's decision (section 4.2); none loses data as built.

- Argon2 verification runs inside the database transaction; a burst of
  unauthenticated logins blocks SQLite.
- `B64` and `Secret` of `crates/sync-api` are not wiped on drop.
- The server secret stays in the process environment; `totp-rs` keeps copies
  of the TOTP secret.
- The vendored russh reads an empty `CHANNEL_DATA` as the end of the stream,
  which closes an SFTP session.
- Natural keys of the dedupe take a host's own port and user, not the inherited
  ones (data-model 18.6); groups, snippets and rules compare raw parent ids.
- The server's `recovery_wrap` is checked only after the first pull.
- An abandoned sign-in merge leaves an extra vault row.
- The `failed` set and `stale_key_epoch` do not follow the SP 6.5 table.
- Merging two copies of one key takes two transactions (`keys_merge`).

### 2.3 Findings of run 8 (agent C) in the sync code

Reported by the security review of run 8 and not fixed since; not
re-verified during the cleanup, so each is checked again before work on it.

- (medium) "Lost or stolen" does not re-issue the recovery key: whoever holds
  the old recovery key can still complete a recovery after the rotation.
- The recovery key is reported as never shown, while "Restore from sync"
  asks for it.
- Sign out with "Also delete the local copy" fails: `start_new_vault` needs the
  Locked state.
- Re-encrypting session records at sign-in can lose them and is not retried
  after a failure.
- A changed pinned certificate cannot be confirmed without signing out; the
  interface has no "Add a CA" or "system trust", which the server README
  promises.
- The sync dialogs set no initial focus, and Enter in a field does not submit
  the step.
- Smaller: the server's error code is shown verbatim; the TOTP QR code takes the
  server URI without checking its scheme; no limit on the size of a server
  answer; the refresh token is not bound to the origin; the server README's
  `SHELLPP_TRUSTED_PROXIES` value for compose puts every client in one rate
  limit bucket; the backup script has no `pipefail`.

### 2.4 Checks not obtained

- The 120-minute test against OpenSSH in Docker under all three loads at once
  (run 7): 110–120 minutes without growth of memory, handles or threads were
  seen, a clean 120 minutes was not.
- The flaky test search `scripts\find-flaky.ps1 -Runs 15` (run 8): 7 of 15
  runs done, one real cause found and fixed; the rest needs a quiet machine.
- The owner's own install: the server in the homelab (`server/README.md`,
  `docs/guide/06-sync.md`) and two devices.

## 3. Stage 8: what is left

All of it is code; the list with references is `docs/stage8-readiness.md`.

1. HPKE wraps of the vault key for each member (the `hpke` crate, version and
   audit status open, security.md 15).
2. One session (key ring, index, clock) per vault instead of one per account.
3. Member keys, fingerprints, `member_pin` records, the verified flag.
4. Invitations: the server's state machine (tables exist, endpoints do not),
   the client's accept flow, the interface (UI 6.x).
5. Roster and rotations: `roster_entry` records and their type rule (missing in
   `engine::type_rule`), rotation on member removal, background re-encryption,
   `epoch_conflict`.
6. Archives of a removed member (read-only copy).
7. The restore procedure for every bound vault, not only the personal one.
8. A per-user lock in the quota step (today two pushes to two vaults of one
   owner on PostgreSQL can overshoot the quota).
9. Interface: vault switcher, sharing dialog, member list, invitation inbox,
   the "server state rolled back" re-post for team vaults; the duplicates hint
   hidden for viewers and archives.

Before it starts, the owner answers 4.1 items 1 and 2 (now or with stage 8).

## 4. Open questions to the owner

### 4.1 Run 7

1. Move the Argon2 check out of the database transaction now or with stage 8?
2. Make `B64` and `Secret` in `sync-api` zeroizing now, or accept until stage 8?
3. "Remember on this device" after a rotation on another device: keep turning
   it off, or recreate the device row on rebind (needs the key from the OS
   key store)?
4. Prelogin on a bound, signed-out device: when the server sends weaker KDF
   parameters (still above the floor) than the local ones, refuse or ask?

### 4.2 Run 7 review findings

- TOTP limit: keep SP 10.1 (10 failures in 15 minutes, no growing delay), or
  add a growing delay?
- Confirm that the quota lock moves to stage 8.
- Identifiers from an import at the sign-in merge can leave an extra pair of
  duplicates: accept, or compare in two passes?
- A forged `key_epoch` in a header stops sync for good (data stays): accept, or
  give the user a way out in SP?
- The server secret: accept, read it from a file (`SHELLPP_SERVER_SECRET_FILE`),
  or fork `totp-rs`?
- The empty `CHANNEL_DATA` in russh: carry a second patch, or send the fix
  upstream?
- A device back after a long absence pushes its versions over edits made on a
  restored server (rule 0, D102): accept, or compare clocks on rebind?
- Each item of 2.2 from "Natural keys" down: finish it or accept it.

### 4.3 Run 8, agent B

- Turn on `fido2-hid` for macOS and Linux release builds (after a check with a
  real key)? PIN-protected keys over HID are not supported.
- CSV: the `notes` and `password` columns were added to the six of SPEC; the
  password is stored as the host's secret. Keep, or never import passwords
  from CSV?
- Autocomplete only in shells with bracketed paste (bash 5.1+, zsh, fish):
  confirm.
- Script generation: the API key is in the OS credential store, not in the
  vault (it does not sync); one provider type, OpenAI-compatible: confirm.

### 4.4 Run 8, agent C

- `ForwardAgent` from an ssh config read from an arbitrary path (D60, D172).
- Keys of the system agent sign through forwarding while the app is locked
  (Keychain keys do not).
- A forgotten password with "Remember on this device" cannot be changed (a
  change needs the current one); the way out is Export, a new store, Import.
- The release signing key: generate it and fill `release/minisign.pub` and the
  secrets (`docs/release.md`). Until then the release workflow stops and the
  app says it cannot check for updates.

### 4.5 The cleanup

- Three branches of run 7's subagents hold commits that are not in main
  (`git log main..<branch>` is not empty). Each commit has a counterpart in
  main with the same subject and an adapted patch; nothing from them was
  merged. They are renamed to `wip/salvage-<old name>`:
  - `wip/salvage-worktree-agent-a4cba5b8fd4d8ea93` (7bd7331): the first
    version of the duplicates side by side (UI 13.7); main has 91ff5ce;
  - `wip/salvage-worktree-agent-a5fe74c8fd03d303e` (cb5f06f, 289303e): the
    first version of the conflict comparison, the Conflicts list and the host
    badge (UI 13.6); main has e1d3bc3 and 119e38a;
  - `wip/salvage-worktree-agent-af46f228698138305` (05f1150, 5702f2b): review
    fixes of Keep other (UI 13.6); main has 9a7209b and dd68bab, but not the
    `user_deletable` helper that 05f1150 adds to
    `crates/core/src/service/records.rs`.

  Compare them with main (first of all `user_deletable`) and then delete
  them, or keep them?
- main has not been pushed since `5425f9c` (`origin/main`): push it, and when?

## 5. Technical debt

- **Vendored russh** (`third_party/russh`, D100): remove the patch when
  upstream releases the fix (CLAUDE.md lists every place it touches).
- **Secrets on the server and in `sync-api`**: sections 2.2 and 4.2.
- **Sync engine leftovers**: sections 2.2 and 2.3.
- **FIDO2 in the host panel**: the "+" credential menu still shows "In
  development" for FIDO2 (`apps/desktop/src/panels/host_panel/credentials.rs`,
  `open_credential`), although FIDO2 keys work since D143. On macOS and Linux
  signing needs the `fido2-hid` feature, off by default.
- **Stale text**: the "Status" section of `README.md` still says there is no
  application; the module comments of `apps/desktop/src/new_tab/mod.rs` and
  `page.rs` call "Create a workspace" disabled until stage 9.
- **Terminal compatibility** (`docs/compatibility.md`): VS16 emoji drawn two
  cells wide in one cell, ZWJ sequences, vttest 80/132 columns.
- **Test infrastructure**:
  - the criterion runs without `--locked`, so a stale `server/Cargo.lock` is
    rewritten instead of failing (it happened in run 8);
  - cargo copies `shellpp.exe` and `shellpp-test-sshd.exe` to the top of the
    target folder, so two worktrees sharing one target folder replace each
    other's binaries and lock them while running;
  - the OpenSSH scripts label their containers `dev.shellpp.test=openssh` and
    `tests/openssh/stop.sh` removes every such container; the PostgreSQL tests
    label theirs `shellpp-test=postgres`: neither tells parallel agents apart;
  - each server test binary on PostgreSQL starts its own container with
    `--rm` and `timeout 1800` (`server/tests/support/mod.rs`, `postgres`):
    the containers outlive the run by up to 30 minutes, while `CLAUDE.md`
    says the tests remove them.

## 6. Process rules

What the runs settled on, in addition to `CLAUDE.md` and SPEC.

1. **main stays green.** Code reaches main only with the full criterion of
   `CLAUDE.md` green on the exact commit that becomes main, run in a clean
   worktree with its own `CARGO_TARGET_DIR`. main moves fast-forward only.
   Before merging, also check `cargo metadata --locked` for both workspaces.
2. **No `git push` and no remote changes** without the owner's explicit request.
3. **Branches.** One branch per run or agent (`run<N>/<name>`). What cannot be
   finished in about 45 minutes or 3 attempts goes to `wip/<topic>`, and main
   stays green. At the end of a run, service branches and verify worktrees are
   deleted.
4. **Parallel agents.** Each has its own folder, branch and target folder
   (never shared, see section 5), its own range of decision numbers, its own
   Docker label `shellpp-test=run<N>-<agent>` and port range. Windows of the
   app (screenshots) run one at a time. No speed measurements while several
   agents build.
5. **Disk.** Check free space before a build: under 40 GB delete your own
   `target\debug\incremental`, under 25 GB `cargo clean`, and note it.
6. **Tests.** Never weaken, delete or `#[ignore]` a test to get green; a timing
   test waits for its condition with a margin. Session tests against a shared
   OpenSSH container run with `--test-threads=1`. Containers a run creates are
   removed by it; Docker Desktop is never stopped or restarted by an agent.
7. **Each stage**: reread the specs, plan in the run's PROGRESS, small
   Conventional Commits, tests, a review by a subagent that did not write the
   code, fixes, a full run, screenshots in both themes, a line in PROGRESS, the
   report.
8. **Decisions** go to `docs/decisions.md`. The next free number is D173;
   D104–D139 and D145–D169 stay unused (the per-agent ranges of run 8).
   Disputed ones also go to the report's owner section.
9. **Language.** Code, comments, commits and documentation in English; the
   English specifications are canonical and `docs/ru/` follows in the same
   commit; run reports in Russian.
10. **Run notes** (PLAN, PROGRESS, REPORT) live in `docs/overnight/`, which git
    ignores because they hold private server details; a finished run moves to
    `docs/overnight/archive/run-<N>/`.
11. **Secrets** are never printed, logged or committed; tests use keys and
    passwords generated on the fly.
12. **Clean room** (SPEC) applies to every change.
