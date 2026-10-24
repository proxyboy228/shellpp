<#
.SYNOPSIS
  The full test criterion of CLAUDE.md (run 10): every step runs, and the
  script fails when one of them failed.

.DESCRIPTION
  Runs, in the working copy the script belongs to (for the check before
  main moves: the script of a clean worktree with its own CARGO_TARGET_DIR,
  ROADMAP 7.1):

  - cargo metadata --locked for both workspaces;
  - the root workspace: rustfmt, clippy (default and all features), the
    tests (SHELLPP_TEST_SSHD=russh) and the desktop's tests with all
    features;
  - the server workspace: rustfmt, clippy (default and all features), the
    tests on SQLite and on PostgreSQL in Docker;
  - with -Deploy, the deployment tests in Docker (they need the network).

  Every cargo command that reads Cargo.lock runs with --locked, so a stale
  lock file fails the step instead of being rewritten; every test step runs
  with --no-fail-fast, so the failures of every test binary show. A step
  that fails does not stop the others. The output goes to
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

# One step: its name, its environment, its cargo arguments. A function, not
# nested array literals: in PowerShell the comma binds tighter than `+`, so
# `@('a', @{}, @('x') + $y)` is not three elements (run 10).
function Step([string] $Name, [hashtable] $Environment, [string[]] $Arguments) {
    [pscustomobject]@{ Name = $Name; Environment = $Environment; Arguments = $Arguments }
}
$russh = @{ SHELLPP_TEST = '1'; SHELLPP_TEST_SSHD = 'russh' }
$steps = @(
    (Step 'metadata' @{} @('metadata', '--locked', '--format-version', '1')),
    (Step 'metadata-server' @{} (@('metadata') + $server + @('--locked', '--format-version', '1'))),
    (Step 'fmt' @{} @('fmt', '--all', '--', '--check')),
    (Step 'clippy' @{} @('clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings')),
    (Step 'clippy-all-features' @{} @('clippy', '--locked', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings')),
    (Step 'test' $russh @('test', '--locked', '--no-fail-fast', '--workspace')),
    (Step 'test-desktop-all-features' $russh @('test', '--locked', '--no-fail-fast', '-p', 'shellpp-desktop', '--all-features')),
    (Step 'fmt-server' @{} (@('fmt') + $server + @('--all', '--', '--check'))),
    (Step 'clippy-server' @{} (@('clippy', '--locked') + $server + @('--workspace', '--all-targets', '--', '-D', 'warnings'))),
    (Step 'clippy-server-all-features' @{} (@('clippy', '--locked') + $server + @('--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings'))),
    (Step 'test-server' @{ SHELLPP_TEST = '1' } (@('test', '--locked', '--no-fail-fast') + $server + @('--workspace'))),
    (Step 'test-server-postgres' @{ SHELLPP_TEST = '1'; SHELLPP_TEST_POSTGRES = 'docker' } (@('test', '--locked', '--no-fail-fast') + $server + @('--workspace')))
)
if ($Deploy) {
    $steps += Step 'test-deploy' @{ SHELLPP_TEST = '1'; SHELLPP_TEST_DEPLOY = 'docker' } (@('test', '--locked', '--no-fail-fast') + $server + @('--test', 'deploy', '--', '--test-threads=1'))
}

if ($List) {
    foreach ($step in $steps) {
        $environment = ($step.Environment.Keys | Sort-Object | ForEach-Object { "$_=$($step.Environment[$_])" }) -join ' '
        Write-Host ("{0,-28} {1,-50} cargo {2}" -f $step.Name, $environment, ($step.Arguments -join ' '))
    }
    exit 0
}

$failed = @()
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
        # Start-Process keeps cargo's output as it is (UTF-8, no error
        # records around the lines of stderr) and gives its exit code. A
        # step that does not even start is a failure.
        $code = -1
        try {
            $process = Start-Process -FilePath 'cargo' -ArgumentList $step.Arguments -NoNewWindow -Wait -PassThru `
                -RedirectStandardOutput $log -RedirectStandardError (Join-Path $logDir "$name.err.log") -ErrorAction Stop
            $code = $process.ExitCode
        } catch {
            Write-Host "could not start: $($_.Exception.Message)" -NoNewline
        }
        foreach ($key in $saved.Keys) {
            [Environment]::SetEnvironmentVariable($key, $saved[$key])
        }
        $minutes = ((Get-Date) - $started).TotalMinutes.ToString('0.0')
        if ($code -eq 0) {
            Write-Host "ok      ($minutes min)" -ForegroundColor Green
            $passed += 1
        } else {
            Write-Host "FAILED  ($minutes min, exit $code; $log)" -ForegroundColor Red
            $failed += $name
        }
    }
} finally {
    Pop-Location
}

if ($passed -ne $steps.Count -and $failed.Count -eq 0) {
    $failed += 'the step count'
}
if ($failed.Count -gt 0) {
    Write-Host "verify: failed: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host 'verify: every step passed' -ForegroundColor Green
exit 0
