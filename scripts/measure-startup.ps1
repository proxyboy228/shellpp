<#
.SYNOPSIS
  Cold start of Shell++: from launch to the first frame of the main window
  (SPEC.md, "Performance": under 1 s).

.DESCRIPTION
  Builds shellpp with the `dev-tools` feature (release profile by default)
  and launches it -Runs times per scenario with SHELLPP_BENCH_STARTUP=1:
  once the main window has presented its first frame, the app prints
  `bench: first frame <ms> ms` (time since the start of `main`) and quits
  (src/dev/startup.rs). The script also times each launch from outside,
  from process creation to that line (loading the binary included).

  Scenarios (-Scenario):
  - first-run: an empty profile, the window shows the first-run screen;
  - demo: `--demo`, which first creates a temporary profile with 300 hosts
    and 5 groups (a new vault: one Argon2id derivation, 305 encrypted
    records) and starts unlocked; an upper bound that includes the data
    generation.
  A profile that already holds 300 hosts and opens with the device key is
  not measured: that needs the OS keychain, which test runs never use.

  Every launch gets SHELLPP_TEST=1 and SHELLPP_DATA_DIR, TEMP, TMP, APPDATA
  and LOCALAPPDATA in a fresh temporary folder, removed afterwards. The OS
  file cache is warm after the first launch; the table shows the first
  launch, the median and the maximum.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\measure-startup.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\measure-startup.ps1 -NoBuild -Runs 3 -Scenario first-run
#>
[CmdletBinding()]
param(
    [ValidateSet('release', 'debug')]
    [string] $BuildProfile = 'release',
    [ValidateRange(1, 50)]
    [int] $Runs = 5,
    [string[]] $Scenario = @('first-run', 'demo'),
    # Time allowed for one launch.
    [int] $TimeoutSeconds = 60,
    [switch] $NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$exe = Join-Path $targetDir "$BuildProfile\shellpp.exe"
$Scenario = @($Scenario | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
foreach ($name in $Scenario) {
    if ($name -notin @('first-run', 'demo')) { throw "unknown scenario '$name'" }
}

# Culture-invariant numbers ("1234 ms" also in a ru-RU console).
$invariant = [System.Globalization.CultureInfo]::InvariantCulture

function Fail([string] $message) {
    Write-Host "measure-startup: $message" -ForegroundColor Red
    exit 1
}


Push-Location $repo
try {
    if (-not $NoBuild) {
        $cargoArgs = @('build', '--locked', '-p', 'shellpp-desktop', '--features', 'dev-tools')
        if ($BuildProfile -eq 'release') { $cargoArgs += '--release' }
        Write-Host "measure-startup: cargo $($cargoArgs -join ' ')"
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
    }
}
finally {
    Pop-Location
}
if (-not (Test-Path -LiteralPath $exe)) { Fail "binary not found: $exe" }

# One launch; returns @(inside ms, outside ms).
function Measure-Launch([string] $scenario) {
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ('shellpp-startup-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tempDir | Out-Null
    $process = $null
    try {
        $info = New-Object System.Diagnostics.ProcessStartInfo
        $info.FileName = $exe
        $info.Arguments = if ($scenario -eq 'demo') { '--demo --theme dark' } else { '' }
        $info.UseShellExecute = $false
        $info.WorkingDirectory = $repo
        $info.RedirectStandardOutput = $true
        $info.EnvironmentVariables['SHELLPP_TEST'] = '1'
        $info.EnvironmentVariables['SHELLPP_BENCH_STARTUP'] = '1'
        $info.EnvironmentVariables['SHELLPP_DATA_DIR'] = (Join-Path $tempDir 'data')
        foreach ($name in @('TEMP', 'TMP', 'APPDATA', 'LOCALAPPDATA')) {
            $info.EnvironmentVariables[$name] = $tempDir
        }
        $watch = [System.Diagnostics.Stopwatch]::StartNew()
        $process = [System.Diagnostics.Process]::Start($info)
        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        while ($true) {
            $read = $process.StandardOutput.ReadLineAsync()
            $left = [int][Math]::Max(1, ($deadline - (Get-Date)).TotalMilliseconds)
            if (-not $read.Wait($left)) { throw "no first frame after $TimeoutSeconds s" }
            $line = $read.Result
            if ($null -eq $line) { throw "the app exited without a first frame (code $($process.ExitCode))" }
            if ($line -match '^bench: first frame (\d+) ms$') {
                $outside = $watch.Elapsed.TotalMilliseconds
                return @([double]$Matches[1], $outside)
            }
        }
    }
    finally {
        if ($process) {
            if (-not $process.WaitForExit(15000)) {
                $process.Kill()
                $process.WaitForExit()
            }
        }
        for ($attempt = 0; $attempt -lt 20 -and (Test-Path -LiteralPath $tempDir); $attempt++) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $tempDir) { Start-Sleep -Milliseconds 250 }
        }
    }
}

function Median([double[]] $values) {
    $sorted = @($values | Sort-Object)
    $middle = [int][Math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$middle] }
    return ($sorted[$middle - 1] + $sorted[$middle]) / 2
}

$rows = @()
foreach ($name in $Scenario) {
    $inside = @()
    $outside = @()
    for ($run = 1; $run -le $Runs; $run++) {
        try {
            $result = Measure-Launch $name
        }
        catch {
            Fail "$name, run ${run}: $($_.Exception.Message)"
        }
        $inside += $result[0]
        $outside += $result[1]
        Write-Host ("measure-startup: {0} run {1}: {2} ms in the app, {3} ms from process start" -f $name, $run, ([Math]::Round($result[0])).ToString($invariant), ([Math]::Round($result[1])).ToString($invariant))
    }
    $rows += [pscustomobject]@{
        Scenario       = $name
        'First (ms)'   = [Math]::Round($outside[0])
        'Median (ms)'  = [Math]::Round((Median $outside))
        'Max (ms)'     = [Math]::Round(($outside | Measure-Object -Maximum).Maximum)
        'In app, median (ms)' = [Math]::Round((Median $inside))
    }
}

$table = $rows | Format-Table -AutoSize | Out-String
Write-Output ("shellpp cold start ({0}, {1} runs; from process start to the first presented frame of the main window):" -f $BuildProfile, $Runs)
Write-Output $table.TrimEnd()
if ($env:GITHUB_STEP_SUMMARY) {
    Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Encoding utf8 -Value ("**Cold start:**`n" + '```' + "`n" + $table.TrimEnd() + "`n" + '```')
}
exit 0
