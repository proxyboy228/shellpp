# Roadmap

State of the project on 2026-10-25, after run 12. What each stage is comes from
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
| 7 | Sync server and client, several devices | personal vault done | run 10: a restore while a device is out of the account (D197, D198), backups that keep good copies (D199), "Also delete the local copy" deletes it (D197), changed KDF settings at sign-in (D200); run 11, the verification run: the checks of section 2.3; run 12: a sign-in the merge refuses as another account's revokes the session its login opened (D209); what is left is in section 2; team parts belong to stage 8 |
| 8 | Team vaults, roles, invitations, sharing | after release 1.0 (D197) | schema and push checks are ready (`docs/stage8-readiness.md`); section 4 |
| 9 | Polish | mostly done | workspaces, broadcast, Telnet (D87), serial (D88, D95), update check (D97) with a signed manifest (D170); releases need the owner's signing key (`docs/release.md`) |
| 10 | Later stages | partly, ahead of order | autocomplete (D142) and script generation (D144) are built; the mobile client (Android, D204; section 3.1), shared sessions, cloud provider import and Mosh are not started (Mosh comes with the mobile client; its toggle is hidden and a saved value kept, D204) |

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
- **Sessions after a refused sign-in** (D209): a login the merge refuses as
  another account's revokes its session; the other refusals after a login
  (a rebind's `rolled_back` or `integrity`, a merge's `forged_epoch` or
  `newer_epoch`) still leave theirs live, and the login's device row stays
  in that account's list.
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
has the numbers. Run 12 added the soak of the memory finding (D207).

- The 120-minute test against OpenSSH in Docker under all three loads at
  once: a clean 120 minutes, no errors (34 733 HTTP downloads, 34 723 through
  SOCKS, 54 620 SFTP files), no growth: the lowest private bytes of each ten
  minutes 197 MB at the start and 195 MB at the end, handles 744–752, threads
  80. Private bytes go up by 30–110 MB for a few seconds at a time; a
  five-minute sample that met one was the "320 MB peak at minute 15" of runs
  6 and 7.
- Run 12 (D207): 25 minutes of the SFTP load alone (11 494 files) keep
  private bytes at 177.7–180.5 MB and the Rust heap at 35–36 MB; under the
  whole load the swings are the heap's: tabs that flood output keep
  full-width rows for up to 2 seconds before the compact journal takes them
  (D85, D99), up to the 10 000 lines of the scrollback. Nothing grows.
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
  and PostgreSQL in Docker, D202), but GitHub Actions are off (D204); the
  same tests run in the full criterion.

### 2.4 Findings of run 12

The findings of run 11 are fixed or explained (D205–D208). What is left:

- **Flags on Windows** are drawn as two letters: Windows has no flag glyphs,
  and shipping a color font with them is the owner's choice (D206).
- **Flood rows** (D207): a tab that floods output holds up to 2 seconds of
  full-width rows, up to about 36 MB, before the journal takes them; seven
  such tabs swing the heap by 100–190 MB. Not a growth; a bound costs
  throughput (D99).
- **Dates outside Windows** are shown in UTC: the local zone is read on
  Windows only (`apps/desktop/src/dates.rs`).
- **CI** (D208): the causes of the red workflow are fixed, but GitHub Actions
  are off; `cargo deny` (licenses, advisories) runs only there. One session
  test (`a_lock_cancels_prompts_and_a_dropped_session_waits_for_unlock`) got
  `Cancelled` instead of `Locked` once on CI's Linux runner and never since.
## 3. The path to release 1.0

Release 1.0 ships without team vaults (D197), for Windows and Linux (D204);
macOS is decided in the release preparation run (there is no Mac to check
on). Checks are local: GitHub Actions are off (D204), and the full criterion
of `CLAUDE.md` scans for secrets too (D205).

1. **Run 10** — interface and small things.
2. **The verification run** (run 11) — the checks of 2.3: the long test, the
   flaky test search, the large property run, the full screenshot set at
   every size; no new features.
   **Run 12** (this state) — the findings of run 11, the terminal's emoji
   (D206), the memory finding explained (D207), the red CI's causes (D208),
   Linux in Docker: the client builds and every test passes (D208), the git history checked for private data
   (none to remove), and the Android study (section 3.1).
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

### 3.1 The mobile client (Android), a line of its own

**The mobile runs** (run 13; rough sizes, one run is one overnight run of one
agent). The first version is the owner's list (D211): SSH, host groups, the
Keychain, SFTP, known hosts, full sync, unlock by the master password; its
screens are `docs/mobile/ui-spec.md` (D214), its architecture
`docs/mobile/architecture.md` (D213).

1. **Run 13: the design run and the prototype** — the core for Android (the
   session layer and the terminal model as crates, D212), the architecture,
   the screen specification, and an APK that connects over SSH (quick
   connect) to a terminal with the extra keys (`apps/android`).
2. **M1, the vault on the phone** (1 run): the command dispatcher of the
   JNI layer over the core's service API; Android's platform services
   (`SecretStore` on the Keystore, the clipboard, the event sink); first run
   (a new vault), unlock and auto-lock; Hosts and groups (list, search,
   editor); connecting from a host.
3. **M2, the terminal for real** (1–2 runs): the binary screen snapshot,
   every gesture of UI spec 9 (the cursor by tap and by the row, selection
   with handles and the magnifier, walks with feedback in full-screen
   programs, Delete on the command line, Cut in nano), sticky Ctrl and Alt,
   the input method tested with Gboard and the Samsung Keyboard, the desktop's key mapping moved into
   `shellpp-terminal` and shared, the foreground service, reconnects on
   network changes, rotation and pinch.
4. **M3, sync** (1 run): `rustls-platform-verifier`'s initialization and its
   Kotlin part, "Restore from sync" and the connect wizard, the account
   screen, sync on demand and on resume; checked against the owner's server
   with a second device.
5. **M4, keys, known hosts, SFTP** (1 run): the Keychain (generate, import,
   share the public key), the known hosts screens and the host key dialogs,
   SFTP (browse, download, upload, the queue).
6. **M5, the first version** (1 run): Settings, fingerprint unlock (off by
   default), both themes at the S25's size, a signed APK on the owner's phone
   and the owner's acceptance.

So the first version is about 5–6 runs after run 13. After it, each about one
run: snippets (and running them on a host); the command history and
autocomplete (the capture and the list are already in `shellpp-terminal`);
port forwarding (local and dynamic while the app runs); Mosh (the largest:
no Mosh in the Rust core yet; its own study first, D204, D211); then
distribution (a release key of the owner's; Google Play only by the owner).

The owner's case (D204): now and then, away from home, SSH to one's own
servers, with full sync of the personal vault from day one; Android only
(a Samsung S25); Mosh later (D211). Run 12 studied the Rust core
(`docs/android-feasibility.md`): `shellpp-ssh`, `shellpp-sync-api` and `shellpp-private-file` build for `aarch64-linux-android` as they are; `shellpp-core` after one change (`crates/core/src/sync/tls.rs`: an added CA beside the platform verifier, which takes no extra roots on Android); the desktop app does not (clipboard, GPUI). Stripped test executables weigh about 7 MB (core) and 6 MB (SSH). The session layer and the terminal model are Rust without GPUI and the next crates to split out; Android needs the Keystore behind `SecretStore`, the platform verifier's JNI setup and a foreground service for long sessions. Next: the design run (the
interface stack, the binding between Kotlin and Rust, the split of the
session layer and the terminal model out of the desktop app), then the
mobile runs.

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

After run 12 (its report has the details):

1. **Flags on Windows**: ship a color font with flag glyphs (OFL, in a
   format Windows 10 draws in color), and which one; the run did not
   download one (D206).
2. **Flood rows** (D207): keep the 2-second compaction of a flooding tab, or
   bound its full rows (by count or by memory) and measure the throughput
   with `scripts\measure-terminal.ps1`.
3. **Licenses and advisories** (`cargo deny`, CI's job) are not in the local
   criterion: add them now that GitHub Actions are off? A new advisory would
   then stop main until it is handled.
4. **Dates on Linux** are shown in UTC (the local zone is read on Windows
   only): read the zone on Linux before release 1.0?
5. **Linux on a desktop**: the client was built and tested in Docker only;
   the window, Wayland or X11, the Secret Service keychain and the fonts
   want a check on a real Linux desktop before release 1.0.
6. **VS16 emoji take two cells** (D206), as the plan asked: readline counts
   one, so the cursor can be one column off on a command line that holds
   such an emoji. Keep it?

## 6. Technical debt

- **Vendored russh** (`third_party/russh`, D100, D191): two patches; remove each
  when upstream releases its fix (`third_party/russh/PATCHES.md`, `CLAUDE.md`).
- **Known limitations of secrets** (security.md): copies inside serde_json,
  reqwest, hyper and axum, and inside `totp-rs` on the server.
- **Sync engine leftovers**: section 2.2.
- **Terminal compatibility** (`docs/compatibility.md`): flags are drawn as
  two letters on Windows (no flag glyphs; a font is the owner's choice,
  D206); the blank cells after an emoji sequence stay (D92).
- **The core for Android** (`docs/android-feasibility.md`):
  `crates/core/src/sync/tls.rs` calls a verifier constructor that
  `rustls-platform-verifier` lacks on Android, so the "mobile-ready" check of
  `docs/architecture.md` 2.3 fails today; the fix belongs to the mobile
  runs.
- **Flood rows of the scrollback** (D207): a tab that floods output holds up
  to 2 seconds of full-width rows (up to about 36 MB) before the journal
  takes them; a bound by row count costs throughput (D99).
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
   workspaces and the secret scan of the whole history (gitleaks in Docker);
   clippy also runs without `--all-targets`, as a release build sees the
   code (D205). `scripts\verify-linux.ps1` repeats the build and the tests on
   Linux in Docker, outside the criterion.
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
8. **Decisions** go to `docs/decisions.md`. The next free number is D210;
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
