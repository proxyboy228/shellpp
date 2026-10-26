# The signed release of the Android app

The first version of the Android app (D223) is a release APK signed with
the owner's own key, installed on the owner's phone by hand. Publishing it
(Google Play or anywhere else) is the owner's own step. The agent never
makes, looks for or reads the owner's key (D218); its own checks sign with a
throwaway key (`scripts\verify-android.ps1 -Release`).

## 1. The release signing key (once)

The release APK is signed with the owner's own key. **It cannot be lost**:
every later version must be signed with the same key, or phones refuse the
update and the only way on is to uninstall, which deletes the app's data on
the phone.

1. Make the key with `keytool`, which comes with Android Studio's JDK
   (RSA 4096, valid for 10 000 days, about 27 years):

   ```powershell
   & "$env:ProgramFiles\Android\Android Studio\jbr\bin\keytool.exe" -genkeypair -v -keystore "$env:USERPROFILE\.shellpp\shellpp-release.jks" -alias shellpp -keyalg RSA -keysize 4096 -validity 10000
   ```

   (Make the folder first: `New-Item -ItemType Directory -Force
   "$env:USERPROFILE\.shellpp"`.) It asks for the keystore's password and
   the owner's name. Choose a long password and keep it in a password
   manager.
2. Note the certificate's fingerprint, to compare with the one every build
   prints:

   ```powershell
   & "$env:ProgramFiles\Android\Android Studio\jbr\bin\keytool.exe" -list -v -keystore "$env:USERPROFILE\.shellpp\shellpp-release.jks" -alias shellpp
   ```

   (the line `SHA256:`).
3. Write the signing file `%USERPROFILE%\.shellpp\android-signing.properties`
   (two lines, forward slashes; **no password in it**):

   ```properties
   storeFile=C:/Users/<you>/.shellpp/shellpp-release.jks
   keyAlias=shellpp
   ```

4. Keep the key **outside the repository** (never commit it; `.gitignore`
   and the secret scan stop `*.jks`, `*.keystore`, `*.p12` and the signing
   file, but do not rely on them), with at least two copies: one on the
   computer that builds releases, one offline (an encrypted USB stick, or
   the password manager's file attachments, with the password apart).

## 2. Build the signed APK

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-android-release.ps1
```

It asks for the keystore's password and the key's (hidden; Enter when they
are the same), builds the release APK (both ABIs: the phone's and the
emulator's), and prints:

- the path: `apps\android\app\build\outputs\apk\release\app-release.apk`;
- the certificate's SHA-256: it must be the fingerprint noted in step 1.2.

The passwords go to Gradle only in the script's own environment and end
with it; nothing is written to disk, nothing is printed, Gradle runs without
its daemon and its configuration cache, and PowerShell's history does not
keep what `Read-Host` reads. Another signing file: `-SigningProperties <path>`.

The release build has no R8 and no resource shrinking (the first version;
the Rust library is most of its size). Its version is `1.0.0-rc1`, the
Android app's own line of versions (D223); `versionCode` grows with every
release that goes on a phone.

## 3. From the debug build to the signed one

The debug build on the phone is signed with the build computer's debug key;
the signed release is another signature, so it **does not install over it**
(`INSTALL_FAILED_UPDATE_INCOMPATIBLE`), and uninstalling the debug build
deletes its data. The vault comes back from the sync server:

1. **Sync first.** On the debug build: Settings → Account and sync; the last
   sync must be after the last change, and successful. Pull down on the
   host list to sync now.
2. Have at hand what "Restore from sync" asks: the server's address, the
   account's email and password, and the TOTP app when the account has
   TOTP.
3. Uninstall the debug build: long-press the app's icon → Uninstall (or
   `adb uninstall io.github.proxyboy228.shellpp`).
4. Install the signed APK: copy `app-release.apk` to the phone and open it
   (allow installing from that source), or with the cable:

   ```powershell
   & "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" install apps\android\app\build\outputs\apk\release\app-release.apk
   ```

5. Open Shell++ → **Restore from sync**: the hosts, groups, identities,
   keys, known hosts and the connection log come back.

**What does not come back** (it is the phone's own, not synced):

- the device's settings: the extra-keys row (Customize keyboard), the
  terminal's font size and theme, pinch to zoom, the cursor speed, "Keep the
  screen on", the bell's vibration, the sort orders, the SFTP download
  folder;
- fingerprint unlock: turn it on again in Settings → Lock and fingerprint;
- the permissions the app asked (notifications, the local network): it asks
  again;
- session recordings: they are files on the phone only (D16).

**Without a sync server** the hosts and keys have to be made again on the
signed build: the phone has no import (UI spec 6.1, 13). Keys made on the
phone exist only there; put another key of yours on those servers first.

## 4. Later versions

Build with the same key (section 2) and install over the release build:
`adb install -r app-release.apk`, or open the file on the phone. The data
stays. The checks of a version: `scripts\verify-android.ps1 -Emulator
-Release` (the release build signed with a throwaway key, installed on the
emulator, its first run and a session on OpenSSH).
