<#
.SYNOPSIS
  The Android app's checks (run 13; docs/mobile/architecture.md 7): the
  Rust binding's tests on this computer, the debug APK and the Kotlin unit
  tests; with -Emulator also the end-to-end test on the emulator against
  OpenSSH in Docker.

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
  has booted, runs connectedDebugAndroidTest with the server's port and its
  password user as instrumentation arguments (the emulator reaches the
  computer's loopback at 10.0.2.2), then stops the emulator it started and
  removes the container. Needs a running Docker and Git for Windows' bash.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify-android.ps1 -Emulator
#>
param(
    [switch] $Emulator,
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
    try {
        if (-not $bash -or -not (Test-Path -LiteralPath $bash)) { throw "no Git for Windows bash (set SHELLPP_BASH)" }
        if (Test-Path -LiteralPath $work) { Remove-Item -Recurse -Force -LiteralPath $work }
        New-Item -ItemType Directory -Force $work | Out-Null
        Write-Host ("{0,-28}" -f 'openssh-container') -NoNewline
        $code = Invoke-Logged $bash @((Join-Path $repo 'tests\openssh\start.sh'), $work) (Join-Path $logDir 'openssh-start.log') (Join-Path $logDir 'openssh-start.err.log')
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
                if ("$(& $adb shell getprop sys.boot_completed 2>$null)".Trim() -eq '1') { $booted = $true; break }
            }
            if (-not $booted) { throw "the emulator ($Avd) did not boot in 6 minutes" }
            Write-Host "ok      ($Avd)" -ForegroundColor Green
        }
        $arguments = @(
            'connectedDebugAndroidTest', '--console=plain',
            # The screenshots run only when asked (ScreenshotsTest, argument
            # screenshots=<theme>); here the test would report its skip as a failure.
            '-Pandroid.testInstrumentationRunnerArguments.notClass=dev.shellpp.android.ScreenshotsTest',
            "-Pandroid.testInstrumentationRunnerArguments.sshPort=$($server.port)",
            "-Pandroid.testInstrumentationRunnerArguments.sshUser=$($server.users.password.name)",
            "-Pandroid.testInstrumentationRunnerArguments.sshPassword=$($server.users.password.password)"
        )
        $null = Run-Step 'emulator-end-to-end' $gradlew $arguments @{} $android
    } catch {
        Write-Host "FAILED  ($($_.Exception.Message))" -ForegroundColor Red
        $failed += 'emulator'
    } finally {
        if ($startedEmulator) { & $adb emu kill 2>$null | Out-Null }
        if ($containerStarted) {
            $null = Invoke-Logged $bash @((Join-Path $repo 'tests\openssh\stop.sh'), $work) (Join-Path $logDir 'openssh-stop.log') (Join-Path $logDir 'openssh-stop.err.log')
        }
    }
}

if ($failed.Count -gt 0) {
    Write-Host "verify-android: failed: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host 'verify-android: every step passed' -ForegroundColor Green
exit 0
