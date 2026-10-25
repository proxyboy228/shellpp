# Mobile client architecture (Android)

Status: run 13, the design run (D213; reviewed in run 13, the findings are
in). The owner's description of the app is `docs/mobile/owner-notes.md`
(D211); the Rust core's state for Android is `docs/android-feasibility.md`;
the screens are `docs/mobile/ui-spec.md` (D214). Choices beyond the owner's
words are marked [decision].

## 1. The choice

**A Kotlin app (Jetpack Compose screens, a terminal drawn by a custom
Android `View`) over the shared Rust core, joined by a small hand-written JNI
layer in one Rust library (`libshellpp_android.so`) that has two kinds of
calls: JSON commands for the core's service API, and direct calls for
sessions, the terminal and file transfers.** [decision]

Why, against the criteria of the plan:

- **A fast terminal with exact cursor and selection** (the owner's two pains,
  D211). The terminal is a custom `View` drawing on a `Canvas` from a binary
  snapshot of the screen the Rust model writes in one pass: no serialization
  of cells into text formats. Touch, gestures, the magnifier, the selection
  handles and the floating toolbar are Android's own (`GestureDetector`,
  `Magnifier`, `ActionMode`), so they behave as everywhere else on the phone.
  Where the cursor is, which modes the program set, what a selection holds:
  the model answers, the same code as on the desktop (`shellpp-terminal`),
  and the cursor can be walked to a tapped cell with feedback from it
  (section 6.3).
- **Android's system services without a fight.** Keystore, the foreground
  service and its notification, the soft keyboard (`InputConnection`), the
  clipboard, the file picker (Storage Access Framework) and biometrics are
  Kotlin APIs; Kotlin calls them directly.
- **Little own code, low risk.** The boundary is a few dozen `extern
  "system"` functions. The core's service API already takes and returns
  `serde` DTOs (`crates/core/src/service/dto.rs`), so one `call(command,
  json) -> json` entry serves the vault's screens, and that dispatcher is
  plain Rust, tested on the computer. The session layer's types are not
  `serde` (`SessionView`, `PromptReply`): the Android crate keeps their JSON
  shapes by hand, in one module with tests. The `jni` crate (0.22) is already
  in the dependency graph through `rustls-platform-verifier`.
- **Builds on this machine without extra tools.** Gradle (the wrapper, with
  its checksum), the Android Gradle plugin, the NDK and `cargo`; a Gradle task
  runs `cargo build` with the NDK's clang (no `cargo-ndk`, no generator).

Rejected (one line each):

- **UniFFI**: a generator to run and generated Kotlin to keep in step, JNA
  per call, and terminal frames would still need a hand-made byte path; worth
  revisiting if the command set grows past what JSON serves well.
- **JNI only, typed by hand for every call**: every DTO converted by hand;
  slow to grow, easy to leak local references.
- **A command protocol only**: serializing every terminal frame and every
  output burst costs what the terminal cannot spare.
- **The interface in Rust** (GPUI's mobile work, Slint, Dioxus, egui): input
  methods, selection handles and system integration are young there, which
  is exactly where the owner's pains are.
- **Kotlin Multiplatform**: no iPhone app (D211); a second shared layer over
  a core already shared in Rust.

## 2. The boundary

```text
Kotlin (app)                               Rust (libshellpp_android.so)
──────────────────────────────────────     ─────────────────────────────────────
screens (Compose)          ── call(json) ─►  dispatcher ─► shellpp-core service API
                           ◄─ events ──────  EventSink (JSON events)
SessionService (FGS)       ── direct ─────►  shellpp-sessions: open, write, resize,
                           ◄─ events ──────  reply, close; session views (JSON)
TerminalView (Canvas, IME) ── direct ─────►  shellpp-terminal: snapshot, modes, cursor,
                                             input bytes, selection text
SFTP, key import           ── direct ─────►  shellpp-ssh SFTP to a file descriptor;
                                             key bytes into the core
SecretStore, Clipboard     ◄─ JNI calls ───  core platform traits (Kotlin objects)
Application.onCreate       ── init ───────►  rustls-platform-verifier (JNI env, Context)
```

- **Shared Rust** (as on the desktop): `shellpp-core` (vault, hosts,
  groups, keys, known hosts, sync client), `shellpp-ssh` (SSH, SFTP),
  `shellpp-sessions` (sessions, prompts, reconnect, the lock),
  `shellpp-terminal` (the terminal model), `shellpp-sync-api`,
  `shellpp-private-file`.
- **The Android crate** (`apps/android/native`, package `shellpp-android`,
  a `cdylib` built for `aarch64-linux-android` and `x86_64-linux-android`):
  the JNI exports, the command dispatcher, the JSON shapes of the session
  views, the screen snapshot, the input mapping until it moves into
  `shellpp-terminal`, and the Kotlin-backed platform services. A member of
  the client workspace: it builds and tests on the computer too.
- **Kotlin**: every screen, the terminal's drawing and input, the extra keys
  row, the foreground service, notifications, the Keystore, biometrics, the
  clipboard, files, network callbacks, the app's lifecycle.
- Nothing about the data changes: the same vault, records, sync protocol and
  rules as the desktop (`docs/data-model.md`, `docs/sync-protocol.md`); the
  phone is one more device of the account.

**The calls.** `call(command, input)` runs one command of the service API
with a JSON input and returns `{"ok": ...}` or `{"error": {"code",
"message"}}` (the core's error codes). Every call that can block runs on
Kotlin's worker threads (`Dispatchers.IO`), never on the main thread.
Sessions and terminals are addressed by handles: numbers that index a table
on the Rust side, never pointers; an unknown handle is an error.

**Errors and panics at the boundary.** Every export runs inside the `jni`
crate's `EnvUnowned::with_env`, which catches a panic and turns it into a
Java exception; the Android build profile keeps `panic = "unwind"` so that
it can. JNI local references live in the call's frame.

**Secrets at the boundary.** The master password, SSH passwords and
passphrases, prompt answers and the `secrets` of records cross the boundary.
[decision]

- Kotlin hands them over as `ByteArray` (UTF-8) or `CharArray`, never as
  `String`, and fills its arrays with zeros after the call; Rust copies them
  into `Zeroizing` buffers and `SecretString`.
- JSON that carries secrets (record saves) is parsed from a `Zeroizing`
  buffer; the dispatcher logs only the command's name and the error code,
  never inputs or outputs.
- The residual risk: text fields of Compose hold the typed password as a
  `String` until they are cleared, and the JVM may keep copies until garbage
  collection; the same as any Android password field.

## 3. Threads and lifecycle

- **Rust threads**: the session layer's Tokio runtime (2 workers,
  `shellpp-sessions`), one parser thread per terminal (`shellpp-terminal`),
  blocking pool threads for core calls, and the sync client's thread.
- **Calls into the JVM from Rust threads** (the platform services, and the
  platform verifier during TLS) attach the thread with the `jni` crate's
  `JavaVM::attach_current_thread`, which opens its own local frame. They use
  global references to the Kotlin objects and classes taken at
  initialization: `FindClass` on a native thread cannot see the app's
  classes. `rustls-platform-verifier` keeps its own `JavaVM` and `Context`
  from its initialization (section 5).
- **Wake-ups without callbacks**: Kotlin waits on Rust instead of Rust
  calling Kotlin. One pump thread per terminal blocks in `awaitWake(handle)`
  (the model's wake channel; it returns `false` when the terminal closed)
  and posts `invalidate()` to the view; one event thread blocks in
  `nextEvent(timeoutMs)` for session and core events (JSON) and hands them
  to the UI.
- **`Core::tick`** runs about once a second, as on the desktop: the
  auto-lock check and the 30-second clearing of a copied password. A
  handler on the main thread ticks while the app is in the foreground, the
  foreground service's thread while sessions keep the process alive, and the
  app ticks once on every resume (the core compares wall-clock time with the
  last input, so a frozen process locks at once when it wakes).
- **The foreground service** (`SessionService`) runs while any session is
  open: connecting, connected, dropped and waiting to reconnect, ended with
  its output kept, and while an SFTP transfer runs. It stops when the last
  one is closed. Android 12+ does not let a background app start it again,
  which is why a dropped session keeps it. [decision]
  - Type `specialUse` (an interactive remote shell has no better-fitting
    type; `dataSync` is time-limited on Android 15), with the
    `FOREGROUND_SERVICE` and `FOREGROUND_SERVICE_SPECIAL_USE` permissions and
    the `android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE` property ("Keeps the
    SSH terminal sessions the user opened").
  - The notification says how many sessions are open and offers "Disconnect
    all". The notification permission (Android 13+) is asked when the first
    session starts; when it is denied, the service still runs and the
    notification is not shown in the shade: the Sessions screen has
    "Disconnect all" too.
  - The service holds the Rust objects through the application object, so an
    activity that is recreated (rotation, theme, swiped away from recents)
    finds its sessions again.
- **Network changes**: a `ConnectivityManager.NetworkCallback` tells the
  session layer when the default network is lost or comes back; a dropped
  session follows the reconnect policy of UI 4.6, and a network that comes
  back starts a waiting attempt at once [decision].
- **Sync** runs after unlock, when the app comes to the foreground with the
  vault unlocked, after a local change (debounced as on the desktop), on
  "Sync now", and when a network comes back after a failed sync; there is no
  periodic cycle (the desktop's 60-second cycle has no counterpart), and
  failures back off as `docs/sync-protocol.md` 6.11 says while the app stays
  in the foreground [decision]. Every sync call runs off the main thread.
- **The lock**: the core's auto-lock as on the desktop, counted from the
  last touch in the app (UI spec 5). Live terminals keep running while
  locked; prompts are cancelled (`shellpp-sessions`).
- **Process death**: Android may kill the process when no service keeps it;
  the vault is closed, and the next start asks for the master password (or
  the fingerprint, section 4). Terminal contents do not survive it.

## 4. `SecretStore` on the Android Keystore; fingerprint unlock

The core's `SecretStore` (`crates/core/src/platform/mod.rs`) keeps the
32-byte device key per account (`device-key:<db_id>`), which the core uses
to unlock without the master password, and also reads during rotations,
rebinds, key changes and recoveries (docs/architecture.md 5.2;
docs/security.md, RR8). On Android the core takes it at launch and after a
lock (D218).

- **Default: nothing stored.** The owner unlocks with the master password
  (D211).
- **Fingerprint unlock** (Settings → Security, off by default; D211, D218)
  binds the device key to the fingerprint cryptographically:
  - The device key is encrypted with an RSA key pair generated in the
    `AndroidKeyStore` (2048 bits, OAEP with SHA-256; in StrongBox where the
    phone has one): its private key needs `BIOMETRIC_STRONG` for every use
    and becomes invalid when a fingerprint is added. The ciphertext is kept
    in the app's private storage, outside backups. The public key encrypts
    without a prompt, so turning the option on, a sign-in's new device key
    (D195) and the new key after a rotation need no prompt; only reading
    the key does [decision].
  - Reading: the app shows the `BiometricPrompt` with the private key's
    cipher; when it passes, the app decrypts the key and holds it for the
    one step that asked (an unlock, a sign-in, a recovery, a key change),
    then wipes it. Without a passed prompt the store answers
    "authentication required" (`SecretStoreError::AuthenticationRequired`):
    the core keeps the item and does not mark the option lost, and a
    rotation goes on without a `device` row for its new epoch.
  - Before the steps that may read the key (a sign-in again after a
    rotation made on another device, a recovery, a key change) the app asks
    for the fingerprint first. Cancelled: the step goes on with the master
    password and the option stays on; the new epoch has no `device` row, so
    the next unlock asks for the password once, and after it the app wraps
    a new device key [decision].
  - The master password is asked when the Keystore key became invalid (a
    fingerprint added: the option goes off and Settings says why), when the
    system locked biometrics out after failed attempts, and once every 7
    days since the last password unlock [decision] (D218).
  - Turning the option off deletes the item and the Keystore key pair.
- The 32 bytes cross JNI as a byte array that both sides wipe after use;
  they are never logged.

**The clipboard** (`Clipboard`): Android 10+ lets an app read the clipboard
only while it has focus. The Kotlin implementation returns an error from
`read_text` while the app has no focus, so the core keeps its guard and
clears a copied password when it can, instead of taking an empty read for
"cleared". Sensitive text is marked with `ClipDescription.EXTRA_IS_SENSITIVE`
(Android 13+) [decision].

## 5. TLS: `rustls-platform-verifier` on Android

- The sync server's certificate is checked by Android's own trust manager
  (the owner's server has a Let's Encrypt certificate, D204); an added CA is
  checked beside it (D212); a pinned certificate works as on the desktop.
- The crate needs its JNI initialization before the first TLS connection:
  `Application.onCreate` calls `Native.initTls(context)`, which calls
  `rustls_platform_verifier::android::init_with_env` with the JNI
  environment and the application context. Without it the crate panics, so
  every sync command of the dispatcher checks that the initialization ran
  and answers `unavailable` otherwise.
- The crate needs a small Kotlin component in the app. The crate
  `rustls-platform-verifier-android` on crates.io carries only a version
  marker, and the component's published Maven repository is on GitHub, which
  is not one of the allowed sources (Maven Central, Google Maven). The app
  vendors the component's source (MIT OR Apache-2.0) from the upstream
  repository's tag `v/0.7.1` (commit `252e2516`, component 0.2.0) as the
  Gradle library `apps/android/third_party/rustls-platform-verifier`,
  unchanged, with its line in THIRD_PARTY_NOTICES and the R8 rule
  `-keep class org.rustls.platformverifier.** { *; }` (D213, D218; run 14).
  Its manifest brings the library's network security config (cleartext only
  to the CRL hosts it lists), which the app's own Rust sockets do not use.
  Its trust manager reads `AndroidCAStore`, so it also trusts the CAs the
  user installed on the phone, unlike Android's default for apps (review of
  run 14; the desktop's platform verifier trusts the user's roots too).

## 6. The terminal: drawing and input

### 6.1 Drawing

- The model runs on its parser thread exactly as on the desktop (the
  journal, the limits, VS16 and emoji widths, D85, D206).
- `snapshot(handle)` copies the visible rows in one pass under the
  terminal's lock: per cell the character (a cell with combining marks or an
  emoji sequence also gets its whole text in a side table), the resolved
  foreground and background (the palette, dim, bold-bright and inverse rules
  applied in Rust), and flags; then the cursor, which rows wrap, the scroll
  position, and the modes the view needs (mouse reporting and its encoding,
  bracketed paste, application cursor keys, the alternate screen). The
  layout is fixed-size little-endian fields
  (`apps/android/native/src/screen.rs`). A reused direct buffer can replace
  the array when profiling asks for it.
- The view draws rows with `Canvas.drawText` per run of equal attributes, in
  the desktop's terminal font, JetBrains Mono (OFL, in the app's assets)
  [decision]: backgrounds, then text, then decorations, the cursor and the
  selection on top.
- Pinch zooms the font size (the cell size), which changes the terminal's
  columns and rows; the new size goes to the server when the gesture ends
  [decision].
- Accessibility: a `Canvas` view has none by itself; an
  `AccessibilityNodeProvider` that reads the screen's rows is part of the
  first version's terminal run [decision].

### 6.2 Input

- The view is its own input target (`onCreateInputConnection`) with the
  input type `TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_VISIBLE_PASSWORD |
  TYPE_TEXT_FLAG_NO_SUGGESTIONS`, which turns off predictions and
  composition on Gboard and the Samsung Keyboard alike, so each key commits
  as it is typed (`htop` keys, nano's Y/N and Tab completion need that)
  [decision].
- What the keyboard does anyway is sent at once: committed text as UTF-8;
  composing text that a keyboard still uses is sent as the difference to
  what was sent before (new characters, or Backspace for removed ones);
  `deleteSurroundingText` as Backspace; key events that keyboards send
  through `sendKeyEvent` (`KEYCODE_DEL`, Enter, the arrows, Tab) as those
  keys. Backspace sends what the host's setting says (UI 3.1; DEL by
  default). The run that builds it tests Gboard and the Samsung Keyboard,
  with the English and the Russian layout.
- Keys from the extra keys row and from a hardware keyboard go through the
  desktop's mapping (the bytes of Esc, Tab, the arrows in normal or
  application cursor mode, Ctrl and Alt combinations); that mapping moves
  from the desktop's `terminal/input.rs` and `mouse.rs` into
  `shellpp-terminal` as a pure function in the first version's terminal run,
  so both clients send the same bytes [decision]. Until then the Android
  crate has its own copy, tested against the desktop's sequences.
- Ctrl and Alt on the extra keys row are sticky (UI spec 9.2).
- Paste goes through the model's paste rules (bracketed paste, the
  multi-line confirmation of UI 4.2).

### 6.3 Cursor and selection

The owner's two pains (D211); the gestures are in `docs/mobile/ui-spec.md`
9.3–9.6.

- **A tap puts the cursor on a character** [decision]:
  - when the program asked for mouse reports, the tap is a click at that
    cell;
  - otherwise the terminal walks the cursor there with the arrow keys,
    **with feedback**: it sends one arrow, waits until the program has
    answered (the model's cursor moved, or a short time passed), reads the
    cursor from the model and decides the next step (a few arrows at a time
    once the first one moved it), until the cursor is on the tapped cell or
    stops moving. On the shell's command line it walks ←
    and → along the typed line; in a full-screen program (the alternate
    screen: nano without mouse support, vim) ↑ and ↓ first, then ← and →.
    Feedback makes tabs, wide characters and an editor's own soft wrap land
    right, where counting cells blindly would not.
  - A walk sends at most as many arrows as there are cells between the
    cursor and the tap and gives up after a step that did not move the
    cursor; a program where arrows do not move a cursor (a pager) gets one.
  - A fingertip covers several cells: after a tap, a handle under the cursor
    can be dragged with the magnifier, and its release walks again (UI spec
    9.3).
- **Selecting**: long-press selects a word with the model's word rules,
  with handles, a magnifier (Android 9+, API 28; below it no lens) and a
  floating toolbar (UI spec 9.4).
- **Deleting a selected piece in an editor**: a terminal cannot delete text
  on the server; only the program can, with its own keys. On the command
  line, the walk goes just past the selection's end and sends Backspaces.
  For nano (the owner's example), while nano's shortcut rows are on the
  screen, "Cut in nano" walks to the selection's start, sets the mark
  (Alt+A), walks just past its end and cuts (Ctrl+K): the same walks as a
  tap. No deleting key is sent unless every walk landed exactly. The app does
  not know which program runs, and the UI spec says where this stops (9.6).

### 6.4 Files: SFTP and key files

- **SFTP transfers** go between the server and a file in the app's cache
  (`cache/transfers`), through the SFTP code of the desktop
  (`shellpp_ssh::Sftp::download` and `upload`, with their `.part` files and
  size checks); Kotlin copies that file to the document the Storage Access
  Framework gave ("Save to…" for a download) or from the picked document
  before an upload, and deletes it after the transfer ends [decision, run 14:
  the SFTP code works on paths, and a document's file descriptor has no
  sibling for a `.part`]. No file content crosses JNI as arrays or JSON.
  The cost is the file's size in the cache for the time of one transfer.
  Transfers are queued and their ends handled by the application object
  on its own threads, whatever screen shows or whether the app locked
  meanwhile: a download is copied into its document before it shows as
  done, a failed copy shows as a failure, a cancelled download removes its
  empty document, a failed one keeps its cache file and document for Retry
  until its session closes, and the cache folder is emptied at every start
  (reviews of run 14).
- Transfers report progress and their end as events, can be cancelled, and
  keep the foreground service running (section 3).
- **Key files** for import are read by Kotlin from the picked document into
  a `ByteArray` that is wiped after the call; the core parses them as on the
  desktop.

## 7. Build and layout

- `apps/android`: a Gradle project (Kotlin DSL, the Gradle wrapper with
  `distributionSha256Sum`), module `app`; `apps/android/native`: the Rust
  crate. A Gradle task builds the library for `aarch64-linux-android` (the
  phone) and `x86_64-linux-android` (the emulator) with the NDK's clang and
  copies it to the APK's native libraries before packaging. The Android
  build profile is release-like with symbols stripped and `panic = "unwind"`.
- Minimum Android 8.0 (API 26, as the Rust check uses; the magnifier needs
  API 28 and is left out below it); target and compile SDK 37, the SDK
  installed here [decision].
- **Android 17's local network permission**: an app that targets API 37
  reaches no address of the local network (a server at home, the emulator's
  host at 10.0.2.2) without `ACCESS_LOCAL_NETWORK`; the connection only
  times out. The app declares it and asks for it before the first session
  to such an address (the prototype asks at the first session) (D215).
- Checks: `scripts\verify-android.ps1` (the debug APK, the Rust binding's
  tests on the computer, Kotlin unit tests; with `-Emulator` an end-to-end
  test on the emulator against OpenSSH in Docker). The desktop's full
  criterion keeps its "mobile-ready" step (D212).
