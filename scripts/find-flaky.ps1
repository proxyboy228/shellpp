<#
.SYNOPSIS
  Runs the whole test suite several times in a row and lists every test that
  failed at least once (run 8, plan C2): the client workspace on the russh
  test server and the server workspace on SQLite.

.DESCRIPTION
  Builds the tests once (`--no-run`), then runs each suite -Runs times with
  --no-fail-fast, so one failing binary does not hide the others. Every
  run's output goes to <OutDir>\client-<n>.log and server-<n>.log; the
  summary <OutDir>\summary.txt lists each failed test with the runs it
  failed in. SHELLPP_TEST=1 keeps every test away from the permanent
  profile; SHELLPP_TEST_SSHD=russh keeps the desktop tests off a shared
  OpenSSH container (CLAUDE.md).

  The machine's load is part of the result: a test that fails only under
  load is still flaky.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\find-flaky.ps1 -Runs 15 -OutDir D:\flaky
#>
param(
    [int] $Runs = 15,
    # Output folder; default <repo>\target\flaky.
    [string] $OutDir,
    # Leave out the server workspace.
    [switch] $ClientOnly
)

# 'Continue': in Windows PowerShell 5.1 a native command's stderr through
# 2>&1 is an error record, and 'Stop' would end the run at cargo's first
# progress line. Exit codes are checked instead.
$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'target\flaky' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$env:SHELLPP_TEST = '1'
$env:SHELLPP_TEST_SSHD = 'russh'
Remove-Item Env:SHELLPP_TEST_POSTGRES -ErrorAction SilentlyContinue

$suites = @(@{ Name = 'client'; Args = @('test', '--workspace') })
if (-not $ClientOnly) {
    $suites += @{ Name = 'server'; Args = @('test', '--manifest-path', 'server/Cargo.toml', '--workspace') }
}

Push-Location $repo
try {
    foreach ($suite in $suites) {
        Write-Host "find-flaky: building the $($suite.Name) tests"
        & cargo @($suite.Args + @('--no-run')) 2>&1 | Out-File -Encoding utf8 (Join-Path $OutDir "$($suite.Name)-build.log")
        if ($LASTEXITCODE -ne 0) { Write-Host "find-flaky: the $($suite.Name) build failed" -ForegroundColor Red; exit 1 }
    }
    # test name -> the runs it failed in
    $failed = @{}
    for ($run = 1; $run -le $Runs; $run++) {
        foreach ($suite in $suites) {
            $log = Join-Path $OutDir "$($suite.Name)-$run.log"
            $started = Get-Date
            & cargo @($suite.Args + @('--no-fail-fast')) 2>&1 | Out-File -Encoding utf8 $log
            $code = $LASTEXITCODE
            $names = @(Select-String -LiteralPath $log -Pattern '^test (\S+) \.\.\. FAILED' |
                    ForEach-Object { "$($suite.Name): $($_.Matches[0].Groups[1].Value)" })
            # A binary that crashed or timed out has no "FAILED" line.
            if ($code -ne 0 -and $names.Count -eq 0) { $names = @("$($suite.Name): (exit $code, see $log)") }
            foreach ($name in $names) {
                if (-not $failed.ContainsKey($name)) { $failed[$name] = @() }
                $failed[$name] += $run
            }
            $took = [int]((Get-Date) - $started).TotalSeconds
            Write-Host ("find-flaky: run {0}/{1} {2}: exit {3}, {4} failed, {5} s" -f $run, $Runs, $suite.Name, $code, $names.Count, $took)
        }
    }
    $lines = @("$Runs runs; tests that failed at least once:")
    if ($failed.Count -eq 0) { $lines += '(none)' }
    foreach ($name in ($failed.Keys | Sort-Object)) {
        $lines += "$name  (runs $($failed[$name] -join ', '))"
    }
    $lines | Set-Content -Encoding utf8 (Join-Path $OutDir 'summary.txt')
    $lines | ForEach-Object { Write-Host "find-flaky: $_" }
}
finally {
    Pop-Location
}
