# Shell++ for Android

The Android app is being designed and built in the mobile runs
(`docs/ROADMAP.md`, 3.1). Its documents:

- `owner-notes.md`: the owner's description of the app (D211);
- `architecture.md`: the stack, the boundary with the Rust core, threads,
  secrets, TLS, the terminal (D213);
- `ui-spec.md`: the screens of the first version (D214).

Run 13 built a **prototype** (`apps/android`): a quick connect over SSH
(address, port, user, a password or a key file) to a terminal with the extra
keys, cursor placement by tap and selection. It has no vault and no sync:
known hosts last until the app restarts.

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
the end-to-end test on the emulator against OpenSSH in Docker. The Gradle
build runs `cargo build --profile android` for both targets itself.

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
this one. To remove it: `adb uninstall dev.shellpp.android`, or long-press
the app's icon → Uninstall.

Without a cable, Android 11+ can pair over Wi-Fi: Developer options →
**Wireless debugging** → Pair device with pairing code, then
`adb pair <ip>:<port>` and `adb connect <ip>:<port>` with the numbers it
shows.
