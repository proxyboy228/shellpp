<#
.SYNOPSIS
  The Android app's checks (runs 13 and 14; docs/mobile/architecture.md 7):
  the Rust binding's tests on this computer, the debug APK and the Kotlin
  unit tests; with -Emulator also the end-to-end tests on the emulator
  against OpenSSH in Docker.

.DESCRIPTION
  Finds the Android SDK (ANDROID_HOME, ANDROID_SDK_ROOT, else Android
  Studio's default under LOCALAPPDATA) and a JDK (JAVA_HOME, else Android
  Studio's own `jbr`) and sets them for this script's process only: no
  system setting or variable of Windows changes. Steps run through
  scripts\process.ps1 (the program alone is awaited); their output goes to
  <target>\verify-android\<step>.log and <step>.err.log, the summary to the
  console. A step that fails does not stop the others.

  -Emulator: starts OpenSSH in Docker (tests/openssh/start.sh; its
  container carries shellpp-run=$env:SHELLPP_TEST_RUN), starts the emulator
  (-Avd, by default the first AVD) unless one already runs, waits until it
  has booted, installs the APKs afresh and runs the end-to-end classes one
  by one with `am instrument`, each in a process of its own (so a class
  finds what the one before left after a real restart), with the server's
  port and password user and a master password made for the run as
  instrumentation arguments (the emulator reaches the computer's loopback
  at 10.0.2.2). For fingerprint unlock (D218) it gives the emulator the PIN
  1111, enrolls fingers 1 and 2 through Settings
  (tests/android/enroll-fingerprint.sh) and touches the sensor during
  FingerprintOnTest (tests/android/touch-sensor.sh). Then it uninstalls the
  app, clears the PIN (and with it the fingerprints), stops the emulator it
  started and removes the container. Needs a running Docker and Git for
  Windows' bash.

  -Programs (with -Emulator; plan of run 16, 3.3): instead of the other
  classes, the walks through full-screen programs (ProgramsTest: nano with
  Cut in nano, vim, less, htop, mc, tmux from the keys row) on the OpenSSH
  image with the terminal programs (SHELLPP_OPENSSH_PROGRAMS=1 for
  tests/openssh/start.sh; its first build needs the network).

  -Release (D223 item 15): the release build too, signed with a throwaway
  key that this script makes (keytool of the JDK, its passwords only in this
  process's environment) in a folder of its own outside the repository and
  deletes at the end, through scripts\build-android-release.ps1; with
  -Emulator it is then installed afresh and runs VaultFirstRunTest (the
  first run, a host and a session on OpenSSH, by the interface alone: the
  test-only answers are off in a release build) and ReleaseBuildTest (not
  debuggable, fingerprint unlock off by default).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify-android.ps1 -Emulator
  powershell -ExecutionPolicy Bypass -File scripts\verify-android.ps1 -Emulator -Release
#>
param(
    [switch] $Emulator,
    # The release build with a throwaway key (D223 item 15).
    [switch] $Release,
    # With -Emulator: only the walks through full-screen programs
    # (ProgramsTest), on the OpenSSH image with the terminal programs.
    [switch] $Programs,
    # The AVD to start when no emulator runs (default: the first one).
    [string] $Avd = ''
)

$ErrorActionPreference = 'Continue'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'process.ps1')

$repo = Split-Path -Parent $PSScriptRoot
$android = Join-Path $repo 'apps\android'
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$logDir = Join-Path $targetDir 'verify-android'
New-Item -ItemType Directory -Force $logDir | Out-Null
if (-not $env:SHELLPP_TEST_RUN) { $env:SHELLPP_TEST_RUN = 'verify-android' }

function Find-Sdk {
    foreach ($sdk in $env:ANDROID_HOME, $env:ANDROID_SDK_ROOT) {
        if ($sdk -and (Test-Path -LiteralPath (Join-Path $sdk 'platform-tools'))) { return $sdk }
    }
    if ($env:LOCALAPPDATA) {
        $sdk = Join-Path $env:LOCALAPPDATA 'Android\Sdk'
        if (Test-Path -LiteralPath (Join-Path $sdk 'platform-tools')) { return $sdk }
    }
    return $null
}

function Find-Jdk {
    if ($env:JAVA_HOME -and (Test-Path -LiteralPath (Join-Path $env:JAVA_HOME 'bin\java.exe'))) { return $env:JAVA_HOME }
    foreach ($root in $env:ProgramFiles, ${env:ProgramFiles(x86)}) {
        if (-not $root) { continue }
        $jbr = Join-Path $root 'Android\Android Studio\jbr'
        if (Test-Path -LiteralPath (Join-Path $jbr 'bin\java.exe')) { return $jbr }
    }
    return $null
}

$sdk = Find-Sdk
$jdk = Find-Jdk
if (-not $sdk) { Write-Host 'verify-android: no Android SDK (set ANDROID_HOME)' -ForegroundColor Red; exit 1 }
if (-not $jdk) { Write-Host 'verify-android: no JDK (set JAVA_HOME or install Android Studio)' -ForegroundColor Red; exit 1 }
$env:ANDROID_HOME = $sdk
$env:JAVA_HOME = $jdk
$gradlew = Join-Path $android 'gradlew.bat'
# The app's id (D218) and the end-to-end classes, in the order they run.
$appId = 'io.github.proxyboy228.shellpp'
$EndToEndClasses = @(
    'dev.shellpp.android.VaultFirstRunTest',
    'dev.shellpp.android.VaultAfterRestartTest',
    'dev.shellpp.android.EditorKeepsOtherChangesTest',
    'dev.shellpp.android.ConnectTest',
    'dev.shellpp.android.KeysSftpTest',
    'dev.shellpp.android.FingerprintOnTest',
    'dev.shellpp.android.FingerprintInvalidatedTest',
    'dev.shellpp.android.SyncSignInTest',
    'dev.shellpp.android.SyncAfterUnlockTest',
    'dev.shellpp.android.SyncRestoreTest'
)
if ($Programs) { $EndToEndClasses = @('dev.shellpp.android.ProgramsTest', 'dev.shellpp.android.SnapshotTimingTest') }

# The owner's signing never reaches these checks: a signing file or
# passwords set in the environment (for Android Studio) are left out; only
# -Release sets its own throwaway ones (review of run 16).
Remove-Item -ErrorAction SilentlyContinue Env:SHELLPP_ANDROID_SIGNING, Env:SHELLPP_ANDROID_STORE_PASSWORD, Env:SHELLPP_ANDROID_KEY_PASSWORD

$failed = @()
$passed = 0
function Run-Step([string] $Name, [string] $Program, [string[]] $Arguments, [hashtable] $Environment = @{}, [string] $Directory = $repo) {
    $saved = @{}
    foreach ($key in $Environment.Keys) {
        $saved[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $Environment[$key])
    }
    $started = Get-Date
    Write-Host ("{0,-28}" -f $Name) -NoNewline
    Push-Location $Directory
    try {
        $code = Invoke-Logged $Program $Arguments (Join-Path $logDir "$Name.log") (Join-Path $logDir "$Name.err.log")
    } finally {
        Pop-Location
        foreach ($key in $saved.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key]) }
    }
    $minutes = ((Get-Date) - $started).TotalMinutes.ToString('0.0')
    if ($code -eq 0) {
        Write-Host "ok      ($minutes min)" -ForegroundColor Green
        $script:passed += 1
    } else {
        Write-Host "FAILED  ($minutes min, exit $code; $(Join-Path $logDir "$Name.log"))" -ForegroundColor Red
        $script:failed += $Name
    }
    return ($code -eq 0)
}

$null = Run-Step 'rust-binding-tests' 'cargo' @('test', '--locked', '--no-fail-fast', '-p', 'shellpp-android') @{ SHELLPP_TEST = '1'; SHELLPP_TEST_SSHD = 'russh' }
$null = Run-Step 'apk-and-unit-tests' $gradlew @('assembleDebug', 'testDebugUnitTest', '--console=plain') @{} $android
# The license list of the APK (D223 item 14): the Rust crates (and the
# desktop's, same script) and the Android libraries of THIRD_PARTY_NOTICES.md.
$null = Run-Step 'notices' 'node' @('scripts/rust-notices.mjs')
$null = Run-Step 'notices-android' $gradlew @('androidNotices', '--console=plain') @{} $android

# -Release: a throwaway key in a folder of its own outside the repository,
# deleted in the `finally` below (plan of run 16, rule 9); the owner's key
# is never looked for.
$releaseDir = $null
$releaseBuilt = $false
try {
if ($Release) {
    $releaseDir = Join-Path ([IO.Path]::GetTempPath()) ('shellpp-release-check-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force $releaseDir | Out-Null
    $keystore = Join-Path $releaseDir 'release-check.jks'
    $env:SHELLPP_ANDROID_STORE_PASSWORD = [guid]::NewGuid().ToString('N')
    $env:SHELLPP_ANDROID_KEY_PASSWORD = $env:SHELLPP_ANDROID_STORE_PASSWORD
    # The passwords go to keytool through the environment, not its command line.
    $made = Run-Step 'release-key' (Join-Path $jdk 'bin\keytool.exe') @(
        '-genkeypair', '-keystore', $keystore, '-storetype', 'PKCS12', '-alias', 'shellpp',
        '-keyalg', 'RSA', '-keysize', '2048', '-validity', '2', '-dname', 'CN=Shellpp release check',
        '-storepass:env', 'SHELLPP_ANDROID_STORE_PASSWORD', '-keypass:env', 'SHELLPP_ANDROID_KEY_PASSWORD', '-noprompt'
    )
    $signing = Join-Path $releaseDir 'signing.properties'
    [IO.File]::WriteAllLines($signing, [string[]]@("storeFile=$($keystore -replace '\\', '/')", 'keyAlias=shellpp'), (New-Object Text.UTF8Encoding $false))
    if ($made) {
        # Its APK goes into the throwaway folder, never where the owner's goes.
        $releaseBuilt = Run-Step 'release-build' 'powershell' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repo 'scripts\build-android-release.ps1'), '-SigningProperties', $signing, '-OutputDir', (Join-Path $releaseDir 'apk'))
    }
    # For the emulator's install of the release build.
    $env:SHELLPP_ANDROID_SIGNING = $signing
}

if ($Emulator) {
    $adb = Join-Path $sdk 'platform-tools\adb.exe'
    $bash = $env:SHELLPP_BASH
    if (-not $bash) {
        $git = Get-Command git -ErrorAction SilentlyContinue
        if ($git) { $bash = Join-Path (Split-Path -Parent (Split-Path -Parent $git.Source)) 'bin\bash.exe' }
    }
    $work = Join-Path $logDir 'openssh'
    $startedEmulator = $false
    $containerStarted = $false
    $pinSet = $false
    $peer = $null
    $stopTouch = $null
    try {
        if (-not $bash -or -not (Test-Path -LiteralPath $bash)) { throw "no Git for Windows bash (set SHELLPP_BASH)" }
        if (Test-Path -LiteralPath $work) { Remove-Item -Recurse -Force -LiteralPath $work }
        New-Item -ItemType Directory -Force $work | Out-Null
        Write-Host ("{0,-28}" -f 'openssh-container') -NoNewline
        $savedPrograms = $env:SHELLPP_OPENSSH_PROGRAMS
        if ($Programs) { $env:SHELLPP_OPENSSH_PROGRAMS = '1' }
        $code = Invoke-Logged $bash @((Join-Path $repo 'tests\openssh\start.sh'), $work) (Join-Path $logDir 'openssh-start.log') (Join-Path $logDir 'openssh-start.err.log')
        $env:SHELLPP_OPENSSH_PROGRAMS = $savedPrograms
        if ($code -ne 0) { throw "tests/openssh/start.sh failed (exit $code)" }
        $containerStarted = $true
        Write-Host 'ok' -ForegroundColor Green
        $server = Get-Content -Raw -LiteralPath (Join-Path $work 'server.json') | ConvertFrom-Json

        $running = @(& $adb devices 2>$null | Select-String -Pattern '^emulator-\d+\s+device')
        if ($running.Count -eq 0) {
            $emulatorExe = Join-Path $sdk 'emulator\emulator.exe'
            if (-not $Avd) { $Avd = @(& $emulatorExe -list-avds 2>$null | Where-Object { $_ -and $_ -notmatch '^INFO' })[0] }
            if (-not $Avd) { throw 'no AVD to start (create one in Android Studio)' }
            Write-Host ("{0,-28}" -f 'emulator') -NoNewline
            Start-Process -FilePath $emulatorExe -ArgumentList @('-avd', $Avd, '-no-snapshot-save', '-no-boot-anim', '-no-audio') -WindowStyle Minimized | Out-Null
            $startedEmulator = $true
            $deadline = (Get-Date).AddMinutes(6)
            $booted = $false
            while ((Get-Date) -lt $deadline) {
                Start-Sleep -Seconds 5
                # -e: the emulator, also while a phone is connected over USB.
                if ("$(& $adb -e shell getprop sys.boot_completed 2>$null)".Trim() -eq '1') { $booted = $true; break }
            }
            if (-not $booted) { throw "the emulator ($Avd) did not boot in 6 minutes" }
            Write-Host "ok      ($Avd)" -ForegroundColor Green
        }
        # Every adb command and Gradle's install go to the emulator only: a
        # phone connected over USB is never touched (plan of run 15, 1.8).
        # For this process and its children only.
        $env:ANDROID_SERIAL = @(& $adb devices 2>$null | Select-String -Pattern '^(emulator-\d+)\s+device' | ForEach-Object { $_.Matches[0].Groups[1].Value })[0]
        if (-not $env:ANDROID_SERIAL) { throw 'no emulator to run on' }
        # Sync (M3): the second device and the server behind its TLS front
        # (server/tests/android_peer.rs) start now, as they build meanwhile;
        # the phone's sync classes wait for its peer.json, and the script
        # runs the desktop's steps between them through files.
        $peerDir = Join-Path $logDir 'sync-peer'
        if (Test-Path -LiteralPath $peerDir) { Remove-Item -Recurse -Force -LiteralPath $peerDir }
        New-Item -ItemType Directory -Force $peerDir | Out-Null
        $savedPeer = $env:SHELLPP_TEST_ANDROID_PEER
        $savedTest = $env:SHELLPP_TEST
        $env:SHELLPP_TEST_ANDROID_PEER = $peerDir
        $env:SHELLPP_TEST = '1'
        # The walks of -Programs need no second device.
        if (-not $Programs) {
            $peer = Start-Process -FilePath 'cargo' -ArgumentList @('test', '--locked', '--manifest-path', (Join-Path $repo 'server\Cargo.toml'), '--test', 'android_peer') -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $logDir 'sync-peer.log') -RedirectStandardError (Join-Path $logDir 'sync-peer.err.log')
            # Taken now: PowerShell reports no exit code of a process whose
            # handle nobody held while it ran.
            $null = $peer.Handle
        }
        $env:SHELLPP_TEST_ANDROID_PEER = $savedPeer
        $env:SHELLPP_TEST = $savedTest
        # A fresh install (the app's data from earlier runs goes), then each
        # test class in a process of its own, in order: the second class
        # finds what the first one left after a real restart (M1).
        foreach ($package in $appId, "$appId.test") { & $adb uninstall $package 2>$null | Out-Null }
        $installed = Run-Step 'emulator-install' $gradlew @('installDebug', 'installDebugAndroidTest', '--console=plain') @{} $android
        if (-not $installed) { throw 'the APKs did not install' }
        $vaultPassword = 'e2e-' + [guid]::NewGuid().ToString('N')
        $instrumentArgs = @(
            '-e', 'sshPort', "$($server.port)",
            '-e', 'sshUser', "$($server.users.password.name)",
            '-e', 'sshPassword', "$($server.users.password.password)",
            '-e', 'vaultPassword', $vaultPassword,
            # The key user and the run's client key (KeysSftpTest imports it to
            # put the phone's own key on the server, tests/openssh/README.md).
            '-e', 'sshKeyUser', "$($server.users.publickey.name)",
            '-e', 'sshClientKey', [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $work 'client_ed25519')))
        )
        # Fingerprint unlock (D218): the emulator gets the PIN 1111 and
        # finger 1 before FingerprintOnTest, whose prompts pass by a finger
        # touching the sensor meanwhile; finger 2 is added before
        # FingerprintInvalidatedTest (the Keystore key becomes invalid).
        $env:ADB = $adb
        & $adb shell svc power stayon true 2>$null | Out-Null
        & $adb shell locksettings clear --old 1111 2>$null | Out-Null
        & $adb shell locksettings set-pin 1111 2>$null | Out-Null
        $pinSet = $true
        $stopTouch = Join-Path $logDir 'touch-sensor.stop'
        foreach ($class in $EndToEndClasses) {
            $short = $class.Split('.')[-1]
            $toucher = $null
            if ($short -eq 'FingerprintOnTest' -or $short -eq 'FingerprintInvalidatedTest') {
                $finger = if ($short -eq 'FingerprintOnTest') { '1' } else { '2' }
                $enrolled = Invoke-Logged $bash @((Join-Path $repo 'tests\android\enroll-fingerprint.sh'), $finger) (Join-Path $logDir "enroll-$finger.log") (Join-Path $logDir "enroll-$finger.err.log")
                & $adb shell input keyevent KEYCODE_HOME 2>$null | Out-Null
                if ($enrolled -ne 0) {
                    Write-Host ("{0,-34}" -f "e2e-$short") -NoNewline
                    Write-Host "FAILED  (fingerprint $finger not enrolled)" -ForegroundColor Red
                    $script:failed += "e2e-$short"
                    continue
                }
            }
            $classArgs = $instrumentArgs
            if ($short -like 'Sync*') {
                $ready = $true
                if ($short -eq 'SyncSignInTest') {
                    $deadline = (Get-Date).AddMinutes(20)
                    while (-not (Test-Path -LiteralPath (Join-Path $peerDir 'peer.json')) -and -not $peer.HasExited -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 2 }
                    $ready = Test-Path -LiteralPath (Join-Path $peerDir 'peer.json')
                }
                if ($short -eq 'SyncAfterUnlockTest') {
                    New-Item -ItemType File -Force (Join-Path $peerDir 'phone-1.done') | Out-Null
                    $deadline = (Get-Date).AddMinutes(3)
                    while (-not (Test-Path -LiteralPath (Join-Path $peerDir 'desktop-1.done')) -and -not $peer.HasExited -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 1 }
                    $ready = Test-Path -LiteralPath (Join-Path $peerDir 'desktop-1.done')
                }
                if ($short -eq 'SyncRestoreTest') {
                    # A clean install: nothing of the phone's data stays.
                    foreach ($package in $appId, "$appId.test") { & $adb uninstall $package 2>$null | Out-Null }
                    foreach ($apk in 'app\build\outputs\apk\debug\app-debug.apk', 'app\build\outputs\apk\androidTest\debug\app-debug-androidTest.apk') {
                        & $adb install -r (Join-Path $android $apk) 2>$null | Out-Null
                    }
                }
                if (-not $ready) {
                    Write-Host ("{0,-34}" -f "e2e-$short") -NoNewline
                    Write-Host "FAILED  (the sync peer did not come: $(Join-Path $logDir 'sync-peer.log'))" -ForegroundColor Red
                    $script:failed += "e2e-$short"
                    continue
                }
                $peerInfo = Get-Content -Raw -LiteralPath (Join-Path $peerDir 'peer.json') | ConvertFrom-Json
                # The phone reaches the server at the same origin as the
                # desktop (https://localhost:<port>): the account's keys are
                # bound to the origin.
                $peerPort = ([Uri]$peerInfo.origin).Port
                & $adb reverse "tcp:$peerPort" "tcp:$peerPort" 2>$null | Out-Null
                $classArgs = $instrumentArgs + @(
                    '-e', 'syncOrigin', $peerInfo.origin,
                    '-e', 'syncEmail', $peerInfo.email,
                    '-e', 'syncPassword', $peerInfo.password,
                    '-e', 'syncSha256', $peerInfo.sha256,
                    '-e', 'syncCa', [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes($peerInfo.ca_pem))
                )
            }
            if ($short -eq 'FingerprintOnTest') {
                Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $stopTouch
                $toucher = Start-Process -FilePath $bash -ArgumentList @((Join-Path $repo 'tests\android\touch-sensor.sh'), '1', ($stopTouch -replace '\\', '/')) -WindowStyle Hidden -PassThru
            }
            $arguments = @('shell', 'am', 'instrument', '-w', '-r', '-e', 'class', $class) + $classArgs + @("$appId.test/androidx.test.runner.AndroidJUnitRunner")
            Write-Host ("{0,-34}" -f "e2e-$short") -NoNewline
            $log = Join-Path $logDir "e2e-$short.log"
            $code = Invoke-Logged $adb $arguments $log (Join-Path $logDir "e2e-$short.err.log")
            $text = if (Test-Path -LiteralPath $log) { Get-Content -Raw -LiteralPath $log } else { '' }
            # `am instrument` exits 0 even when tests fail: its report says.
            $ok = ($code -eq 0) -and ($text -match 'OK \(\d+ tests?\)') -and ($text -notmatch 'FAILURES!!!|INSTRUMENTATION_FAILED|Process crashed')
            if ($toucher) {
                New-Item -ItemType File -Force $stopTouch | Out-Null
                $null = $toucher.WaitForExit(10000)
            }
            if ($ok) {
                Write-Host 'ok' -ForegroundColor Green
                $script:passed += 1
            } else {
                Write-Host "FAILED  ($log)" -ForegroundColor Red
                $script:failed += "e2e-$short"
            }
        }
        if ($Release -and $releaseBuilt) {
            # The release build, installed afresh: its first run and a session
            # by the interface alone, then its own checks (D223 item 15).
            foreach ($package in $appId, "$appId.test") { & $adb uninstall $package 2>$null | Out-Null }
            $installed = Run-Step 'release-install' $gradlew @('installRelease', 'installReleaseAndroidTest', '-PshellppTestBuildType=release', '--no-daemon', '--no-configuration-cache', '--project-cache-dir', (Join-Path $releaseDir 'gradle'), '-Pkotlin.compiler.execution.strategy=in-process', '--console=plain') @{} $android
            if ($installed) {
                foreach ($class in 'dev.shellpp.android.VaultFirstRunTest', 'dev.shellpp.android.ReleaseBuildTest') {
                    $short = 'release-' + $class.Split('.')[-1]
                    $arguments = @('shell', 'am', 'instrument', '-w', '-r', '-e', 'class', $class) + $instrumentArgs + @("$appId.test/androidx.test.runner.AndroidJUnitRunner")
                    Write-Host ("{0,-34}" -f "e2e-$short") -NoNewline
                    $log = Join-Path $logDir "e2e-$short.log"
                    $code = Invoke-Logged $adb $arguments $log (Join-Path $logDir "e2e-$short.err.log")
                    $text = if (Test-Path -LiteralPath $log) { Get-Content -Raw -LiteralPath $log } else { '' }
                    $ok = ($code -eq 0) -and ($text -match 'OK \(\d+ tests?\)') -and ($text -notmatch 'FAILURES!!!|INSTRUMENTATION_FAILED|Process crashed')
                    if ($ok) {
                        Write-Host 'ok' -ForegroundColor Green
                        $script:passed += 1
                    } else {
                        Write-Host "FAILED  ($log)" -ForegroundColor Red
                        $script:failed += "e2e-$short"
                    }
                }
            }
        }
    } catch {
        Write-Host "FAILED  ($($_.Exception.Message))" -ForegroundColor Red
        $failed += 'emulator'
    } finally {
        # A sensor toucher left by an exception stops too.
        if ($stopTouch) { New-Item -ItemType File -Force $stopTouch | Out-Null }
        if ($peer) {
            & $adb reverse --remove-all 2>$null | Out-Null
            New-Item -ItemType File -Force (Join-Path $peerDir 'stop') | Out-Null
            $exited = $peer.WaitForExit(120000)
            if (-not $exited) { Stop-Process -Id $peer.Id -Force -ErrorAction SilentlyContinue }
            $peerFailed = Join-Path $peerDir 'failed'
            if ((Test-Path -LiteralPath $peerFailed) -or -not $exited -or $peer.ExitCode -ne 0) {
                $why = if (Test-Path -LiteralPath $peerFailed) { Get-Content -Raw -LiteralPath $peerFailed } else { "exit $($peer.ExitCode); $(Join-Path $logDir 'sync-peer.log')" }
                Write-Host ("{0,-34}" -f 'e2e-sync-peer') -NoNewline
                Write-Host "FAILED  ($why)" -ForegroundColor Red
                $script:failed += 'e2e-sync-peer'
            }
        }
        foreach ($package in $appId, "$appId.test") { & $adb uninstall $package 2>$null | Out-Null }
        # Clearing the PIN removes the test's fingerprints too.
        if ($pinSet) { & $adb shell locksettings clear --old 1111 2>$null | Out-Null }
        if ($startedEmulator) { & $adb emu kill 2>$null | Out-Null }
        if ($containerStarted) {
            $null = Invoke-Logged $bash @((Join-Path $repo 'tests\openssh\stop.sh'), $work) (Join-Path $logDir 'openssh-stop.log') (Join-Path $logDir 'openssh-stop.err.log')
        }
    }
}

} finally {
    # The throwaway key, its APKs and passwords go, whatever happened.
    if ($releaseDir -and (Test-Path -LiteralPath $releaseDir)) { Remove-Item -Recurse -Force -LiteralPath $releaseDir -ErrorAction SilentlyContinue }
    if ($Release) { Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -LiteralPath (Join-Path $android 'app\build\outputs\apk\release') }
    foreach ($name in 'SHELLPP_ANDROID_STORE_PASSWORD', 'SHELLPP_ANDROID_KEY_PASSWORD', 'SHELLPP_ANDROID_SIGNING') {
        Remove-Item -ErrorAction SilentlyContinue "Env:$name"
    }
}
if ($releaseDir -and (Test-Path -LiteralPath $releaseDir)) {
    Write-Host "verify-android: the throwaway key is still at $releaseDir" -ForegroundColor Red
    $failed += 'release-key-left'
}

if ($failed.Count -gt 0) {
    Write-Host "verify-android: failed: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host 'verify-android: every step passed' -ForegroundColor Green
exit 0
