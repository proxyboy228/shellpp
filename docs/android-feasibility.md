# Android feasibility of the Rust core

What of the client's Rust code builds for Android, what does not and why,
where the core's boundary lies, and the ways to join the core to an Android
interface. A study of run 12 (decision D204: the mobile client is Android;
the owner's phone is a Samsung S25). No app code, no choice of the interface
stack: that is the work of the design run. Nothing in the core was changed
for this study.

## 1. The case

From the owner (D204): now and then, away from home, open SSH to one's own
servers and do what is needed; full sync of the personal vault is wanted
from day one. Mosh comes with the mobile client. No iPhone app.

## 2. What was built

On Windows, with the Android NDK 30.0.16248370 of the owner's Android Studio
(its clang for `aarch64-linux-android26` called directly, `CC`, `AR` and the
linker set for this shell only; no SDK packages or licenses touched, no
emulator), Rust 1.99.1 and the targets `aarch64-linux-android` (the owner's
Samsung S25) and `x86_64-linux-android` (the emulator image) added:

| Crate | `cargo check --target aarch64-linux-android` |
|---|---|
| `shellpp-ssh` (russh with `ring`, russh-sftp, Tokio) | builds |
| `shellpp-sync-api` | builds |
| `shellpp-private-file` | builds (Unix permissions) |
| `shellpp-term-local` | builds (not wanted on a phone) |
| `shellpp-core` | **one error**: `crates/core/src/sync/tls.rs:204` calls `rustls_platform_verifier::Verifier::new_with_extra_roots`, which that crate does not offer on Android (on Android the verifier asks the platform and takes no extra roots). With that one call bypassed (a local experiment, not committed) the rest builds: the bundled SQLite and `ring` compile with the NDK's clang. |
| `shellpp-desktop` | does not build, as expected: `arboard` (the clipboard) has no Android support, and GPUI's platform layer comes after it |

The API level is 26 (Android 8.0), a reasonable floor far below the S25's
Android 15; the NDK offers 21 to 37, CI's check uses 24, and nothing here
needs more than 24. The design run sets the app's minimum.

CI's Linux job runs the same check for `shellpp-core` and `shellpp-ssh`
(`docs/architecture.md`, 2.3); in the runs read for D208 its test steps
failed first, so it never got there. The `tls.rs` call came in with the
sync client's TLS trust (739c567).

**Size.** Release builds (`opt-level` 3, no LTO) for `aarch64`, stripped:
the executable of the core's test `core_flows` (the core and its
dependencies, with the test harness) is 7.3 MB, that of `shellpp-ssh`'s
test `shell` (SSH and SFTP, Tokio, and the russh test server) 5.7 MB. A
shared library of both, with LTO and `opt-level = "s"`, would land near
or under 10 MB.

**The fix for `tls.rs`**, for the mobile runs: on Android, the "system's
trust plus an added CA" choice (`TrustMode::Ca`, D187) builds its verifier
from the added CA with `WebPkiServerVerifier` (as `SystemRoots::Fixed`
already does) and asks the platform verifier only when that fails; the
plain "system's trust" keeps the platform verifier. Note that
`TrustVerifier::new` builds the platform verifier for every mode, pinned
ones included, and a changed pin asks it whether the system trusts the new
certificate: on Android all of that needs the crate's initialization
(section 3.3).

## 3. The crates and their ties to the desktop

| Crate | Role | For Android |
|---|---|---|
| `shellpp-core` | storage (SQLite, encrypted records), keys and their formats, the lock, sync engine and HTTP client, import and export | builds after one change (section 2); the platform services come in through traits already |
| `shellpp-ssh` | SSH (russh), SFTP (russh-sftp), forwarding, jump hosts, proxies, agents | builds (section 2); its system agent part is for desktops |
| `shellpp-sync-api` | the wire types of the sync protocol | pure Rust, builds |
| `shellpp-private-file` | files only the user can read (`0600` on Unix, an ACL on Windows) | builds: Android is Unix |
| `shellpp-term-local` | the local terminal (ConPTY, `portable-pty`) and serial ports (`serialport`) | desktop only; not part of a phone client |
| `shellpp-desktop` | the GPUI app | desktop only |

### 3.1 Already behind interfaces

`shellpp-core` takes its platform services as traits
(`crates/core/src/platform/mod.rs`); the desktop implements those it uses:

- `SecretStore`: the device key of "Remember on this device"
  (`apps/desktop/src/platform/keychain.rs`: `keyring` with the Windows
  Credential Manager, the macOS Keychain or the Secret Service on Linux). On
  Android it is the Android Keystore: a key that never leaves the secure
  hardware wraps the 32 bytes; it needs JNI or the interface side.
- `Clipboard`, `ExternalLauncher` (`open_url`; the desktop passes a
  `NoLauncher` and opens links through GPUI), `EventSink` (the core's events
  to the interface), `Clock` (the core's own `SystemClock` serves every
  platform).
- `PlatformClass::Mobile` already exists: a device of this class writes the
  KDF preset for phones (D189), and the server's tests already run a phone
  (`server/tests/sign_in_kdf.rs`).
- The data directory is a parameter (`CoreConfig::data_dir`): on Android the
  app's private files directory (`Context.getFilesDir()`), which only the
  app can read.

### 3.2 Still in the desktop app, to move behind interfaces

- **The session layer** (`apps/desktop/src/sessions/`): connecting, the
  reconnect policy, the host key and password prompts, the lock that cancels
  prompts, keepalive, forwarding rules that start on launch. It is plain
  Rust on Tokio with GPUI only at its edges (`sessions/ui.rs`): the natural
  next crate of the core, shared by both clients.
- **The terminal emulator** (`apps/desktop/src/terminal/`): alacritty's
  parser and grid, the scrollback journal (D85), the limits of
  `terminal/limits.rs`, search and selection are Rust without GPUI
  (`model.rs`, `journal.rs`, `limits.rs`, `search.rs`, `selection.rs`,
  `paste.rs`, `colors.rs`, `contrast.rs`). GPUI is in the view and input
  (`render.rs`, `view.rs`, `input.rs`, `mouse.rs`) and in a few helpers that
  take its `App` or `Context` (`autocomplete.rs`, `fonts.rs`,
  `history_capture.rs`, `settings.rs`, `highlight.rs`, `mod.rs`), which the
  split has to free from it. Splitting the model from the view gives Android
  the same terminal behaviour (VS16 and emoji sequences, D206; the memory
  bounds of D85).
- **Dates**: local time comes from the Windows API (`platform/local_time.rs`),
  on Linux and macOS from the C library (`TZ`, `/etc/localtime`; run 13,
  D210); a phone client takes the zone and the format from Android.
- **FIDO2** (`platform/security_key*.rs`: the system WebAuthn API on
  Windows, `ctap-hid-fido2` over HID behind the optional `fido2-hid`
  feature elsewhere): desktops only. Android reaches security keys over NFC or USB through its own
  FIDO2 API (Google Play services); a phone client would sign through it.
- **The system SSH agent** (`crates/ssh/src/agent.rs`: `SSH_AUTH_SOCK`, the
  Windows OpenSSH pipe): no such agent on Android; the built-in agent of the
  Keychain (D60) is what a phone uses.
- **Update check and the release manifest** (D97, D170): app stores update
  apps; not needed on Android.
- **Local terminal and serial** (`shellpp-term-local`): not part of a phone
  client (Termux exists for a local shell).

### 3.3 Network and background assumptions

- **TLS trust of the sync server.** `crates/core/src/sync/tls.rs` uses
  `rustls-platform-verifier` for "the system's trust" (the owner's server is
  Let's Encrypt behind Caddy, D204). On Android that crate verifies through
  the platform's `X509TrustManager` over JNI: the app must call its Android
  initialization (in 0.7.1 `rustls_platform_verifier::android::init_with_env`,
  `init_with_runtime` or `init_with_refs`) with the JNI environment and the
  application `Context` before the first sync, and ship the small Kotlin part
  the crate carries as a Maven artifact. Today every trust mode needs it,
  pinned certificates and a private CA (D93, D187) included: the core builds
  the platform verifier for each, and a changed pin asks it about the new
  certificate. Without the initialization the crate panics.
- **Long-lived connections.** SSH sessions, keepalive tasks
  (`keepalive@openssh.com`, `crates/ssh/src/client.rs`), forwarding listeners
  and the sync cycle are Tokio tasks that expect to keep running. Android
  stops a backgrounded app's network after a short while and kills its
  process when memory is short: an SSH session that must survive the screen
  turning off needs a foreground service with its notification; sync runs
  on demand and on resume (or through WorkManager), not as a timer of its
  own. The reconnect policy of the session layer (the countdown and the retries of UI 4.6) fits a phone's
  changing networks once it learns about network changes from the app.
- **Blocking HTTP.** The sync client is `reqwest::blocking` on a thread the
  caller provides (`crates/core/src/sync/http.rs`): on Android every sync
  call has to stay off the main thread, as on the desktop.
- **Local listeners.** Local and dynamic forwarding bind ports on the
  phone's loopback; they work while the app runs.

## 4. The core's boundary

The boundary that the code suggests: everything up to the session layer and
the terminal model is shared Rust (`shellpp-core`, `shellpp-ssh`,
`shellpp-sync-api`, `shellpp-private-file`, and two crates to split out of
the desktop app: sessions and the terminal model); each client owns its
interface, its platform services (`SecretStore` and the others), the
terminal's drawing and input, and the OS's lifecycle (foreground service,
network changes, the Keystore).
The boundary's API is the core's service API (`crates/core/src/service/`: plain calls with
DTOs, events through `EventSink`) plus the session layer once it moves out of the desktop
app; nothing in it needs GPUI.

## 5. Ways to join the Rust core to an Android interface

Only the options and their trade-offs; the choice is the design run's.

| Way | How | For | Against |
|---|---|---|---|
| UniFFI (Mozilla) | a Rust crate with `#[uniffi::export]` (or a UDL file); generated Kotlin bindings over a C ABI, loaded with JNA | mature, used in Firefox and others; async functions, callbacks (traits implemented in Kotlin, e.g. `SecretStore`), records and errors map to Kotlin types; also generates Swift | JNA adds overhead per call and a dependency; generated code to keep in step; large byte streams (terminal output) want batching |
| `jni` crate by hand | `extern "system"` functions named `Java_…`, called from Kotlin `external fun` | no generator, full control, no JNA; `rustls-platform-verifier` already needs the JNI environment | every type converted by hand, unsafe code at the boundary, easy to leak local references; slow to grow |
| A command protocol | one entry point: Kotlin sends serialized commands (JSON, CBOR or Protobuf), Rust answers and pushes events on a channel | the boundary is a few functions; commands can mirror the core's service API; easy to log and to test | serialization cost on every call (terminal output, SFTP); a second, hand-kept API description |
| Rust interface (Compose not used) | the interface in Rust too: GPUI's mobile support, Slint, Dioxus or egui on Android, the app built with `cargo-apk` or `xbuild` | one language; the desktop's terminal view could be shared | the toolkits' Android support is young (input methods, accessibility, system integration); still needs JNI for the Keystore and the foreground service |
| Kotlin Multiplatform beside Rust | a shared Kotlin layer over the Rust core (through UniFFI or JNI) | if an iOS client ever comes back | more layers; the core is already shared in Rust |

## 6. After run 13

- `crates/core/src/sync/tls.rs` checks an added CA beside the platform
  verifier on Android (D212): `shellpp-core` builds for
  `aarch64-linux-android`.
- The session layer and the terminal model are crates of their own,
  `shellpp-sessions` and `shellpp-terminal` (D212), without GPUI; both build
  for `aarch64-linux-android`. `shellpp-sessions` links `shellpp-term-local`
  (local terminals, serial ports), which builds there too; a phone client
  does not open those sessions.
- `scripts\verify.ps1` runs the "mobile-ready" check of every crate a phone
  client shares (`shellpp-core`, `shellpp-ssh`, `shellpp-sync-api`,
  `shellpp-private-file`, `shellpp-sessions`, `shellpp-terminal`) with the
  NDK's clang; without an NDK it says SKIPPED.
- What stays in the desktop app: the drawing and input of the terminal
  (render, view, input, mouse, fonts), the terminal themes (the model takes
  plain RGB), the GPUI glue of the sessions (`sessions/ui.rs`), the history
  recorder and the suggestions registry, and the platform services.

## 7. Summary (run 12)

- **Builds**: `shellpp-ssh`, `shellpp-sync-api`, `shellpp-private-file` as they are;
  `shellpp-core` after one change in `crates/core/src/sync/tls.rs` (the extra CA with the
  platform verifier). The C parts (SQLite, `ring`) compile with the NDK.
- **Does not**: the desktop app (clipboard, GPUI); the local terminal and serial are not
  wanted on a phone.
- **To move out of the desktop app** for a phone client: the session layer and the
  terminal model (Rust without GPUI already); the core's platform traits get Android
  implementations (the Keystore for `SecretStore`).
- **Android's own constraints**: `rustls-platform-verifier` needs its JNI initialization
  and its Kotlin part; long SSH sessions need a foreground service; sync runs on demand.
- **Size**: about 7 MB for the core and 6 MB for SSH as test executables, stripped.
- **Binding**: UniFFI, plain JNI, a command protocol, or a Rust UI toolkit; the design run
  chooses (section 5).
