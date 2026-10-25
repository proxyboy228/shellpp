<#
.SYNOPSIS
  Tests Invoke-Logged of scripts\process.ps1, the runner of verify.ps1 and
  verify-linux.ps1. Exits with 1 when a check fails.

.DESCRIPTION
  - exit codes pass through (0 and 3), and the output lands in the logs;
  - an argument with a space and a path with a space arrive whole;
  - a program that does not start gives -1 and says why in the error log;
  - a program that leaves a process behind (as MSVC's linker leaves
    vctip.exe) is done when it exits: first Start-Process -Wait is shown to
    wait for the left-behind process, then Invoke-Logged is shown not to.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\test-process.ps1
#>

$ErrorActionPreference = 'Continue'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'process.ps1')

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("shellpp process test " + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$powershell = Join-Path $PSHOME 'powershell.exe'
$failures = 0

function Check([string] $name, [bool] $ok, [string] $detail) {
    if ($ok) {
        Write-Host "ok      $name" -ForegroundColor Green
    } else {
        Write-Host "FAILED  $name ($detail)" -ForegroundColor Red
        $script:failures += 1
    }
}

function Encoded([string] $command) {
    [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($command))
}

try {
    # Exit codes and output.
    $log = Join-Path $work 'exit.log'
    $code = Invoke-Logged $powershell @('-NoProfile', '-EncodedCommand', (Encoded "Write-Output 'out'; [Console]::Error.WriteLine('err'); exit 0")) $log "$log.err"
    Check 'exit code 0' ($code -eq 0) "got $code"
    Check 'stdout in the log' ("$(Get-Content -LiteralPath $log -Raw)".Trim() -eq 'out') 'no output'
    # powershell.exe adds its CLIXML progress records to a redirected stderr.
    Check 'stderr in the error log' (@(Get-Content -LiteralPath "$log.err") -contains 'err') 'no output'
    $code = Invoke-Logged $powershell @('-NoProfile', '-EncodedCommand', (Encoded 'exit 3')) $log "$log.err"
    Check 'exit code 3' ($code -eq 3) "got $code"

    # Arguments with spaces, and a script in a folder whose name has one.
    $argsScript = Join-Path $work 'print args.ps1'
    Set-Content -LiteralPath $argsScript -Value 'Write-Output $args.Count; $args | ForEach-Object { Write-Output $_ }' -Encoding ASCII
    $log = Join-Path $work 'args.log'
    $code = Invoke-Logged $powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $argsScript, 'one two', 'three') $log "$log.err"
    $lines = @(Get-Content -LiteralPath $log)
    Check 'arguments with spaces' ($code -eq 0 -and $lines.Count -eq 3 -and $lines[0] -eq '2' -and $lines[1] -eq 'one two' -and $lines[2] -eq 'three') "exit $code, output: $($lines -join ' | ')"

    # A program that is not there.
    $log = Join-Path $work 'missing.log'
    $code = Invoke-Logged (Join-Path $work 'no-such-program.exe') @() $log "$log.err"
    Check 'a program that does not start gives -1' ($code -eq -1) "got $code"
    Check 'and says why' ((Test-Path -LiteralPath "$log.err") -and ("$(Get-Content -LiteralPath "$log.err" -Raw)" -match 'could not start')) 'no message'

    # A process left behind for 20 s, found again by a marker in its
    # command line.
    $marker = 'shellpp-left-behind-' + [guid]::NewGuid().ToString('N')
    $child = Encoded "Start-Sleep -Seconds 20 # $marker"
    $parent = Encoded "Start-Process -FilePath '$powershell' -ArgumentList '-NoProfile','-EncodedCommand','$child' -NoNewWindow; exit 0"
    $kill = {
        Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" |
            Where-Object { $_.CommandLine -and $_.CommandLine.Contains($child) } |
            ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }

    $log = Join-Path $work 'wait.log'
    $started = Get-Date
    $old = Start-Process -FilePath $powershell -ArgumentList @('-NoProfile', '-EncodedCommand', $parent) -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $log -RedirectStandardError "$log.err"
    $seconds = ((Get-Date) - $started).TotalSeconds
    Check 'Start-Process -Wait waits for the process left behind (the case is real)' ($seconds -ge 15) ("{0:0.0} s" -f $seconds)
    & $kill
    $null = $old

    $log = Join-Path $work 'logged.log'
    $started = Get-Date
    $code = Invoke-Logged $powershell @('-NoProfile', '-EncodedCommand', $parent) $log "$log.err"
    $seconds = ((Get-Date) - $started).TotalSeconds
    $stillThere = @(Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" |
            Where-Object { $_.CommandLine -and $_.CommandLine.Contains($child) }).Count -gt 0
    Check 'Invoke-Logged returns when the program exits' ($code -eq 0 -and $seconds -lt 10) ("exit $code after {0:0.0} s" -f $seconds)
    Check 'while the process it left is still running' $stillThere 'it was gone already'
    & $kill
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

if ($failures -gt 0) {
    Write-Host "test-process: $failures check(s) failed" -ForegroundColor Red
    exit 1
}
Write-Host 'test-process: every check passed' -ForegroundColor Green
exit 0
