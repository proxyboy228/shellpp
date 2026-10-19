<#
.SYNOPSIS
  Idle memory of Shell++ with 300 hosts (SPEC.md, "Performance": under 150 MB RSS).

.DESCRIPTION
  Builds shellpp with the `dev-tools` feature (release profile by default),
  launches `shellpp --demo` (a temporary demo profile: 300 hosts, 5 groups),
  waits until the main window exists and then for -IdleSeconds more, and
  reads the working set (RSS) and private bytes of the process and of its
  child processes, if any. Then it closes the window (the app deletes its
  demo profile) and prints one line. On GitHub Actions the line is also
  added to the job summary (CI runs this as an informational step that
  never fails the build).

  The demo profile uses the in-memory secret store, never the OS keychain.

  The launch gets SHELLPP_TEST=1, SHELLPP_DATA_DIR and TEMP/TMP pointing at a
  fresh temporary folder (the demo profile is created there), so the
  permanent profile is never used; the folder is removed afterwards.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\measure-idle.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\measure-idle.ps1 -BuildProfile debug -NoBuild
#>
[CmdletBinding()]
param(
    # Cargo profile of the measured binary.
    [ValidateSet('release', 'debug')]
    [string] $BuildProfile = 'release',
    # Idle time after the main window appeared.
    [int] $IdleSeconds = 10,
    # Time allowed for the main window to appear.
    [int] $StartTimeoutSeconds = 90,
    # Use the existing binary.
    [switch] $NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$exe = Join-Path $targetDir "$BuildProfile\shellpp.exe"

function Fail([string] $message) {
    Write-Host "measure-idle: $message" -ForegroundColor Red
    exit 1
}

# Culture-invariant ("55.6 MB" also in a ru-RU console).
function MB([long] $bytes) { ($bytes / 1MB).ToString('N1', [System.Globalization.CultureInfo]::InvariantCulture) + ' MB' }

Push-Location $repo
try {
    if (-not $NoBuild) {
        $cargoArgs = @('build', '--locked', '-p', 'shellpp-desktop', '--features', 'dev-tools')
        if ($BuildProfile -eq 'release') { $cargoArgs += '--release' }
        Write-Host "measure-idle: cargo $($cargoArgs -join ' ')"
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
    }
}
finally {
    Pop-Location
}
if (-not (Test-Path -LiteralPath $exe)) { Fail "binary not found: $exe" }

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ('shellpp-measure-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempDir | Out-Null
$process = $null
$line = $null
$problem = $null
try {
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $exe
    $info.Arguments = '--demo --theme dark'
    $info.UseShellExecute = $false
    $info.WorkingDirectory = $repo
    $info.RedirectStandardOutput = $true
    $info.EnvironmentVariables['SHELLPP_TEST'] = '1'
    $info.EnvironmentVariables['SHELLPP_DATA_DIR'] = (Join-Path $tempDir 'data')
    $info.EnvironmentVariables['TEMP'] = $tempDir
    $info.EnvironmentVariables['TMP'] = $tempDir
    $process = [System.Diagnostics.Process]::Start($info)
    # The two lines --demo prints; read asynchronously so the pipe never fills.
    $null = $process.StandardOutput.ReadToEndAsync()

    $deadline = (Get-Date).AddSeconds($StartTimeoutSeconds)
    while ($true) {
        if ($process.HasExited) { $problem = "the app exited early (code $($process.ExitCode))"; break }
        $process.Refresh()
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { break }
        if ((Get-Date) -gt $deadline) { $problem = "no window after $StartTimeoutSeconds s"; break }
        Start-Sleep -Milliseconds 200
    }

    if (-not $problem) {
        Start-Sleep -Seconds $IdleSeconds
        if ($process.HasExited) {
            $problem = "the app exited while idle (code $($process.ExitCode))"
        }
        else {
            $process.Refresh()
            $workingSet = $process.WorkingSet64
            $private = $process.PrivateMemorySize64
            $peak = $process.PeakWorkingSet64
            $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($process.Id)" -ErrorAction SilentlyContinue)
            $childSet = 0L
            $childPrivate = 0L
            foreach ($child in $children) {
                $childProcess = Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue
                if ($childProcess) {
                    $childSet += $childProcess.WorkingSet64
                    $childPrivate += $childProcess.PrivateMemorySize64
                }
            }
            $line = ('shellpp idle ({0}, --demo 300 hosts, {1} s after the window appeared): RSS (working set) {2}, private bytes {3}; peak RSS {4}; child processes {5} (RSS {6}, private bytes {7})' -f `
                    $BuildProfile, $IdleSeconds, (MB ($workingSet + $childSet)), (MB ($private + $childPrivate)), (MB $peak), $children.Count, (MB $childSet), (MB $childPrivate))
        }
    }
}
finally {
    if ($process -and -not $process.HasExited) {
        # A normal close: the app quits and deletes its demo profile.
        $null = $process.CloseMainWindow()
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

if ($problem) { Fail $problem }
if (Test-Path -LiteralPath $tempDir) { Write-Host "measure-idle: temporary folder not removed: $tempDir" -ForegroundColor Yellow }
Write-Output $line
if ($env:GITHUB_STEP_SUMMARY) {
    Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Encoding utf8 -Value "**Idle memory (informational):** $line"
}
exit 0
