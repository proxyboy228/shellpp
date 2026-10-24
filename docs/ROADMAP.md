# Roadmap

State of the project on 2026-10-24, after run 10. What each stage is comes from
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
| 3 | Keychain, import, built-in agent, export | done | ssh config, native JSON, known_hosts, PuTTY sessions (D140), CSV (D141); FIDO2 signing and making keys on the device (D143), linked to a host as a key (D201); on macOS and Linux signing needs the `fido2-hid` feature, off until a check with a real key (D173) |
| 4 | SFTP | done | names that are not UTF-8 on the server are a known limitation (D50, item 12) |
| 5 | Port forwarding, jump hosts, proxy, snippets | done | |
| 6 | Logs, settings, shortcuts, highlighting | done | interface scale (run 6) |
| 7 | Sync server and client, several devices | personal vault done | run 10: a restore while a device is out of the account (D197, D198), backups that keep good copies (D199), "Also delete the local copy" deletes it (D197), changed KDF settings at sign-in (D200); what is left is in section 2; team parts belong to stage 8 |
| 8 | Team vaults, roles, invitations, sharing | after release 1.0 (D197) | schema and push checks are ready (`docs/stage8-readiness.md`); section 4 |
| 9 | Polish | mostly done | workspaces, broadcast, Telnet (D87), serial (D88, D95), update check (D97) with a signed manifest (D170); releases need the owner's signing key (`docs/release.md`) |
| 10 | Later stages | partly, ahead of order | autocomplete (D142) and script generation (D144) are built; mobile apps, shared sessions, cloud provider import and Mosh are not started (Mosh's toggle only saves its value, D29) |

## 2. Stage 7: what is left

The personal vault syncs between devices through the owner's own server:
server on SQLite and PostgreSQL, registration, sign-in with merge, the sync
cycle, conflicts, Recently deleted, server restore (also while a device is out
of the account), rotations, recovery through the server, the recovery key shown
and reissued, password changes and "Lost or stolen" through one request, the
certificate dialogs, the interface and the Docker deployment with an encrypted
backup recipe and a rotation that keeps good backups, all covered by tests (two
clients and a server on both backends, a hostile server, the "no data is lost"
property, fuzzing, load, injected faults, the deployment in Docker).

### 2.1 Not built

- Registering or signing in on a device whose vault is bound to another
  account (the previous account's archive, data-model 18.5) is refused; the
  message names the safe path (D197). The archive comes after release 1.0
  (section 4); the work of run 9 is on `wip/run9-archive` (f262bcf).
- Publishing the SSH ID again; changing the email (postponed, D173).

### 2.2 Review findings left open

None loses data as built.

- **Server restore** (D184, D198): a late edit with a stamp below the lost
  version's that the restored server gave a sequence still below the synced
  version's is pushed over without a copy (it needs a clock behind by more
  than the time between the edits and fewer writes after the restore than
  were lost). A hostile server can cause superfluous copies by re-serving old
  versions with high sequences in a restore.
- **Conflict copies**: a copy resolved with "Keep this" (a tombstone) is not
  counted by `has_copy`, so a later identical stale version makes one more
  copy (never a loss); recognising it needs a store of its own (a migration).
- **A purged deletion brought back** (run 10, D103): a device that pushes back
  a record whose tombstone the server purged meets another device's full
  resync, which pushes its deletion again; the record goes to Recently
  deleted on both, no copy.
- **Recordings** (D182): the re-encryption runs inline in `unlock()` and,
  after a sign-in, under the sync lock (a thread needs an `Arc<Core>`); a tiny
  window remains between its check that the file still exists and the rename.
- **Sign-in merge**: a merge that meets a forged epoch only stops (there is no
  reset before binding); the fixed-point matching is O(n·d).
- **Keys**: the core still allows Show and Reissue of the recovery key in
  local mode (only the interface hides them; forbidding it needs a test hook
  for some 15 tests that read the key); after "Start with a new vault" the
  next first sign-in does not compare the KDF with the set-aside store
  (D189).
- **Known limitations** (recorded, no work planned): the server's sign-in
  limits (D175, D185: a turn lost to an attacker at the moment a wait ends,
  the soft limit counted after Argon2id, counts in memory, the email map);
  a 500-item push answer may hold about 192 MB (D188); TOTP codes travel as
  plain strings; the short gap after the core's auto-lock in the D194 gate.

### 2.3 Checks not obtained

The verification run (section 3) is for these.

- The 120-minute test against OpenSSH in Docker under all three loads at once
  (run 7): 110–120 minutes without growth of memory, handles or threads were
  seen, a clean 120 minutes was not.
- The flaky test search `scripts\find-flaky.ps1 -Runs 15` (run 8): 7 of 15
  runs done, one real cause found and fixed; the rest needs a quiet machine.
  Run 9 saw `hosts_screen::quick_connect_opens_a_session_without_saving` fail
  once (a gpui "test is not deterministic" panic).
- The "no data is lost" property with large parameters: runs 9 and 10 ran 40
  seeds × 500 steps (5–9 minutes, green); `SHELLPP_PROPERTY_SEEDS=100`
  `SHELLPP_PROPERTY_STEPS=800` takes about 40 minutes.
- The full screenshot set (`scripts\ui-screens.ps1` without `-States`): runs
  9 and 10 took only their new and changed states, at 1280x800.
- The owner's own install: the server in the homelab (`server/README.md`,
  `docs/guide/06-sync.md`) and two devices.
- CI runs the deployment tests (`server-deploy.yml`, run 10) but not the
  server's own tests on SQLite and PostgreSQL, though section 9.2 of
  `docs/architecture.md` lists "sync end-to-end with PostgreSQL" in the Linux
  job (question in section 5).

## 3. The path to release 1.0

Release 1.0 ships without team vaults (D197).

1. **Run 10** — interface and small things (this state).
2. **The verification run** — the checks of 2.3: the long test, the flaky
   test search, the large property run, the full screenshot set at every
   size; no new features.
3. **The owner uses it** day to day and writes down the differences from
   Termius that matter.
4. **1–2 parity runs** — the list of step 3.
5. **The release preparation run** — the git history checked for anything
   private, the license, README, SECURITY.md, the platforms and code signing,
   the owner's minisign key for the update manifest (`docs/release.md`), and a
   check that stage 8 will later fit without breaking the data format.
6. **Release 1.0.**

After the release: the previous account's archive (its keys then wrapped under
the personal vault key, storage-format 6.2 changes with it; D197), then
stage 8 (section 4).

## 4. Stage 8: what is left (after release 1.0)

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

## 5. Open questions to the owner

The run 10 report (section 4) has the details.

- CI: add the server's own tests (SQLite, and PostgreSQL in Docker) to the
  workflows, as `docs/architecture.md` 9.2 says, or change the text?
- Restore while signed out (D197): a device that signed out before this
  version keeps no generation, so its next rebind keeps a copy of every
  version it replaces once (copies it may not need); accept, or skip the
  copies for such devices?

## 6. Technical debt

- **Vendored russh** (`third_party/russh`, D100, D191): two patches; remove each
  when upstream releases its fix (`third_party/russh/PATCHES.md`, `CLAUDE.md`).
- **Known limitations of secrets** (security.md): copies inside serde_json,
  reqwest, hyper and axum, and inside `totp-rs` on the server.
- **Sync engine leftovers**: section 2.2.
- **Terminal compatibility** (`docs/compatibility.md`): VS16 emoji drawn two
  cells wide in one cell, ZWJ sequences, vttest 80/132 columns.
- **One target folder per worktree** (a rule, no code): cargo copies
  `shellpp.exe` and `shellpp-test-sshd.exe` to the top of the target folder,
  so two worktrees sharing one target folder replace each other's binaries
  and lock them while running; every worktree builds into its own
  `CARGO_TARGET_DIR` (6.4).
- **Imported terminal themes** are not records yet (no theme import), so the
  duplicates hint of Settings → Terminal covers the highlighting rules only
  (UI 13.7).

## 7. Process rules

What the runs settled on, in addition to `CLAUDE.md` and SPEC.

1. **main stays green.** Code reaches main only with the full criterion of
   `CLAUDE.md` green on the exact commit that becomes main, run in a clean
   worktree with its own `CARGO_TARGET_DIR` (`scripts\verify.ps1`, with
   `-Deploy` after changes the deployment tests cover). main moves
   fast-forward only. The criterion runs cargo with `--locked` and the tests
   with `--no-fail-fast`, and starts with `cargo metadata --locked` for both
   workspaces.
2. **No `git push` and no remote changes** without the owner's explicit request.
3. **Branches.** One branch per run or agent (`run<N>/<name>`). What cannot be
   finished in about 45 minutes or 3 attempts goes to `wip/<topic>`, and main
   stays green. At the end of a run, service branches and verify worktrees are
   deleted.
4. **Parallel agents.** Each has its own folder, branch and target folder
   (never shared, see section 6), its own range of decision numbers, its own
   `SHELLPP_TEST_RUN=run<N>-<agent>` (the label `shellpp-run` of every test
   container; `tests/openssh/stop.sh --all` removes only its own) and port
   range. Windows of the app (screenshots) run one at a time. No speed
   measurements while several agents build.
5. **Disk.** Check free space before a build: under 40 GB delete your own
   `target\debug\incremental`, under 25 GB `cargo clean`, and note it.
6. **Tests.** Never weaken, delete or `#[ignore]` a test to get green; a timing
   test waits for its condition with a margin. Session tests against a shared
   OpenSSH container run with `--test-threads=1`. Containers a run creates are
   removed by it (the PostgreSQL test containers go with their test process);
   Docker Desktop is never stopped or restarted by an agent.
7. **Each stage**: reread the specs, plan in the run's PROGRESS, small
   Conventional Commits, tests, a review by a subagent that did not write the
   code, fixes, a full run, screenshots in both themes, a line in PROGRESS, the
   report.
8. **Decisions** go to `docs/decisions.md`. The next free number is D202;
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
