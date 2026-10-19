<#
.SYNOPSIS
  Screenshots of the Shell++ window on demo data (owner's step 4).

.DESCRIPTION
  Builds shellpp with the `screenshots` feature and runs it once per size
  (`shellpp --screenshots <dir> --size WxH`). Each run starts the app on a
  fresh demo profile (300 hosts, 5 groups) inside a temporary folder given as
  SHELLPP_DATA_DIR (with SHELLPP_TEST=1, so the permanent profile can never be
  used), renders OUR window offscreen through GPUI for the dark and the light
  theme and three states (root, inside a group, host panel open), saves PNGs
  and quits. Nothing is read from the screen: other windows never appear.

  Output: target\ui-screens\<W>x<H>-<theme>-<state>.png (12 files for the two
  default sizes). Exits with 1 when the build, a run or a file check fails.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -NoBuild
#>
[CmdletBinding()]
param(
    # Window content sizes in logical pixels; one app launch per size.
    [string[]] $Sizes = @('1920x1200', '1280x800'),
    # Output folder; default <repo>\target\ui-screens.
    [string] $OutDir,
    # A launch that takes longer is killed and counts as a failure.
    [int] $TimeoutSeconds = 240,
    # Use the existing debug binary.
    [switch] $NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'target\ui-screens' }
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$exe = Join-Path $targetDir 'debug\shellpp.exe'
$themes = @('dark', 'light')
$states = @('root', 'group', 'host-panel')

function Fail([string] $message) {
    Write-Host "ui-screens: $message" -ForegroundColor Red
    exit 1
}

# PNG width and height from the IHDR chunk (big-endian at offsets 16 and 20).
function Get-PngSize([string] $path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    if ($bytes.Length -lt 24 -or $bytes[1] -ne 0x50 -or $bytes[2] -ne 0x4E -or $bytes[3] -ne 0x47) {
        return $null
    }
    $width = ([int]$bytes[16] -shl 24) -bor ([int]$bytes[17] -shl 16) -bor ([int]$bytes[18] -shl 8) -bor [int]$bytes[19]
    $height = ([int]$bytes[20] -shl 24) -bor ([int]$bytes[21] -shl 16) -bor ([int]$bytes[22] -shl 8) -bor [int]$bytes[23]
    return @($width, $height)
}

foreach ($size in $Sizes) {
    if ($size -notmatch '^\d{3,4}x\d{3,4}$') { Fail "bad size '$size' (expected WxH, e.g. 1920x1200)" }
}

Push-Location $repo
try {
    if (-not $NoBuild) {
        Write-Host 'ui-screens: building shellpp (features: screenshots)...'
        & cargo build -p shellpp-desktop --features screenshots
        if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
    }
}
finally {
    Pop-Location
}
if (-not (Test-Path -LiteralPath $exe)) { Fail "binary not found: $exe" }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
Get-ChildItem -LiteralPath $OutDir -Filter '*.png' -File | Remove-Item -Force

$failures = @()
foreach ($size in $Sizes) {
    # A temporary data folder for this launch only; removed afterwards.
    $dataDir = Join-Path ([System.IO.Path]::GetTempPath()) ('shellpp-ui-screens-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $dataDir | Out-Null
    $process = $null
    try {
        $info = New-Object System.Diagnostics.ProcessStartInfo
        $info.FileName = $exe
        $info.Arguments = "--screenshots `"$OutDir`" --size $size"
        $info.UseShellExecute = $false
        $info.WorkingDirectory = $repo
        $info.EnvironmentVariables['SHELLPP_DATA_DIR'] = $dataDir
        $info.EnvironmentVariables['SHELLPP_TEST'] = '1'
        # Debug builds log every core event to stderr; only warnings,
        # errors and the tool's own messages are shown below.
        $info.RedirectStandardError = $true
        Write-Host "ui-screens: launch $size"
        $process = [System.Diagnostics.Process]::Start($info)
        $stderr = $process.StandardError.ReadToEndAsync()
        $exited = $process.WaitForExit($TimeoutSeconds * 1000)
        if (-not $exited) {
            $process.Kill()
        }
        $process.WaitForExit()
        $stderr.Result -split "`r?`n" |
            ForEach-Object { $_ -replace "$([char]27)\[[0-9;]*m", '' } |
            Where-Object { $_ -and $_ -notmatch '^\d{4}-\d\d-\d\dT\S+\s+(TRACE|DEBUG|INFO)\s' } |
            ForEach-Object { Write-Host "  $_" }
        if (-not $exited) {
            $failures += "${size}: no result after $TimeoutSeconds s (killed)"
        }
        elseif ($process.ExitCode -ne 0) {
            $failures += "${size}: exit code $($process.ExitCode)"
        }
    }
    finally {
        if ($process -and -not $process.HasExited) { $process.Kill() }
        for ($attempt = 0; $attempt -lt 20 -and (Test-Path -LiteralPath $dataDir); $attempt++) {
            Remove-Item -LiteralPath $dataDir -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $dataDir) { Start-Sleep -Milliseconds 250 }
        }
        if (Test-Path -LiteralPath $dataDir) { $failures += "${size}: temporary data folder not removed: $dataDir" }
    }

    foreach ($theme in $themes) {
        foreach ($state in $states) {
            $file = Join-Path $OutDir "$size-$theme-$state.png"
            if (-not (Test-Path -LiteralPath $file)) {
                $failures += "missing $file"
                continue
            }
            $pixels = Get-PngSize $file
            if (-not $pixels) {
                $failures += "not a PNG: $file"
                continue
            }
            Write-Host ("  {0}  {1}x{2}" -f $file, $pixels[0], $pixels[1])
        }
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Host "ui-screens: $failure" -ForegroundColor Red }
    exit 1
}
Write-Host "ui-screens: done, screenshots in $OutDir"
exit 0
