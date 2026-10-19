<#
.SYNOPSIS
  Screenshots of the Shell++ window (owner's stage 1, step 5.2).

.DESCRIPTION
  Builds shellpp with the `screenshots` feature and launches it ONCE for all
  sizes (`shellpp --screenshots <dir> --size 1920x1200,1280x800,960x600`).
  The app starts on a fresh demo profile (300 hosts, 5 groups) inside a
  temporary folder given as SHELLPP_DATA_DIR (with SHELLPP_TEST=1, so the
  permanent profile can never be used); the first-run, unlock and empty-state
  screens get their own empty profiles in the same folder. Every profile
  keeps its device key in the in-memory secret store, never in the OS
  keychain. The app renders OUR window offscreen through GPUI for the dark
  and the light theme, saves PNGs and quits. Nothing is read from the screen:
  other windows never appear.

  States per size (src/dev/screenshots.rs, `states_for`):
  - 1920x1200: root, group, host-panel;
  - 1280x800: root, group, host-panel, context-menu, search-address
    (root@10.0.0.5:2222, Connect active), search-results, search-no-results,
    three-selected, empty, list-view, new-host, quick-connect-toast,
    tag-filter, tag-chips (two tags on and the host panel open: the chips
    collapse into "+N"), show-more, first-run, unlock;
  - 960x600 (the minimum window size): root, host-panel, first-run;
  - any other size: root, group, host-panel.

  Output: target\ui-screens\<W>x<H>-<theme>-<state>.png (46 files for the
  default sizes). Exits with 1 when the build, the run or a file check fails.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -NoBuild
#>
[CmdletBinding()]
param(
    # Window content sizes in logical pixels; all of them in one app launch.
    [string[]] $Sizes = @('1920x1200', '1280x800', '960x600'),
    # Output folder; default <repo>\target\ui-screens.
    [string] $OutDir,
    # A launch that takes longer is killed and counts as a failure.
    [int] $TimeoutSeconds = 900,
    # Use the existing debug binary.
    [switch] $NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'target\ui-screens' }
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$exe = Join-Path $targetDir 'debug\shellpp.exe'

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

# Accept "-Sizes 1920x1200,1280x800" as well as separate values.
$Sizes = @($Sizes | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
foreach ($size in $Sizes) {
    if ($size -notmatch '^\d{3,4}x\d{3,4}$') { Fail "bad size '$size' (expected WxH, e.g. 1920x1200)" }
}
if (@($Sizes | Select-Object -Unique).Count -ne $Sizes.Count) { Fail 'a size is listed twice' }

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
$saved = @()
# A temporary data folder for this launch only; removed afterwards.
$dataDir = Join-Path ([System.IO.Path]::GetTempPath()) ('shellpp-ui-screens-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dataDir | Out-Null
$process = $null
try {
    $sizeList = $Sizes -join ','
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $exe
    $info.Arguments = "--screenshots `"$OutDir`" --size $sizeList"
    $info.UseShellExecute = $false
    $info.WorkingDirectory = $repo
    $info.EnvironmentVariables['SHELLPP_DATA_DIR'] = $dataDir
    $info.EnvironmentVariables['SHELLPP_TEST'] = '1'
    $info.RedirectStandardOutput = $true
    # Debug builds log every core event to stderr; only warnings, errors and
    # the tool's own messages are shown below.
    $info.RedirectStandardError = $true
    Write-Host "ui-screens: one launch for $sizeList"
    $process = [System.Diagnostics.Process]::Start($info)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $exited = $process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $exited) {
        $process.Kill()
    }
    $process.WaitForExit()
    foreach ($line in ($stdout.Result -split "`r?`n")) {
        if (-not $line) { continue }
        Write-Host "  $line"
        if ($line -match '^Saved (.+\.png) \((\d+)x(\d+)\)$') {
            $saved += , @($Matches[1], [int]$Matches[2], [int]$Matches[3])
        }
    }
    $stderr.Result -split "`r?`n" |
        ForEach-Object { $_ -replace "$([char]27)\[[0-9;]*m", '' } |
        Where-Object { $_ -and $_ -notmatch '^\d{4}-\d\d-\d\dT\S+\s+(TRACE|DEBUG|INFO)\s' } |
        ForEach-Object { Write-Host "  $_" }
    if (-not $exited) {
        $failures += "no result after $TimeoutSeconds s (killed)"
    }
    elseif ($process.ExitCode -ne 0) {
        $failures += "exit code $($process.ExitCode)"
    }
}
finally {
    if ($process -and -not $process.HasExited) { $process.Kill() }
    for ($attempt = 0; $attempt -lt 20 -and (Test-Path -LiteralPath $dataDir); $attempt++) {
        Remove-Item -LiteralPath $dataDir -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $dataDir) { Start-Sleep -Milliseconds 250 }
    }
    if (Test-Path -LiteralPath $dataDir) { $failures += "temporary data folder not removed: $dataDir" }
}

# Every file the app reported is there, is a PNG and has the reported size.
foreach ($entry in $saved) {
    $file = $entry[0]
    if (-not (Test-Path -LiteralPath $file)) {
        $failures += "missing $file"
        continue
    }
    $pixels = Get-PngSize $file
    if (-not $pixels) {
        $failures += "not a PNG: $file"
        continue
    }
    if ($pixels[0] -ne $entry[1] -or $pixels[1] -ne $entry[2]) {
        $failures += "size of $file is $($pixels[0])x$($pixels[1]), reported $($entry[1])x$($entry[2])"
    }
}
# One file per size, state and theme; each size has at least root and
# host-panel in both themes.
foreach ($size in $Sizes) {
    foreach ($theme in @('dark', 'light')) {
        foreach ($state in @('root', 'host-panel')) {
            $file = Join-Path $OutDir "$size-$theme-$state.png"
            if (-not (Test-Path -LiteralPath $file)) { $failures += "missing $file" }
        }
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Host "ui-screens: $failure" -ForegroundColor Red }
    exit 1
}
Write-Host "ui-screens: done, $($saved.Count) screenshots in $OutDir"
exit 0
