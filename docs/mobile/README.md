# Shell++ for Android

The Android app is being designed and built in the mobile runs
(`docs/ROADMAP.md`, 3.1). Its documents:

- `owner-notes.md`: the owner's description of the app (D211);
- `architecture.md`: the stack, the boundary with the Rust core, threads,
  secrets, TLS, the terminal (D213);
- `ui-spec.md`: the screens of the first version (D214).

Run 13 built a **prototype** (`apps/android`): a quick connect over SSH to
a terminal with the extra keys, cursor placement by tap and selection.
Run 14 (M1) gave it **the vault on the phone**: the first run makes a new
vault with the master password, which unlocks it (fingerprint unlock is an
option in Settings, D218); hosts and groups with search and editors; a
session from a host; the data lasts across restarts. Sync comes with M3.

The app's id is `io.github.proxyboy228.shellpp` (D218); the prototype's
was `dev.shellpp.android` (the code's package keeps that name).

## Building

On Windows, with Android Studio installed (its SDK with platform 37,
build-tools 36.0.0 and NDK 30.0.16248370; its JDK), Rust with the targets
`aarch64-linux-android` and `x86_64-linux-android`
(`rustup target add aarch64-linux-android x86_64-linux-android`):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\verify-android.ps1
```

This runs the Rust binding's tests on the computer, builds the debug APK
(`apps/android/app/build/outputs/apk/debug/app-debug.apk`, for the phone and
the emulator) and runs the Kotlin unit tests. With `-Emulator` it also runs
the end-to-end tests on the emulator against OpenSSH in Docker, fingerprint
unlock included (the emulator gets the PIN 1111 and two test fingerprints,
removed at the end). The Gradle build runs `cargo build --profile android`
for both targets itself.

## Installing the debug APK on a phone

For a Samsung S25 (Android 15); other phones are alike.

1. **Developer options**: Settings → About phone → Software information →
   tap **Build number** seven times, then enter the phone's PIN.
2. **USB debugging**: Settings → Developer options → turn on
   **USB debugging**.
3. Connect the phone to the computer with a USB cable. On the phone, allow
   USB debugging for this computer when it asks ("Always allow from this
   computer" if you trust it).
4. Check that the computer sees the phone:

   ```powershell
   & "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" devices
   ```

   The phone shows as `device` (not `unauthorized`).
5. Install the APK (`-r` replaces an earlier install):

   ```powershell
   & "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" install -r apps\android\app\build\outputs\apk\debug\app-debug.apk
   ```

6. Open **Shell++** on the phone. The first SSH session asks to allow
   notifications: the notification keeps sessions alive while the app is in
   the background ("Disconnect all" is there).

A debug APK is signed with the computer's debug key: it cannot be published,
and a later build signed with another key must be installed after removing
this one. To remove it: `adb uninstall io.github.proxyboy228.shellpp`, or
long-press the app's icon → Uninstall. The run 13 prototype is a separate
app (`dev.shellpp.android`): remove it with `adb uninstall dev.shellpp.android`.

## A new app id or signing key means a new install

Android treats an app with another id, or the same id signed with another
key, as another app: it is installed anew and **the data on the phone does
not carry over** (the vault in the app's private storage goes with the old
app). The first version's id was set in run 14, while the app was installed
nowhere but on test phones. Once sync is on the phone (M3), a new install
gets its hosts, groups and keys back with "Restore from sync"; until then,
what was only on the phone is lost with the old install.

## The signed release

The release APK is signed with the owner's own key (D218, D223): how to make
the key, build the signed APK, and move from the debug build to it is in
[release.md](release.md). The agent never makes the key; the debug key of
the build computer signs the debug builds.

Without a cable, Android 11+ can pair over Wi-Fi: Developer options →
**Wireless debugging** → Pair device with pairing code, then
`adb pair <ip>:<port>` and `adb connect <ip>:<port>` with the numbers it
shows.
