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
    [switch] $Deploy
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
# (name, environment, cargo arguments)
$steps = @(
    @('metadata', @{}, @('metadata', '--locked', '--format-version', '1')),
    @('metadata-server', @{}, @('metadata') + $server + @('--locked', '--format-version', '1')),
    @('fmt', @{}, @('fmt', '--all', '--', '--check')),
    @('clippy', @{}, @('clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings')),
    @('clippy-all-features', @{}, @('clippy', '--locked', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings')),
    @('test', @{ SHELLPP_TEST = '1'; SHELLPP_TEST_SSHD = 'russh' }, @('test', '--locked', '--no-fail-fast', '--workspace')),
    @('test-desktop-all-features', @{ SHELLPP_TEST = '1'; SHELLPP_TEST_SSHD = 'russh' }, @('test', '--locked', '--no-fail-fast', '-p', 'shellpp-desktop', '--all-features')),
    @('fmt-server', @{}, @('fmt') + $server + @('--all', '--', '--check')),
    @('clippy-server', @{}, @('clippy', '--locked') + $server + @('--workspace', '--all-targets', '--', '-D', 'warnings')),
    @('clippy-server-all-features', @{}, @('clippy', '--locked') + $server + @('--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings')),
    @('test-server', @{ SHELLPP_TEST = '1' }, @('test', '--locked', '--no-fail-fast') + $server + @('--workspace')),
    @('test-server-postgres', @{ SHELLPP_TEST = '1'; SHELLPP_TEST_POSTGRES = 'docker' }, @('test', '--locked', '--no-fail-fast') + $server + @('--workspace'))
)
if ($Deploy) {
    $steps += , @('test-deploy', @{ SHELLPP_TEST = '1'; SHELLPP_TEST_DEPLOY = 'docker' }, @('test', '--locked', '--no-fail-fast') + $server + @('--test', 'deploy', '--', '--test-threads=1'))
}

$failed = @()
Push-Location $repo
try {
    foreach ($step in $steps) {
        $name, $environment, $arguments = $step
        $log = Join-Path $logDir "$name.log"
        $saved = @{}
        foreach ($key in $environment.Keys) {
            $saved[$key] = [Environment]::GetEnvironmentVariable($key)
            [Environment]::SetEnvironmentVariable($key, $environment[$key])
        }
        $started = Get-Date
        Write-Host ("{0,-28}" -f $name) -NoNewline
        # Start-Process keeps cargo's output as it is (UTF-8, no error
        # records around the lines of stderr) and gives its exit code.
        $process = Start-Process -FilePath 'cargo' -ArgumentList $arguments -NoNewWindow -Wait -PassThru `
            -RedirectStandardOutput $log -RedirectStandardError (Join-Path $logDir "$name.err.log")
        $code = $process.ExitCode
        foreach ($key in $saved.Keys) {
            [Environment]::SetEnvironmentVariable($key, $saved[$key])
        }
        $minutes = ((Get-Date) - $started).TotalMinutes.ToString('0.0')
        if ($code -eq 0) {
            Write-Host "ok      ($minutes min)" -ForegroundColor Green
        } else {
            Write-Host "FAILED  ($minutes min, exit $code; $log)" -ForegroundColor Red
            $failed += $name
        }
    }
} finally {
    Pop-Location
}

if ($failed.Count -gt 0) {
    Write-Host "verify: failed: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host 'verify: every step passed' -ForegroundColor Green
exit 0
