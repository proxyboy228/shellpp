<#
.SYNOPSIS
  The client's build and tests on Linux, in Docker (run 12): release 1.0
  ships for Windows and Linux (D204), and there is no other Linux machine.

.DESCRIPTION
  Builds the image tests/linux/Dockerfile (the Rust toolchain of
  rust-toolchain.toml and the system libraries GPUI links, as CI installs
  them) and runs in a container of it, with this working copy mounted
  read-only:

  - build: cargo build --locked -p shellpp-desktop (the app);
  - test: cargo test --locked --no-fail-fast --workspace with
    SHELLPP_TEST=1 and SHELLPP_TEST_SSHD=russh, as the full criterion does
    on Windows (GPUI's tests run headless: no display server);
  - with -AllFeatures also the desktop's tests with all features.

  The target folder and cargo's registry are Docker volumes
  (shellpp-linux-target, shellpp-linux-cargo), so a second run is warm;
  -Clean removes them first. Each step's output goes to
  <target>\verify-linux\<step>.log. Not part of the full criterion.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify-linux.ps1
#>
param(
    # Also the desktop's tests with all features.
    [switch] $AllFeatures,
    # Remove the volumes (target folder, registry) first.
    [switch] $Clean
)

$ErrorActionPreference = 'Continue'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$logDir = Join-Path $targetDir 'verify-linux'
New-Item -ItemType Directory -Force $logDir | Out-Null
$run = if ($env:SHELLPP_TEST_RUN) { $env:SHELLPP_TEST_RUN } else { 'local' }
$image = 'shellpp-linux-build:local'

function Fail([string] $message) {
    Write-Host "verify-linux: $message" -ForegroundColor Red
    exit 1
}

# Docker with Linux containers, or nothing runs. Every docker call goes
# through Start-Process (as verify.ps1 does): the output stays as it is, and
# a step that does not even start is a failure, never a pass (review of run
# 12: with no docker on the PATH the script read as green).
function Invoke-Docker([string[]] $Arguments, [string] $Log) {
    $quoted = $Arguments | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }
    try {
        $process = Start-Process -FilePath 'docker' -ArgumentList $quoted -NoNewWindow -Wait -PassThru `
            -RedirectStandardOutput $Log -RedirectStandardError "$Log.err" -ErrorAction Stop
        return $process.ExitCode
    } catch {
        Add-Content -LiteralPath "$Log.err" -Value "could not start docker: $($_.Exception.Message)"
        return -1
    }
}

$infoLog = Join-Path $logDir 'docker-info.log'
$code = Invoke-Docker @('info', '--format', '{{.OSType}}') $infoLog
$osType = if (Test-Path -LiteralPath $infoLog) { "$(Get-Content -LiteralPath $infoLog -Raw)".Trim() } else { '' }
if ($code -ne 0 -or $osType -ne 'linux') { Fail "Docker with Linux containers is not running (see $infoLog.err)" }
if ($Clean) {
    $null = Invoke-Docker @('volume', 'rm', 'shellpp-linux-target', 'shellpp-linux-cargo') (Join-Path $logDir 'clean.log')
}
Write-Host 'verify-linux: building the image'
$imageLog = Join-Path $logDir 'image.log'
$code = Invoke-Docker @('build', '-q', '-t', $image, '-f', (Join-Path $repo 'tests\linux\Dockerfile'), (Join-Path $repo 'tests\linux')) $imageLog
if ($code -ne 0) { Fail "the image did not build (see $imageLog and $imageLog.err)" }

$steps = @(
    @{ Name = 'build'; Command = 'cargo build --locked -p shellpp-desktop' },
    @{ Name = 'test'; Command = 'cargo test --locked --no-fail-fast --workspace' }
)
if ($AllFeatures) {
    $steps += @{ Name = 'test-desktop-all-features'; Command = 'cargo test --locked --no-fail-fast -p shellpp-desktop --all-features' }
}

$failed = @()
$passed = 0
foreach ($step in $steps) {
    $log = Join-Path $logDir "$($step.Name).log"
    $started = Get-Date
    Write-Host ("{0,-28}" -f $step.Name) -NoNewline
    $code = Invoke-Docker @('run', '--rm', '--label', 'shellpp-test=linux', '--label', "shellpp-run=$run",
        '-v', "${repo}:/src:ro", '-v', 'shellpp-linux-target:/target', '-v', 'shellpp-linux-cargo:/usr/local/cargo/registry',
        $image, 'bash', '-c', $step.Command) $log
    $minutes = ((Get-Date) - $started).TotalMinutes.ToString('0.0')
    if ($code -eq 0) {
        Write-Host "ok      ($minutes min)" -ForegroundColor Green
        $passed += 1
    } else {
        Write-Host "FAILED  ($minutes min, exit $code; $log)" -ForegroundColor Red
        $failed += $step.Name
    }
}
if ($failed.Count -gt 0 -or $passed -ne $steps.Count) {
    Write-Host "verify-linux: failed: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host 'verify-linux: every step passed' -ForegroundColor Green
exit 0