# Roadmap

State of the project on 2026-10-24, after run 9. What each stage is comes from
`SPEC.md` ("Work order"); the reasons behind the choices are in
`docs/decisions.md` (D-numbers below). "Done" means the code is in main with the
full test criterion of `CLAUDE.md` green; the owner's acceptance of a stage
(SPEC, "Work order") is a separate step.

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
| 7 | Sync server and client, several devices | personal vault done | run 9: the recovery key, password change and "Lost or stolen" (D178–D181, D195, D196), certificate dialogs (D187), server limits and secrets (D174–D177, D185), sync engine fixes (D182–D184); what is left is in section 2; team parts belong to stage 8 |
| 8 | Team vaults, roles, invitations, sharing | not started in code | schema and push checks are ready (`docs/stage8-readiness.md`); section 3 |
| 9 | Polish | mostly done | workspaces (in Export and Import Data since run 5), broadcast, Telnet (D87), serial (D88, D95), update check (D97) with a signed manifest (D170); releases need the owner's signing key (`docs/release.md`) |
| 10 | Later stages | partly, ahead of order | autocomplete (D142) and script generation (D144) are built; mobile apps, shared sessions, cloud provider import and Mosh are not started (Mosh's toggle only saves its value, D29) |

## 2. Stage 7: what is left

The personal vault syncs between devices through the owner's own server:
server on SQLite and PostgreSQL, registration, sign-in with merge, the sync
cycle, conflicts, Recently deleted, server restore, rotations, recovery through
the server, the recovery key shown and reissued, password changes and "Lost or
stolen" through one request, the certificate dialogs, the interface and the
Docker deployment with an encrypted backup recipe, all covered by tests (two
clients and a server on both backends, a hostile server, the "no data is lost"
property, fuzzing, load, the deployment in Docker).

### 2.1 Not built

- Registering or signing in on a device whose vault is bound to another
  account (the previous account's archive, data-model 18.5) is still refused
  with an error. The work of run 9 is on `wip/run9-archive` (f262bcf), with
  what its review found and the order to finish it in the run 9 report,
  section 5; it needs the owner's choice in 4.1 first.
- Publishing the SSH ID again; changing the email (postponed, D173).
- On a bound device, a pre-login whose `kdf` or `kdf_salt` differs from
  `local_auth` should say so and ask for the current account password (SP 4.2,
  stage 0); it does not.
- Run 10 (section 2.4): the duplicates hint on Keychain, Port Forwarding,
  workspaces and themes; export from Recently deleted in the "OpenSSH files"
  format; initial focus and Enter in the sync dialogs; the server's error code
  shown verbatim.

### 2.2 Review findings left open in run 9

None loses data as built, except where an item says so; each needs either
the owner's choice (4.1) or a small change in run 10.

- **Sync after a server restore** (D184): a device signed out during the
  restore cannot tell the restore from ordinary edits at its rebind, so a
  server version above its synced revision replaces its version without a
  conflict copy (residual 2; a restore the device notices keeps a copy). An
  edit below the synced revision whose stamp is older than the lost version
  (clock skew) is also pushed over without a copy (residual 1).
- **Conflict copies**: a copy resolved with "Keep this" (a tombstone) is not
  counted by `has_copy`, so a later identical stale version makes one more
  copy (never a loss).
- **Recordings** (D182): the re-encryption runs inline in `unlock()` and,
  after a sign-in, under the sync lock (a thread needs an `Arc<Core>`); a tiny
  window remains between its check that the file still exists and the rename;
  the `.rekey` wrapping key and AAD have no independent known-answer vector.
- **Sign-in merge**: a merge that meets a forged epoch only stops (there is no
  reset before binding); the fixed-point matching is O(n·d); `report.failed`
  can count a retried item twice.
- **Keys** (D178–D180, D195, D196): the rebind's recovery row built only after
  the first cycle has no test (it needs a fault hook in the server tests);
  the "not known" and "stored nowhere" outcomes of a key change have no
  fault-injection test; the core still allows Show and Reissue in local mode
  (only the interface hides them); a prepared change does not record its
  origin (the token binding of D188 stops a send elsewhere); after "Start with
  a new vault" or a set-aside, the next first sign-in does not compare the KDF
  with the set-aside store (D189).
- **Server limits** (D175, D185): a new network can lose its turn to an
  attacker who tries the moment each wait ends; the soft limit is checked
  before the Argon2id check and counted after it, so up to the number of slots
  (8 here) share one turn; the counts live in memory and a restart forgets
  them; a flood of failures for 50,000 other emails could push one email's
  soft count out of its map.
- **Backups** (D176): the prune can age out good backups after a long run of
  dumps of an empty database; the recipe test covers a restore within 15
  minutes of the backup (devices renewed since then sign in again, README).
- **Client limits** (D188): a 500-item push answer may hold about 192 MB
  (bounded and documented).
- **Secrets**: one-time TOTP codes travel as plain strings in the desktop and
  the core (short-lived, the server refuses a reused code); the D194 gate
  follows the interface's lock event, so there is a short gap after the core's
  auto-lock (as for Keychain keys).

### 2.3 Checks not obtained

- The 120-minute test against OpenSSH in Docker under all three loads at once
  (run 7): 110–120 minutes without growth of memory, handles or threads were
  seen, a clean 120 minutes was not.
- The flaky test search `scripts\find-flaky.ps1 -Runs 15` (run 8): 7 of 15
  runs done, one real cause found and fixed; the rest needs a quiet machine.
  Run 9 saw `hosts_screen::quick_connect_opens_a_session_without_saving` fail
  once (a gpui "test is not deterministic" panic).
- The "no data is lost" property with large parameters: run 9 ran 40 seeds ×
  500 steps (8 minutes, green); `SHELLPP_PROPERTY_SEEDS=100`
  `SHELLPP_PROPERTY_STEPS=800` takes about 40 minutes.
- The full screenshot set (`scripts\ui-screens.ps1` without `-States`): run 9
  took only its new states, at 1280x800.
- The owner's own install: the server in the homelab (`server/README.md`,
  `docs/guide/06-sync.md`) and two devices.

### 2.4 The next runs

- **Run 10, interface and small things:** the run 10 items of 2.1; the
  technical debt of section 5, including Docker labels per agent and a target
  folder per worktree; the low findings of 2.2 that need no owner decision.
- **The verification run:** the checks of 2.3 (the long test, the flaky test
  search, the large property run, the full screenshot set at every size), no
  new features.

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
6. Archives of a removed member (read-only copy), and viewing archives in the
   interface (the previous account's archive included, D173).
7. The restore procedure for every bound vault, not only the personal one.
8. A per-user lock in the quota step (today two pushes to two vaults of one
   owner on PostgreSQL can overshoot the quota).
9. Interface: vault switcher, sharing dialog, member list, invitation inbox,
   the "server state rolled back" re-post for team vaults; the duplicates hint
   hidden for viewers and archives.

## 4. Open questions to the owner

### 4.1 Run 9

The run 9 report (section 4) has the details and the alternatives.

- The previous account's archive: keep its key rows under the `kek`,
  `recovery` and `device` wraps and rewrite them at every key event (as
  storage-format 6.2 has it), or wrap the archive's keys under the personal
  vault key (changes 6.2, needs no rewrite)?
- D188: every bound device signs in once after the update; or a one-time
  migration that rewraps the stored refresh tokens?
- D190: "Also delete the local copy" sets the copy aside; or erase it, with a
  confirmation of its own?
- D194: the "Forward the system agent while locked" setting belongs to the
  device; or does a full restore bring it back, as it does
  `security.auto_lock_minutes`?
- D175 and D185: the numbers of the sign-in and TOTP limits.
- CI does not run the deployment tests (`SHELLPP_TEST_DEPLOY=docker`), while
  security.md 14.4 says the server's CI runs the backup recipes: add the step
  or change the text?
- D184: a device signed out during a server restore replaces its version
  without a copy at the rebind (2.2): accept, or add to the protocol the
  stamp a payload was derived from, so a rebind can tell a restore apart?
- D195: after Log out with the local copy kept, "My old password may be
  compromised" and Reissue are refused until the device signs in again.

### 4.2 The cleanup of run 8

- Three branches of run 7's subagents hold commits that are not in main; they
  are kept as `wip/salvage-<old name>` (D173). Each commit has a counterpart in
  main; run 9 compared the one piece that seemed missing: the `user_deletable`
  helper of 05f1150 is in main as `records::deletable`
  (`crates/core/src/service/records.rs`), with the same kinds and the same use
  in the conflict resolution. Delete the three branches now?

## 5. Technical debt

- **Vendored russh** (`third_party/russh`, D100, D191): two patches; remove each
  when upstream releases its fix (`third_party/russh/PATCHES.md`, `CLAUDE.md`).
- **Known limitations of secrets** (security.md): copies inside serde_json,
  reqwest, hyper and axum, and inside `totp-rs` on the server.
- **Sync engine leftovers**: section 2.2.
- **FIDO2 in the host panel**: the "+" credential menu still shows "In
  development" for FIDO2 (`apps/desktop/src/panels/host_panel/credentials.rs`,
  `open_credential`), although FIDO2 keys work since D143. On macOS and Linux
  signing needs the `fido2-hid` feature, off by default (D173: until a check
  with a real key).
- **Stale text**: the "Status" section of `README.md` still says there is no
  application; the module comments of `apps/desktop/src/new_tab/mod.rs` and
  `page.rs` call "Create a workspace" disabled until stage 9; SP 2.2 still
  speaks of cloudflared in the compose file.
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
    label theirs `shellpp-test=postgres` and the deployment tests
    `shellpp-test=deploy`: none tells parallel agents apart;
  - each server test binary on PostgreSQL starts its own container with
    `--rm` and `timeout 1800` (`server/tests/support/mod.rs`, `postgres`):
    the containers outlive the run by up to 30 minutes, while `CLAUDE.md`
    says the tests remove them;
  - `cargo test` stops at the first failing test binary: a run without
    `--no-fail-fast` hides the failures of the binaries after it.

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
8. **Decisions** go to `docs/decisions.md`. The next free number is D197;
   D104–D139, D145–D169 and D186 stay unused (the per-agent ranges of runs 8
   and 9).
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
