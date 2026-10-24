# Roadmap

State of the project on 2026-10-24, after run 11. What each stage is comes from
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
| 7 | Sync server and client, several devices | personal vault done | run 10: a restore while a device is out of the account (D197, D198), backups that keep good copies (D199), "Also delete the local copy" deletes it (D197), changed KDF settings at sign-in (D200); run 11, the verification run: the checks of section 2.3; what is left is in section 2; team parts belong to stage 8 |
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
- **Deleting the local copy** (D197): files that could not be deleted (a
  recording a running session holds open on Windows) reach only the log, as
  a count; the app is already at first run.
- **Sign-in to another account** (D200): its email differs in salt, so the
  device first says the settings changed; after the user types the password
  the auth key goes to the server and the login succeeds before
  `previous_account` refuses, and the tokens issued are not revoked.
  Comparing the email with `local_auth.kdf_email` before deriving would
  avoid it.
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

### 2.3 Checks

Run 11, the verification run, obtained these on a quiet machine; its report
has the numbers.

- The 120-minute test against OpenSSH in Docker under all three loads at
  once: a clean 120 minutes, no errors (34 733 HTTP downloads, 34 723 through
  SOCKS, 54 620 SFTP files), no growth: the lowest private bytes of each ten
  minutes 197 MB at the start and 195 MB at the end, handles 744–752, threads
  80. Private bytes go up by 30–110 MB for a few seconds at a time under the
  SFTP load (section 2.4); a five-minute sample that met one was the
  "320 MB peak at minute 15" of runs 6 and 7.
- The flaky test search: 8 runs in run 11, none failed (7 more in run 8).
  `hosts_screen::quick_connect_opens_a_session_without_saving` (run 9) failed
  17 times in 4 756 runs on a CPU loaded on purpose: the terminal's parser
  thread could close the wake channel of the view's pump and so wake a GPUI
  task from its own thread; fixed in run 11 (the pump keeps a sender), 0 in
  4 322 runs after.
- The "no data is lost" property with 100 seeds × 800 steps: green, 30
  minutes.
- The full screenshot set (`scripts\ui-screens.ps1` without `-States`): 266
  screenshots, three sizes, both themes; findings in section 2.4.

Not obtained:

- The owner's own install, the server in the homelab (`server/README.md`,
  `docs/guide/06-sync.md`) and two devices; it is step 3 of section 3.
- A run of the server's own tests in CI: run 11 set up `server.yml` (SQLite
  and PostgreSQL in Docker, on the changes the deployment tests run on,
  D202); its first run on GitHub comes with the owner's next push.

### 2.4 Findings of run 11

None loses data or weakens protection; the run changed no code for them
(its plan, rule 1.2).

- **SFTP memory saw-tooth**: under the soak's SFTP load (files of 16 KB, 1 MB
  and 8 MB up and down, about 8 a second) private bytes swing between about
  198 and 265 MB, at times 310 MB, for 5–15 seconds; without SFTP (the russh
  soak) they stay within 195–208 MB. `crates/ssh/src/sftp.rs` keeps about
  2 MiB in flight per direction, so the source is elsewhere (russh's channel
  buffers, russh-sftp); a measurement of SFTP alone with a heap profiler is
  the next step.
- **A spike at the start of the load**: one sample of 498 MB 26 seconds into
  the russh soak, 309 MB at minutes 2–3 of the OpenSSH one; once per run.
- **`scripts\soak.ps1` reports growth that is not there**: it compares one
  sample at minute 15 with the last one and fails (exit 2) when the last
  meets a saw-tooth peak; it should compare the lowest values or medians of
  windows.
- **A warning in builds of the desktop alone**:
  `crates/core/src/service/sync_trust.rs:12` imports `Arc`, which only code
  under `cfg(any(test, feature = "test-hooks"))` uses; `cargo build -p
  shellpp-desktop` (release, dist, CI's "Build the app") warns, and the
  criterion's workspace clippy misses it because feature unification turns
  `test-hooks` on.
- **Screens**: on first run at 960x600 "Restore from sync…" beside the
  subtitle breaks it into two lines and lines up with the subtitle, not the
  title; flags (regional indicator pairs) are drawn as letters in the
  terminal; the CSV import's column list is cut mid-row without a sign of
  scrolling; `scripts\ui-screens.ps1` says 244 files for the default sizes,
  a run makes 266.

## 3. The path to release 1.0

Release 1.0 ships without team vaults (D197).

1. **Run 10** — interface and small things.
2. **The verification run** (run 11, this state) — the checks of 2.3: the
   long test, the flaky test search, the large property run, the full
   screenshot set at every size; no new features.
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

Both questions after run 10 are answered (D202): the server's tests run in
CI, and the superfluous copies after a sign-out before run 10 are accepted.
None is open; the run 11 report lists what to look at in the first run of
`server.yml` on GitHub.

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
8. **Decisions** go to `docs/decisions.md`. The next free number is D203;
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
