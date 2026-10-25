# Mobile client architecture (Android)

Status: run 13, the design run (D213). The owner's description of the app is
`docs/mobile/owner-notes.md` (D211); the Rust core's state for Android is
`docs/android-feasibility.md`. Choices beyond the owner's words are marked
[decision].

## 1. The choice

**A Kotlin app (Jetpack Compose screens, a terminal drawn by a custom
Android `View`) over the shared Rust core, joined by a small hand-written JNI
layer in one Rust library (`libshellpp_android.so`) that has two kinds of
calls: JSON commands for the core's service API, and direct calls for the
terminal's hot path.** [decision]

Why, against the criteria of the plan:

- **A fast terminal with exact cursor and selection** (the owner's two pains,
  D211). The terminal is a custom `View` drawing on a `Canvas` from a binary
  snapshot of the screen the Rust model writes into a reused direct
  `ByteBuffer`: no per-frame allocation, no serialization of cells. Touch,
  gestures, the magnifier, the selection handles and the floating toolbar
  are Android's own (`GestureDetector`, `Magnifier`, `ActionMode`), so they
  behave as on every other app of the phone. Where a cell is, what the
  cursor and the mouse mode are, what a selection holds: the model answers,
  the same code as on the desktop (`shellpp-terminal`).
- **Android's system services without a fight.** Keystore, the foreground
  service and its notification, the soft keyboard (`InputConnection`), the
  clipboard and biometrics are Kotlin APIs; Kotlin calls them directly.
- **Little own code, low risk.** The boundary is a handful of `extern
  "system"` functions. The core's service API already takes and returns
  `serde` DTOs (the desktop's former IPC; `crates/core/src/service/dto.rs`),
  so one `call(command, json) -> json` entry serves every screen but the
  terminal, and the dispatcher is plain Rust, tested on the computer. The
  `jni` crate (0.22) is already in the dependency graph through
  `rustls-platform-verifier`.
- **Builds on this machine without extra tools.** Gradle (the wrapper), the
  Android Gradle plugin, the NDK and `cargo`; the Rust library is built by a
  Gradle task that runs `cargo build` with the NDK's clang (no `cargo-ndk`,
  no generator).

Rejected (one line each):

- **UniFFI**: a generator to run and generated Kotlin to keep in step, JNA
  per call, and terminal frames would still need a hand-made byte path; worth
  revisiting if the command set grows past what JSON serves well.
- **JNI only, typed by hand for every call**: every DTO converted by hand;
  slow to grow, easy to leak local references.
- **A command protocol only**: serializing every terminal frame and every
  output burst costs what the terminal cannot spare.
- **The interface in Rust** (GPUI's mobile work, Slint, Dioxus, egui): input
  methods, selection handles, accessibility and system integration are
  young there, which is exactly where the owner's pains are.
- **Kotlin Multiplatform**: no iPhone app (D211); a second shared layer over
  a core already shared in Rust.

## 2. The boundary

```text
Kotlin (app)                               Rust (libshellpp_android.so)
──────────────────────────────────────     ─────────────────────────────────────
screens (Compose)          ── call(json) ─►  dispatcher ─► shellpp-core service API
                           ◄─ events ──────  EventSink (JSON events)
TerminalView (Canvas, IME) ── direct ─────►  shellpp-terminal: feed, snapshot, select,
                                             cursor, modes, input bytes
SessionService (FGS)       ── call(json) ─►  shellpp-sessions: open, write, resize,
                                             reply, close; SessionEvent (JSON)
SecretStore, Clipboard     ◄─ JNI calls ───  core platform traits (Kotlin objects)
Application.onCreate       ── init ───────►  rustls-platform-verifier (JNI env, Context)
```

- **Shared Rust** (unchanged from the desktop): `shellpp-core` (vault,
  hosts, groups, keys, known hosts, sync client), `shellpp-ssh` (SSH, SFTP),
  `shellpp-sessions` (sessions, prompts, reconnect, the lock),
  `shellpp-terminal` (the terminal model), `shellpp-sync-api`,
  `shellpp-private-file`.
- **The Android crate** (`apps/android/native`, package `shellpp-android`,
  a `cdylib` built for `aarch64-linux-android` and `x86_64-linux-android`):
  the JNI exports, the command dispatcher, the screen snapshot format, the
  Kotlin-backed platform services. It is a member of the client workspace
  and builds and tests on the computer too; only the exports are
  Android-specific in use, not in code.
- **Kotlin**: every screen, the terminal's drawing and input, the extra keys
  row, the foreground service, notifications, the Keystore, biometrics, the
  clipboard, the network callbacks, the app's lifecycle.
- Nothing about the data changes: the same vault, records, sync protocol and
  rules as the desktop (`docs/data-model.md`, `docs/sync-protocol.md`); the
  phone is one more device of the account.

**The calls.** `call(command: String, input: String): String` runs one
command of the service API with a JSON input and returns `{"ok": ...}` or
`{"error": {"code", "message"}}` (the core's error codes). Blocking: Kotlin
calls it on its own worker threads (coroutines on `Dispatchers.IO`), never
on the main thread. Terminal calls take a handle (a `long` that indexes a
table on the Rust side, never a raw pointer) and are short: they take the
terminal's lock for one copy.

**Errors at the boundary.** No Rust panic crosses JNI: every export runs
under `catch_unwind` and turns a panic into an error result (and a log
line). A handle that is not in the table is an error, not undefined
behaviour. JNI local references are scoped per call (`with_local_frame`).

## 3. Threads and lifecycle

- **Rust threads**: the session layer's Tokio runtime (2 workers,
  `shellpp-sessions`), one parser thread per terminal (`shellpp-terminal`),
  blocking pool threads for core calls. Rust never calls into the JVM from
  its own threads except for the platform services, which attach the thread
  once (`JavaVM::attach_current_thread_as_daemon`).
- **Wake-ups without callbacks**: Kotlin waits on Rust instead of Rust
  calling Kotlin. One "pump" thread per terminal blocks in
  `awaitWake(handle, timeoutMs)` (the model's wake channel) and posts
  `invalidate()` to the view; one event thread blocks in
  `nextEvent(timeoutMs)` for session and core events (JSON) and hands them
  to the UI. A closed handle ends its pump.
- **A foreground service keeps live sessions** (`SessionService`): started
  with the first live SSH session, stopped with the last; its notification
  says how many sessions are open and has "Disconnect all". Type
  `specialUse` (an interactive remote shell has no better-fitting type;
  `dataSync` is time-limited on Android 15) [decision]. The notification
  permission (Android 13+) is asked when the first session starts
  [decision]. The service holds the Rust objects through the application
  object, so an activity that is destroyed and recreated (rotation, theme,
  the app swiped away from recents while sessions live) finds its sessions
  again.
- **Network changes**: a `ConnectivityManager.NetworkCallback` tells the
  session layer when the default network is lost or changes; a dropped
  session follows the reconnect policy of UI 4.6 (`shellpp-sessions`), and a
  new network makes a waiting attempt start at once [decision]. Keepalive is
  the host's setting, as on the desktop.
- **Sync runs on demand and when the app comes to the foreground**
  (`ProcessLifecycleOwner` start), and after a local change, debounced as on
  the desktop; no timer of its own and no `WorkManager` job in the first
  version [decision]. Every sync call runs off the main thread (the sync
  client is blocking, `crates/core/src/sync/http.rs`).
- **The lock**: the core's auto-lock as on the desktop; leaving the app does
  not lock by itself, the auto-lock time does [decision: the desktop's
  rule]. Live terminals keep running while locked, prompts are cancelled
  (`shellpp-sessions`).
- **Process death**: Android may kill the process while it is in the
  background without a foreground service; the vault is closed then, and the
  next start asks for the master password. Terminal contents are not kept
  across process death.

## 4. `SecretStore` on the Android Keystore

The core's `SecretStore` (`crates/core/src/platform/mod.rs`) keeps the
32-byte device key per account (`device-key:<db_id>`), which lets a device
unlock without the master password ("remember on this device",
docs/architecture.md 5.2).

- **Default: not used.** The owner unlocks with the master password (D211):
  nothing is stored, and `availability()` says the store is there but the
  app does not offer "remember" except through fingerprint unlock.
- **Fingerprint unlock** (an option, off by default, D211): the device key
  is encrypted with an AES-256-GCM key generated in the `AndroidKeyStore`
  (never exported; `setUserAuthenticationRequired(true)` with
  `BIOMETRIC_STRONG`, invalidated when a new fingerprint is enrolled) and
  the ciphertext with its IV is kept in the app's private storage. `get`
  needs a `Cipher` that a `BiometricPrompt` authorized: the app shows the
  prompt first and calls the core's unlock in its callback, and the Kotlin
  store uses that cipher for that one call. A failed or cancelled prompt
  falls back to the master password. An invalidated key (new fingerprint)
  deletes the entry and asks for the password once.
- The 32 bytes cross JNI as a byte array that both sides wipe after use;
  they are never logged, never put in a `String`.

## 5. TLS: `rustls-platform-verifier` on Android

- The sync server's certificate is checked by Android's own trust manager
  (the owner's server has a Let's Encrypt certificate, D204). Run 13 made the
  core build there: an added CA is checked beside the platform verifier
  (D212); a pinned certificate works as on the desktop.
- The crate needs its JNI initialization before the first TLS connection:
  `Application.onCreate` calls `nativeInit(context)`, which calls
  `rustls_platform_verifier::android::init_with_env` with the JNI
  environment and the application context. Without it the crate panics, so
  every sync entry checks that the initialization ran and returns an error
  otherwise.
- The crate also needs its small Kotlin component in the app's build (it
  ships with the crate as a Maven artifact; how version 0.7.1 provides it is
  checked when sync lands on the phone).

## 6. The terminal: drawing and input

### 6.1 Drawing

- The model runs on its parser thread exactly as on the desktop (the
  journal, the limits, VS16 and emoji widths, D85, D206).
- `snapshot(handle, buffer)` copies the visible rows into a direct
  `ByteBuffer` the view owns and reuses: per cell the character (a code
  point; a cell with combining marks or an emoji sequence points into a
  side table of strings), the resolved foreground and background (RGB from
  the palette, with the theme's dim, bold-bright and inverse rules applied in
  Rust), and flags (bold, italic, underline kinds, strikeout, wide, hidden);
  then the cursor (position, shape, visible), the selection's cells, the
  scroll position, and the modes the view needs (mouse reporting, bracketed
  paste, application cursor keys, the alternate screen).
- The view draws rows with `Canvas.drawText` per run of equal attributes,
  in the desktop's terminal font, JetBrains Mono (OFL, in the app's assets) [decision]; a
  run's background first, then text, then decorations, the cursor and the
  selection on top. Only rows that changed are measured again.
- Pinch zooms the font size (the cell size), which changes the terminal's
  columns and rows; the new size goes to the server (resize) when the
  gesture ends [decision].

### 6.2 Input

- The view is its own input target (`onCreateInputConnection`) with an
  input type that turns off suggestions and auto-correction; committed text
  becomes UTF-8 bytes, deletions become DEL, the Enter action becomes CR.
  The IME's composing text is shown at the cursor and sent when committed.
- Keys from the extra keys row and from a hardware keyboard go through the
  same mapping as the desktop's keys (the bytes of Esc, Tab, the arrows in
  normal or application cursor mode, Ctrl and Alt combinations); that
  mapping moves from the desktop's `terminal/input.rs` into
  `shellpp-terminal` as a pure function when the first version is built, so
  both clients send the same bytes [decision].
- Ctrl and Alt on the extra keys row are sticky: one tap applies to the
  next key, a double tap locks until tapped again [decision].
- Paste goes through the model's paste rules (bracketed paste, the
  multi-line confirmation of UI 4.2).

### 6.3 Cursor and selection

The owner's two pains (D211); the details are in `docs/mobile/ui-spec.md`.

- **Moving the cursor**: a tap on the line being typed moves the cursor
  there. When the program asked for mouse reports (nano with mouse support,
  `htop`, vim with `mouse=a`), the tap is a mouse click at that cell;
  otherwise the view sends the arrow keys that walk from the cursor to the
  tapped cell along the typed line (the model knows where the cursor is and
  where the line wraps) [decision]. A horizontal drag on the extra keys row
  moves the cursor one cell per step, with a magnifier above the finger
  [decision].
- **Selecting**: long-press selects a word with the model's word rules,
  with handles to drag and a floating toolbar (Copy, Paste, Select line,
  Select screen) [decision]; a magnifier shows under the finger while a
  handle moves.
- **Deleting a selected piece in an editor**: a terminal cannot delete text
  on the server by itself, so the toolbar offers what the program can do:
  on a shell's typed line, "Delete" moves the cursor to the selection's end
  and sends the backspaces; in a full-screen program the selection is for
  copying, and the extra keys row offers the editor's own keys (for nano:
  set mark, cut) to cut a block the cursor walked over [decision; the limits
  are in the UI spec].

## 7. Build and layout

- `apps/android`: a Gradle project (Kotlin DSL, the Gradle wrapper with
  `distributionSha256Sum`), module `app`; `apps/android/native`: the Rust
  crate. A Gradle task builds the library for `aarch64-linux-android` (the
  phone) and `x86_64-linux-android` (the emulator) with the NDK's clang and
  copies it to `jniLibs` before the APK is packaged.
- Minimum Android 8.0 (API 26, as the Rust check uses); target and compile
  SDK 37, the SDK installed here [decision].
- Checks: `scripts\verify-android.ps1` (the debug APK, the Rust binding's
  tests on the computer, Kotlin unit tests; with `-Emulator` an end-to-end
  test on the emulator against OpenSSH in Docker). The desktop's full
  criterion keeps its "mobile-ready" step (D212).
