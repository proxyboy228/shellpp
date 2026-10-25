<#
.SYNOPSIS
  The full test criterion of CLAUDE.md (run 10): every step runs, and the
  script fails when one of them failed.

.DESCRIPTION
  Runs, in the working copy the script belongs to (for the check before
  main moves: the script of a clean worktree with its own CARGO_TARGET_DIR,
  ROADMAP 7.1):

  - cargo metadata --locked for both workspaces;
  - the secret scan: gitleaks in Docker over the whole history of every
    branch, with the image, config and ignore file of the Secret scan
    workflow (run 12); a finding fails the step;
  - cargo deny (licenses, bans, advisories, sources) for both workspaces,
    with the configs and the cargo-deny version of CI's "Licenses and
    advisories" job (run 13). The advisory database and cargo's index (for
    yanked crates) are refreshed first; without the network those steps only
    warn and the checks use the copies already downloaded, and with no
    database at all they fail;
  - the root workspace: rustfmt, clippy (default and all features), clippy
    of the libraries and binaries alone, the tests
    (SHELLPP_TEST_SSHD=russh) and the desktop's tests with all features;
  - the server workspace: rustfmt, clippy (default and all features),
    clippy of the libraries and binaries alone, the tests on SQLite and on
    PostgreSQL in Docker;
  - the "mobile-ready" check (docs/architecture.md, 2.3; run 13): cargo
    check of the crates a phone client shares for aarch64-linux-android,
    with the clang of the Android NDK. Without an NDK the step shows
    SKIPPED (no NDK) and is named in the summary: never a green mark;
  - with -Deploy, the deployment tests in Docker (they need the network).

  Clippy of the libraries and binaries alone sees the code as a release
  build does: with --all-targets the tests' dev-dependencies turn on
  shellpp-core's test-hooks for the whole workspace (feature unification),
  and a warning in code that only the hooks use stays hidden (run 11).

  Every cargo command that reads Cargo.lock runs with --locked, so a stale
  lock file fails the step instead of being rewritten; every test step runs
  with --no-fail-fast, so the failures of every test binary show. A step
  that fails does not stop the others; a step that may only warn (the
  advisory and index refreshes) shows WARNING instead and is named in the
  summary. The
  output goes to
  <target>\verify\<step>.log and <step>.err.log; the summary to the
  console.

  The test variables of the caller's session (SHELLPP_TEST_POSTGRES,
  SHELLPP_TEST_DEPLOY, the SSH server, ...) are cleared first: each step
  sets what it needs, as CI does. Test containers carry the label
  shellpp-run=$env:SHELLPP_TEST_RUN (default "verify").

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify.ps1 -Deploy
#>
param(
    # Also the deployment tests (CLAUDE.md, "Проверка развёртывания").
    [switch] $Deploy,
    # Only list the steps with their environment and command.
    [switch] $List
)

$ErrorActionPreference = 'Continue'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'process.ps1')
if (-not (Get-Command Invoke-Logged -ErrorAction SilentlyContinue)) {
    Write-Host 'verify: scripts\process.ps1 did not load' -ForegroundColor Red
    exit 1
}

$repo = Split-Path -Parent $PSScriptRoot
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$logDir = Join-Path $targetDir 'verify'
New-Item -ItemType Directory -Force $logDir | Out-Null
if (-not $env:SHELLPP_TEST_RUN) { $env:SHELLPP_TEST_RUN = 'verify' }
# A variable left from an earlier command would change what a step tests
# (SQLite on PostgreSQL, the deployment tests in parallel, a shared OpenSSH
# container): each step sets its own.
foreach ($name in 'SHELLPP_TEST', 'SHELLPP_TEST_POSTGRES', 'SHELLPP_TEST_DEPLOY', 'SHELLPP_TEST_SSHD',
    'SHELLPP_OPENSSH_SERVER_JSON', 'SHELLPP_TEST_PUTTYGEN', 'SHELLPP_SFTP_BENCH') {
    [Environment]::SetEnvironmentVariable($name, $null)
}

$server = @('--manifest-path', 'server/Cargo.toml')

# One step: its name, its environment, its cargo arguments (or, with
# -Program, another program's). A function, not nested array literals: in
# PowerShell the comma binds tighter than `+`, so `@('a', @{}, @('x') + $y)`
# is not three elements (run 10). A step with a -Warning only warns when it
# fails, with that text; a step with a -Skip does not run and says why.
function Step([string] $Name, [hashtable] $Environment, [string[]] $Arguments, [string] $Program = 'cargo', [string] $Problem = '', [string] $Warning = '', [string] $Skip = '') {
    [pscustomobject]@{ Name = $Name; Environment = $Environment; Arguments = $Arguments; Program = $Program; Problem = $Problem; Warning = $Warning; Skip = $Skip }
}

# The Android NDK for the "mobile-ready" check: ANDROID_NDK_HOME or
# ANDROID_NDK_ROOT, else the newest NDK of an Android SDK (ANDROID_HOME,
# ANDROID_SDK_ROOT, or Android Studio's default under LOCALAPPDATA). $null
# when there is none.
function Find-Ndk {
    foreach ($ndk in $env:ANDROID_NDK_HOME, $env:ANDROID_NDK_ROOT) {
        if ($ndk -and (Test-Path -LiteralPath (Join-Path $ndk 'toolchains'))) { return $ndk }
    }
    $sdks = @($env:ANDROID_HOME, $env:ANDROID_SDK_ROOT)
    if ($env:LOCALAPPDATA) { $sdks += Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
    foreach ($sdk in $sdks) {
        if (-not $sdk) { continue }
        $root = Join-Path $sdk 'ndk'
        if (-not (Test-Path -LiteralPath $root)) { continue }
        $newest = Get-ChildItem -LiteralPath $root -Directory |
            Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'toolchains') } |
            Sort-Object { try { [version]$_.Name } catch { [version]'0.0' } } -Descending |
            Select-Object -First 1
        if ($newest) { return $newest.FullName }
    }
    return $null
}

# The repository's git directory (the common one of a worktree), read from
# the files git writes, without running git: its output would be decoded
# with the console's code page and lose a path's non-ASCII letters (review
# of run 12). $null when the working copy has none.
function Get-GitCommonDir([string] $root) {
    $dotGit = Join-Path $root '.git'
    if (Test-Path -LiteralPath $dotGit -PathType Container) {
        return (Resolve-Path -LiteralPath $dotGit).Path
    }
    if (-not (Test-Path -LiteralPath $dotGit -PathType Leaf)) { return $null }
    $line = Get-Content -LiteralPath $dotGit -Encoding UTF8 -TotalCount 1
    if ("$line" -notmatch '^gitdir:\s*(.+)$') { return $null }
    $gitDir = $Matches[1].Trim()
    if (-not [System.IO.Path]::IsPathRooted($gitDir)) { $gitDir = Join-Path $root $gitDir }
    $commonFile = Join-Path $gitDir 'commondir'
    if (Test-Path -LiteralPath $commonFile -PathType Leaf) {
        $common = (Get-Content -LiteralPath $commonFile -Encoding UTF8 -TotalCount 1).Trim()
        if (-not [System.IO.Path]::IsPathRooted($common)) { $common = Join-Path $gitDir $common }
        $gitDir = $common
    }
    if (-not (Test-Path -LiteralPath $gitDir -PathType Container)) { return $null }
    return (Resolve-Path -LiteralPath $gitDir).Path
}

# The secret scan of the Secret scan workflow (.github/workflows/secret-scan.yml):
# gitleaks of the same version, here its Docker image pinned by digest, with
# no network. The git directory is mounted on its own: in a worktree `.git`
# is a file naming a Windows path the container cannot follow. The scan
# covers every ref of the repository (branches, tags, stash, remote-tracking
# refs), that is the committed history, not uncommitted changes.
$gitleaksImage = 'ghcr.io/gitleaks/gitleaks:v8.30.1@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f'
$gitDir = $null
try { $gitDir = Get-GitCommonDir $repo } catch { $gitDir = $null }
$gitleaksProblem = if ($gitDir) { '' } else { "no git directory found for $repo" }
$gitleaks = @('run', '--rm', '--network', 'none', '-v', "${repo}:/src:ro", '-v', "${gitDir}:/git:ro", $gitleaksImage,
    'git', '--config', '/src/.gitleaks.toml', '--gitleaks-ignore-path', '/src/.gitleaksignore',
    '--redact', '--verbose', '--exit-code', '1', '/git')

# cargo deny with the configs of CI's "Licenses and advisories" job
# (deny.toml, server/deny.toml). Both use RustSec's default advisory
# database, so one fetch serves both checks; the checks then run offline,
# so a missing network costs only the fetch (a warning) and a missing
# database fails them (D205, run 13). The yanked-crate check reads cargo's
# cached index, which a resolve never refreshes for crates already locked:
# `cargo update --dry-run` refreshes it (the lock file stays as it is), as
# CI's fresh index is (review of run 13). The same cargo-deny as CI's.
$denyVersion = '0.20.2'
$denyFetchWarning = 'could not fetch the advisory database (no network?): the deny steps use the copy already downloaded'
$denyIndexWarning = 'could not refresh the crates.io index (no network?): yanked crates are checked against the cached index'
$denyProblem = ''
if (-not (Get-Command cargo-deny -ErrorAction SilentlyContinue)) {
    $denyProblem = "cargo-deny is not installed (CI uses $denyVersion`: cargo install --locked cargo-deny@$denyVersion)"
} else {
    $installed = "$(& cargo-deny --version 2>$null)".Trim()
    if ($installed -ne "cargo-deny $denyVersion") {
        $denyProblem = "found '$installed', CI uses $denyVersion`: cargo install --locked cargo-deny@$denyVersion"
    }
}

# The "mobile-ready" check (docs/architecture.md, 2.3): the crates a phone
# client shares build for the Samsung S25's target with the NDK's clang
# (API level 26, docs/android-feasibility.md) for the C parts (SQLite,
# ring). No NDK: skipped, said so; no Rust target: a failure with the fix.
$androidTarget = 'aarch64-linux-android'
$mobileCrates = @('shellpp-core', 'shellpp-ssh', 'shellpp-sync-api', 'shellpp-private-file', 'shellpp-sessions', 'shellpp-terminal')
$mobileSkip = ''
$mobileProblem = ''
$mobileEnvironment = @{}
$ndk = Find-Ndk
if (-not $ndk) {
    $mobileSkip = 'no NDK: set ANDROID_NDK_HOME or install one with the Android SDK'
} else {
    $llvm = Join-Path $ndk 'toolchains\llvm\prebuilt\windows-x86_64\bin'
    $clang = Join-Path $llvm "${androidTarget}26-clang.cmd"
    if (-not (Test-Path -LiteralPath $clang)) {
        $mobileProblem = "the NDK at $ndk has no $clang"
    } elseif (-not (@(& rustup target list --installed 2>$null) -contains $androidTarget)) {
        $mobileProblem = "the Rust target is missing: rustup target add $androidTarget"
    }
    $mobileEnvironment = @{
        'CC_aarch64_linux_android'                 = $clang
        'AR_aarch64_linux_android'                 = Join-Path $llvm 'llvm-ar.exe'
        'CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER' = $clang
    }
}
$mobileArguments = @('check', '--locked', '--target', $androidTarget) + @($mobileCrates | ForEach-Object { '-p'; $_ })

$russh = @{ SHELLPP_TEST = '1'; SHELLPP_TEST_SSHD = 'russh' }
$steps = @(
    (Step 'metadata' @{} @('metadata', '--locked', '--format-version', '1')),
    (Step 'metadata-server' @{} (@('metadata') + $server + @('--locked', '--format-version', '1'))),
    (Step 'gitleaks' @{} $gitleaks 'docker' $gitleaksProblem),
    (Step 'deny-fetch' @{} @('deny', 'fetch', 'db') -Problem $denyProblem -Warning $denyFetchWarning),
    (Step 'deny-index' @{} @('update', '--dry-run') -Warning $denyIndexWarning),
    (Step 'deny-index-server' @{} (@('update', '--dry-run') + $server) -Warning $denyIndexWarning),
    (Step 'deny' @{} @('deny', '--locked', '--offline', 'check') -Problem $denyProblem),
    (Step 'deny-server' @{} (@('deny', '--locked', '--offline') + $server + @('check')) -Problem $denyProblem),
    (Step 'fmt' @{} @('fmt', '--all', '--', '--check')),
    (Step 'clippy' @{} @('clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings')),
    (Step 'clippy-all-features' @{} @('clippy', '--locked', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings')),
    (Step 'clippy-lib-bin' @{} @('clippy', '--locked', '--workspace', '--', '-D', 'warnings')),
    (Step 'test' $russh @('test', '--locked', '--no-fail-fast', '--workspace')),
    (Step 'test-desktop-all-features' $russh @('test', '--locked', '--no-fail-fast', '-p', 'shellpp-desktop', '--all-features')),
    (Step 'mobile-ready' $mobileEnvironment $mobileArguments -Problem $mobileProblem -Skip $mobileSkip),
    (Step 'fmt-server' @{} (@('fmt') + $server + @('--all', '--', '--check'))),
    (Step 'clippy-server' @{} (@('clippy', '--locked') + $server + @('--workspace', '--all-targets', '--', '-D', 'warnings'))),
    (Step 'clippy-server-all-features' @{} (@('clippy', '--locked') + $server + @('--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings'))),
    (Step 'clippy-server-lib-bin' @{} (@('clippy', '--locked') + $server + @('--workspace', '--', '-D', 'warnings'))),
    (Step 'test-server' @{ SHELLPP_TEST = '1' } (@('test', '--locked', '--no-fail-fast') + $server + @('--workspace'))),
    (Step 'test-server-postgres' @{ SHELLPP_TEST = '1'; SHELLPP_TEST_POSTGRES = 'docker' } (@('test', '--locked', '--no-fail-fast') + $server + @('--workspace')))
)
if ($Deploy) {
    $steps += Step 'test-deploy' @{ SHELLPP_TEST = '1'; SHELLPP_TEST_DEPLOY = 'docker' } (@('test', '--locked', '--no-fail-fast') + $server + @('--test', 'deploy', '--', '--test-threads=1'))
}

if ($List) {
    foreach ($step in $steps) {
        $environment = ($step.Environment.Keys | Sort-Object | ForEach-Object { "$_=$($step.Environment[$_])" }) -join ' '
        Write-Host ("{0,-28} {1,-50} {2} {3}" -f $step.Name, $environment, $step.Program, ($step.Arguments -join ' '))
    }
    exit 0
}

$failed = @()
$warned = @()
$skipped = @()
$passed = 0
Push-Location $repo
try {
    foreach ($step in $steps) {
        $name = $step.Name
        $log = Join-Path $logDir "$name.log"
        $saved = @{}
        foreach ($key in $step.Environment.Keys) {
            $saved[$key] = [Environment]::GetEnvironmentVariable($key)
            [Environment]::SetEnvironmentVariable($key, $step.Environment[$key])
        }
        $started = Get-Date
        Write-Host ("{0,-28}" -f $name) -NoNewline
        if ($step.Skip -and -not $step.Problem) {
            Write-Host "SKIPPED ($($step.Skip))" -ForegroundColor Yellow
            $skipped += "$name ($($step.Skip))"
            foreach ($key in $saved.Keys) {
                [Environment]::SetEnvironmentVariable($key, $saved[$key])
            }
            continue
        }
        if ($step.Problem) {
            # A step that cannot run is a failure, never a skip.
            Write-Host "FAILED  ($($step.Problem))" -ForegroundColor Red
            $failed += $name
            foreach ($key in $saved.Keys) {
                [Environment]::SetEnvironmentVariable($key, $saved[$key])
            }
            continue
        }
        # The program alone is awaited, not what it left running (process.ps1);
        # a step that does not even start is a failure (-1).
        $code = -1
        $code = Invoke-Logged $step.Program $step.Arguments $log (Join-Path $logDir "$name.err.log")
        foreach ($key in $saved.Keys) {
            [Environment]::SetEnvironmentVariable($key, $saved[$key])
        }
        $minutes = ((Get-Date) - $started).TotalMinutes.ToString('0.0')
        if ($code -eq 0) {
            Write-Host "ok      ($minutes min)" -ForegroundColor Green
            $passed += 1
        } elseif ($step.Warning) {
            Write-Host "WARNING ($minutes min, exit $code; $($step.Warning); $log)" -ForegroundColor Yellow
            $warned += $name
            $passed += 1
        } else {
            Write-Host "FAILED  ($minutes min, exit $code; $log)" -ForegroundColor Red
            $failed += $name
        }
    }
} finally {
    Pop-Location
}

# Every step passed, and there were steps: a script that broke before its
# steps (a variable never set under StrictMode) must not read as green.
$stepCount = if (Get-Variable -Name steps -Scope Script -ErrorAction SilentlyContinue) { @($steps).Count } else { 0 }
if (($stepCount -eq 0 -or $passed + $skipped.Count -ne $stepCount) -and $failed.Count -eq 0) {
    $failed += 'the step count'
}
if ($failed.Count -gt 0) {
    Write-Host "verify: failed: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
if ($warned.Count -gt 0 -or $skipped.Count -gt 0) {
    $notes = @()
    if ($warned.Count -gt 0) { $notes += "warnings: $($warned -join ', ')" }
    if ($skipped.Count -gt 0) { $notes += "skipped: $($skipped -join ', ')" }
    Write-Host "verify: every step that ran passed; $($notes -join '; ')" -ForegroundColor Yellow
    exit 0
}
Write-Host 'verify: every step passed' -ForegroundColor Green
exit 0
