# Roadmap

State of the project on 2026-10-25, after run 14. What each stage is comes from
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
| 7 | Sync server and client, several devices | personal vault done | run 10: a restore while a device is out of the account (D197, D198), backups that keep good copies (D199), "Also delete the local copy" deletes it (D197), changed KDF settings at sign-in (D200); run 11, the verification run: the checks of section 2.3; run 12: a sign-in the merge refuses as another account's revokes the session its login opened (D209); run 13: every sign-in that fails after its login and before the binding revokes that login (a rebind keeps the device's own row, D216), the core refuses the recovery key in local mode and a new vault after a bound one keeps the weaker-settings check (D217); what is left is in section 2; team parts belong to stage 8 |
| 8 | Team vaults, roles, invitations, sharing | after release 1.0 (D197) | schema and push checks are ready (`docs/stage8-readiness.md`); section 4 |
| 9 | Polish | mostly done | workspaces, broadcast, Telnet (D87), serial (D88, D95), update check (D97) with a signed manifest (D170); releases need the owner's signing key (`docs/release.md`) |
| 10 | Later stages | partly, ahead of order | autocomplete (D142) and script generation (D144) are built; the mobile client (Android, section 3.1): designed and a prototype built in run 13 (D211–D215), the vault, sync, keys, known hosts and SFTP on the phone in run 14 (M1, M3, M4; D218, D219); shared sessions, cloud provider import and Mosh are not started (Mosh comes after the mobile client's first version, D211; its toggle is hidden and a saved value kept, D204) |

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
- **Sessions after a refused sign-in** (D216): every failure before the
  device is bound revokes the login's device (a rebind: the login's refresh
  token only, the device's own row stays active); the device row stays in
  the account's list marked revoked (the protocol deletes no row). A
  rebind's `rolled_back` before binding (a server whose chain does not
  reach the device's key) goes through that path but has no test scenario
  of its own; a failure with no answer (no network, a timeout) revokes
  nothing; the lock race fixed by the review has no test hook; a failed
  rebind leaves its row active even if another device had revoked it (the
  login itself restores a revoked row, as before).
- **The recovery key after Log out** (D217): Show answers `wrong_state`,
  Reissue `account_binding` (not unified; the interface hides both in local
  mode).
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

### 2.4 Findings of runs 12 and 13

The findings of run 12 are answered (D210): flags on Windows stay two
letters (a known limitation), the flood rows of D207 stay unbounded, dates
on Linux and macOS use the local zone, `cargo deny` runs in the local
criterion. What is left:

- **CI** (D208): GitHub Actions are off until the repository is published
  (D210). One session test
  (`a_lock_cancels_prompts_and_a_dropped_session_waits_for_unlock`) got
  `Cancelled` instead of `Locked` once on CI's Linux runner and never since.
- **A flaky server test fixed** (run 13): the account endpoints' body limit
  was asked over TCP and the 413 could arrive as a reset; it is asked in
  process now.
- **`tzset` on every date** (run 13, D210): the C library re-reads the zone
  for each date shown on Linux and macOS; cheap, not cached.
## 3. The path to release 1.0

Release 1.0 ships without team vaults (D197), for Windows and Linux (D204);
macOS is decided in the release preparation run (there is no Mac to check
on). Checks are local: GitHub Actions are off (D204), and the full criterion
of `CLAUDE.md` scans for secrets too (D205).

1. **Run 10** — interface and small things.
2. **The verification run** (run 11) — the checks of 2.3: the long test, the
   flaky test search, the large property run, the full screenshot set at
   every size; no new features.
   **Run 12** — the findings of run 11, the terminal's emoji
   (D206), the memory finding explained (D207), the red CI's causes (D208),
   Linux in Docker: the client builds and every test passes (D208), the git history checked for private data
   (none to remove), and the Android study (section 3.1).
   **Run 13** — the owner's answers (D210: `cargo deny` in the
   criterion, dates on Linux), the core for Android and the mobile design
   (D211–D214), the Android prototype (D215), the review leftovers of
   stage 7 (D216, D217).
   **Run 14** (this state) — the mobile runs M1, M3 and M4 (section 3.1;
   D218, D219).
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

1. **Run 13: the design run and the prototype** (done) — the core for
   Android (the session layer and the terminal model as crates, D212), the
   architecture (D213), the screen specification (D214), and an APK that
   connects over SSH (quick connect) to a terminal with the extra keys,
   cursor placement by tap and selection (`apps/android`, D215; checked on
   the emulator against OpenSSH; `docs/mobile/README.md` says how to put it
   on the phone).
2. **M1, the vault on the phone** (done, run 14): the command dispatcher of
   the JNI layer over the core's service API; Android's platform services
   (`SecretStore` on the Keystore, the clipboard, the event sink); first run
   (a new vault), unlock and auto-lock; fingerprint unlock behind a Keystore
   key that needs the fingerprint for every use, also after a lock (D218);
   Hosts and groups (list, search, editors, Recently deleted); connecting
   from a host; the app id `io.github.proxyboy228.shellpp`.
3. **M2, the terminal for real** (next, 1–2 runs, after the owner tried the
   prototype on the S25 and said how the cursor and the selection feel): the binary screen snapshot,
   every gesture of UI spec 9 (the cursor by tap and by the row, selection
   with handles and the magnifier, walks with feedback in full-screen
   programs, Delete on the command line, Cut in nano), sticky Ctrl and Alt,
   the input method tested with Gboard and the Samsung Keyboard, the desktop's key mapping moved into
   `shellpp-terminal` and shared, the foreground service, reconnects on
   network changes, rotation and pinch, snapshots only while the terminal
   is visible (the prototype's review, run 13).
4. **M3, sync** (done, run 14): `rustls-platform-verifier`'s initialization
   and its Kotlin part (vendored from the upstream tag), "Restore from sync"
   and the connect wizard (the three ways to trust a certificate, TOTP), the
   account screen, sync after unlock, on resume, after a change and on
   demand; checked on the emulator against the real server behind a TLS
   front with a second device, the TOTP QR code. Left: the system's trust
   with a real certificate (the owner's server) and the device name step of
   the wizard (the core has no way yet to rename a device after its first
   run; the phone's model name is used).
5. **M4, keys, known hosts, SFTP** (done, run 14): the Keychain (generate,
   import, share the public key), the known hosts screens and the host key
   dialogs, SFTP (browse, download, upload through the cache, the queue,
   the progress in the notification).
6. **M5, the first version** (1 run): the rest of Settings (appearance,
   terminal, about), the host editor's terminal theme (the identity's name
   shows read-only since run 14),
   both themes at the S25's size, a signed APK on the owner's phone and the
   owner's acceptance.

So the first version is about 2–3 runs after run 14 (M2, M5). After it, each about one
run: snippets (and running them on a host); the command history and
autocomplete (the capture and the list are already in `shellpp-terminal`);
port forwarding (local and dynamic while the app runs); Mosh (the largest:
no Mosh in the Rust core yet; its own study first, D204, D211); then
distribution (a release key of the owner's; Google Play only by the owner).

The owner's case (D204): now and then, away from home, SSH to one's own
servers, with full sync of the personal vault from day one; Android only
(a Samsung S25); Mosh later (D211). Android 17 asks an app for the local
network permission before it reaches a home server (D215). Run 12 studied the Rust core
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

After run 14 (its report has the details; run 13's two questions are
answered in D218):

1. **Release preparation** (D210): the project's contact address for
   `CODE_OF_CONDUCT.md`, whether to rewrite the history that holds the old
   one, Linux on a real desktop.

## 6. Technical debt

- **Vendored russh** (`third_party/russh`, D100, D191): two patches; remove each
  when upstream releases its fix (`third_party/russh/PATCHES.md`, `CLAUDE.md`).
- **Known limitations of secrets** (security.md): copies inside serde_json,
  reqwest, hyper and axum, and inside `totp-rs` on the server.
- **Sync engine leftovers**: section 2.2.
- **Terminal compatibility** (`docs/compatibility.md`): flags are drawn as
  two letters on Windows (no flag glyphs; a known limitation, D210); the
  blank cells after an emoji sequence stay (D92).
- **The core for Android** (D212): builds; the input mapping of the
  terminal is still the desktop's (`terminal/input.rs`, `mouse.rs`) with a
  copy in the Android crate until it moves into `shellpp-terminal` (M2);
  `shellpp-sessions` links `shellpp-term-local` on Android too (never
  opened there); the Kotlin part of `rustls-platform-verifier` is vendored
  (`apps/android/third_party`, D218): move it with the crate's version.
- **Wider visibility of the terminal model** (review of 3.3): the split made
  `Outbox`'s fields and some test helpers public; a `test-hooks` feature and
  narrower access are a cleanup for a later run.
- **Flood rows of the scrollback** (D207, D210): a tab that floods output
  holds up to 2 seconds of full-width rows (up to about 36 MB) before the
  journal takes them; kept unbounded by the owner's choice.
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
8. **Decisions** go to `docs/decisions.md`. The next free number is D220;
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
