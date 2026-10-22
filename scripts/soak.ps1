<#
.SYNOPSIS
  Long run of Shell++ under load (run 6, plan section 8): what grows.

.DESCRIPTION
  Builds the release binary with the `dev-tools` feature, starts the
  OpenSSH test server in Docker (tests/openssh/start.sh, its lifetime longer
  than the run) and runs `shellpp --demo --soak <minutes>` against it: ten
  session tabs with output (seq in a loop, htop), a local and a dynamic
  forwarding rule with HTTP downloads through both, and SFTP uploads and
  downloads in a loop (apps/desktop/src/dev/soak.rs). With -Server russh
  (or auto without Docker) the russh test server instead: seq typed again
  and again, the forwards to an HTTP server of the run, no SFTP and no htop
  (that server has neither).

  Every -SampleMinutes, from the moment the load runs: the app's working
  set and private bytes, its handle and thread counts, whether its window
  is minimized (D53: a minimized window draws no frames), and the run's own
  counts, into target\soak\soak.csv. At the end: target\soak\soak.png (a
  chart of the four values) and target\soak\soak-summary.txt (the growth of
  each value from the 15-minute sample to the last one; above 10 % is
  marked).

  The container is removed at the end. Docker Desktop must be running.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\soak.ps1 -Minutes 120
#>
param(
    # Length of the load.
    [ValidateRange(1, 600)]
    [int] $Minutes = 120,
    # A sample every so many minutes (and at the start and the end).
    [ValidateRange(1, 60)]
    [int] $SampleMinutes = 5,
    # Output folder; default <repo>\target\soak.
    [string] $OutDir,
    # The SSH server: OpenSSH in Docker, the russh test server, or OpenSSH
    # when Docker runs.
    [ValidateSet('auto', 'openssh', 'russh')]
    [string] $Server = 'auto',
    # Use the existing release binaries.
    [switch] $NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$exe = Join-Path $targetDir 'release\shellpp.exe'
$sshdExe = Join-Path $targetDir 'release\shellpp-test-sshd.exe'
if (-not $OutDir) { $OutDir = Join-Path $targetDir 'soak' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$invariant = [System.Globalization.CultureInfo]::InvariantCulture

function Fail([string] $message) {
    Write-Host "soak: $message" -ForegroundColor Red
    exit 1
}

function Find-GitBash {
    if ($env:SHELLPP_BASH) { return $env:SHELLPP_BASH }
    $git = Get-Command git -ErrorAction SilentlyContinue
    if (-not $git) { return $null }
    $execPath = (& git --exec-path).Trim()
    $root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $execPath))
    $bash = Join-Path $root 'bin\bash.exe'
    if (Test-Path -LiteralPath $bash) { return $bash }
    return $null
}

Add-Type -Namespace SoakWin32 -Name Window -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool IsIconic(System.IntPtr hWnd);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool ShowWindow(System.IntPtr hWnd, int nCmdShow);
'@

if (-not $NoBuild) {
    Push-Location $repo
    try {
        $cargoArgs = @('build', '--locked', '--release', '-p', 'shellpp-desktop', '--features', 'dev-tools', '-p', 'shellpp-test-sshd')
        Write-Host "soak: cargo $($cargoArgs -join ' ')"
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
    }
    finally { Pop-Location }
}
if (-not (Test-Path -LiteralPath $exe)) { Fail "no $exe" }

$dockerProblem = $null
$docker = Get-Command docker -ErrorAction SilentlyContinue
if (-not $docker) { $dockerProblem = 'Docker is not installed' }
else {
    # Windows PowerShell turns a native command's stderr into an error
    # under 'Stop'.
    $ErrorActionPreference = 'Continue'
    $os = & docker info --format '{{.OSType}}' 2>$null
    $dockerExit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($dockerExit -ne 0 -or "$os".Trim() -ne 'linux') { $dockerProblem = 'Docker (Linux containers) is not running' }
}
$bash = Find-GitBash
if (-not $bash -and -not $dockerProblem) { $dockerProblem = "Git's bash was not found" }
if ($Server -eq 'openssh' -and $dockerProblem) { Fail "OpenSSH needs Docker and Git's bash: $dockerProblem" }
$useOpenSsh = ($Server -ne 'russh') -and -not $dockerProblem

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("shellpp-soak-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempRoot | Out-Null
$serverDir = Join-Path $tempRoot 'openssh'
New-Item -ItemType Directory -Path $serverDir | Out-Null
$containerFile = Join-Path $serverDir 'container'

$csv = Join-Path $OutDir 'soak.csv'
$png = Join-Path $OutDir 'soak.png'
$summary = Join-Path $OutDir 'soak-summary.txt'
$samples = New-Object System.Collections.Generic.List[object]
$problem = $null
$process = $null
try {
    $serverArgs = ''
    if ($useOpenSsh) {
        # The container outlives the run by an hour (tests/openssh/start.sh).
        $env:SHELLPP_OPENSSH_LIFETIME = "$($Minutes * 60 + 3600)"
        $serverJson = (& $bash (Join-Path $repo 'tests\openssh\start.sh') $serverDir | Select-Object -Last 1)
        if ($LASTEXITCODE -ne 0 -or -not $serverJson) { Fail "tests/openssh/start.sh failed (exit $LASTEXITCODE)" }
        $serverJson = $serverJson.Trim()
        $serverArgs = " --test-server `"$serverJson`""
        Write-Host "soak: server: OpenSSH in Docker, lifetime $($env:SHELLPP_OPENSSH_LIFETIME) s"
    }
    else {
        $note = if ($dockerProblem) { " ($dockerProblem)" } else { '' }
        Write-Host "soak: server: russh test server, no SFTP and no htop$note"
    }

    $runDir = Join-Path $tempRoot 'app'
    New-Item -ItemType Directory -Path $runDir, (Join-Path $runDir 'appdata'), (Join-Path $runDir 'localappdata') | Out-Null
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $exe
    $info.Arguments = "--demo --theme dark --soak $Minutes$serverArgs"
    $info.UseShellExecute = $false
    $info.WorkingDirectory = $repo
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.RedirectStandardInput = $true
    $info.EnvironmentVariables['SHELLPP_TEST'] = '1'
    $info.EnvironmentVariables['SHELLPP_DATA_DIR'] = (Join-Path $runDir 'data')
    $info.EnvironmentVariables['TEMP'] = $runDir
    $info.EnvironmentVariables['TMP'] = $runDir
    $info.EnvironmentVariables['APPDATA'] = (Join-Path $runDir 'appdata')
    $info.EnvironmentVariables['LOCALAPPDATA'] = (Join-Path $runDir 'localappdata')
    $info.EnvironmentVariables['SHELLPP_TEST_SSHD_EXE'] = $sshdExe
    $process = [System.Diagnostics.Process]::Start($info)
    $stderr = $process.StandardError.ReadToEndAsync()
    Write-Host "soak: app pid $($process.Id)"

    $counts = 'http 0 socks 0 http-failed 0 sftp 0 sftp-failed 0'
    $loadStart = $null
    $nextSample = $null
    $nextCheck = Get-Date
    $restored = 0
    $deadline = (Get-Date).AddMinutes($Minutes + 20)
    $pending = $process.StandardOutput.ReadLineAsync()
    $ready = $false

    function Add-Sample([double] $minute) {
        $process.Refresh()
        $minimized = $false
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { $minimized = [SoakWin32.Window]::IsIconic($process.MainWindowHandle) }
        $values = @{}
        if ($counts -match 'http (\d+) socks (\d+) http-failed (\d+) sftp (\d+) sftp-failed (\d+)') {
            $values = @{ Http = [int]$Matches[1]; Socks = [int]$Matches[2]; HttpFailed = [int]$Matches[3]; Sftp = [int]$Matches[4]; SftpFailed = [int]$Matches[5] }
        }
        $row = [pscustomobject]@{
            Minute       = [math]::Round($minute, 1)
            WorkingSetMB = [math]::Round($process.WorkingSet64 / 1MB, 1)
            PrivateMB    = [math]::Round($process.PrivateMemorySize64 / 1MB, 1)
            Handles      = $process.HandleCount
            Threads      = $process.Threads.Count
            Minimized    = $minimized
            Http         = $values['Http']
            Socks        = $values['Socks']
            HttpFailed   = $values['HttpFailed']
            Sftp         = $values['Sftp']
            SftpFailed   = $values['SftpFailed']
        }
        $samples.Add($row)
        $samples | Export-Csv -LiteralPath $csv -NoTypeInformation -Encoding UTF8
        Write-Host ("soak: sample {0} min: working set {1} MB, private {2} MB, handles {3}, threads {4}{5}" -f
            $row.Minute, $row.WorkingSetMB, $row.PrivateMB, $row.Handles, $row.Threads, $(if ($minimized) { ', MINIMIZED' } else { '' }))
    }

    while ($true) {
        if ($pending.Wait(1000)) {
            $line = $pending.Result
            if ($null -eq $line) { $problem = "the app ended (code $(if ($process.HasExited) { $process.ExitCode } else { '?' }))"; break }
            if ($line.StartsWith('bench: ')) { Write-Host "  $line" }
            if ($line -match '^bench: soak minute \d+ (.+)$') { $counts = $Matches[1] }
            if ($line -eq 'bench: soak load') {
                $loadStart = Get-Date
                $nextSample = $loadStart
            }
            if ($line.StartsWith('bench: failed')) { $problem = $line; break }
            if ($line -eq 'bench: ready') { $ready = $true }
            $pending = $process.StandardOutput.ReadLineAsync()
        }
        # A minimized window draws no frames (D53): shown again without
        # taking the focus (SW_SHOWNOACTIVATE), and counted.
        if ($loadStart -and (Get-Date) -ge $nextCheck) {
            $process.Refresh()
            if ($process.MainWindowHandle -ne [IntPtr]::Zero -and [SoakWin32.Window]::IsIconic($process.MainWindowHandle)) {
                $null = [SoakWin32.Window]::ShowWindow($process.MainWindowHandle, 4)
                $restored++
                Write-Host "soak: the window was minimized; shown again ($restored)"
            }
            $nextCheck = (Get-Date).AddSeconds(30)
        }
        if ($loadStart -and (Get-Date) -ge $nextSample -and -not $ready) {
            Add-Sample ((Get-Date) - $loadStart).TotalMinutes
            $nextSample = $nextSample.AddMinutes($SampleMinutes)
        }
        if ($ready) {
            Add-Sample ((Get-Date) - $loadStart).TotalMinutes
            break
        }
        if ($process.HasExited) { $problem = "the app exited (code $($process.ExitCode))"; break }
        if ((Get-Date) -gt $deadline) { $problem = 'no end of the run in time'; break }
    }
}
finally {
    if ($process -and -not $process.HasExited) {
        try {
            $process.StandardInput.WriteLine('quit')
            $process.StandardInput.Flush()
        }
        catch { }
        if (-not $process.WaitForExit(30000)) {
            $null = $process.CloseMainWindow()
            if (-not $process.WaitForExit(20000)) { $process.Kill(); $process.WaitForExit() }
        }
    }
    if ($problem -and $process) {
        $stderr.Result -split "`r?`n" |
            ForEach-Object { $_ -replace "$([char]27)\[[0-9;]*m", '' } |
            Where-Object { $_ -and $_ -notmatch '^\d{4}-\d\d-\d\dT\S+\s+(TRACE|DEBUG|INFO)\s' } |
            Select-Object -Last 20 |
            ForEach-Object { Write-Host "    $_" }
    }
    if (Test-Path -LiteralPath $containerFile) {
        $container = (Get-Content -LiteralPath $containerFile -Raw).Trim()
        if ($container) { & docker rm -f $container 2>$null | Out-Null }
    }
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}

if ($samples.Count -eq 0) { Fail "no sample$(if ($problem) { ": $problem" })" }

# The growth of each value from the 15-minute sample to the last one.
$base = $samples | Where-Object { $_.Minute -ge 15 } | Select-Object -First 1
$last = $samples[$samples.Count - 1]
$lines = New-Object System.Collections.Generic.List[string]
$serverName = if ($useOpenSsh) { 'OpenSSH in Docker' } else { 'russh test server (no SFTP, no htop)' }
$lines.Add("Soak run: $Minutes min on $serverName, a sample every $SampleMinutes min, $($samples.Count) samples.")
$lines.Add("Last counts: $counts")
$lines.Add("The window was found minimized and shown again $restored times.")
$lines.Add('')
$lines.Add('| Minute | Working set, MB | Private, MB | Handles | Threads | Minimized |')
$lines.Add('|---|---|---|---|---|---|')
foreach ($s in $samples) { $lines.Add("| $($s.Minute) | $($s.WorkingSetMB) | $($s.PrivateMB) | $($s.Handles) | $($s.Threads) | $($s.Minimized) |") }
$lines.Add('')
$grew = @()
if ($base -and $base -ne $last) {
    $lines.Add("Growth from minute $($base.Minute) to minute $($last.Minute):")
    foreach ($name in @('WorkingSetMB', 'PrivateMB', 'Handles', 'Threads')) {
        $from = [double]$base.$name
        $to = [double]$last.$name
        $percent = if ($from -gt 0) { ($to - $from) / $from * 100 } else { 0 }
        $mark = if ($percent -gt 10) { ' ABOVE 10 %' } else { '' }
        if ($percent -gt 10) { $grew += $name }
        $lines.Add(("- {0}: {1} -> {2} ({3:+0.0;-0.0;0.0} %){4}" -f $name, $from, $to, $percent, $mark))
    }
}
else {
    $lines.Add('No sample at 15 minutes or later: no growth to compare.')
}
if ($problem) { $lines.Add(''); $lines.Add("Problem: $problem") }
$lines | Set-Content -LiteralPath $summary -Encoding UTF8
$lines | ForEach-Object { Write-Host $_ }

# The chart: one panel per value, minutes across.
Add-Type -AssemblyName System.Drawing
$width = 1200; $height = 800
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$graphics.Clear([System.Drawing.Color]::White)
$font = New-Object System.Drawing.Font 'Segoe UI', 10
$titleFont = New-Object System.Drawing.Font 'Segoe UI', 12, ([System.Drawing.FontStyle]::Bold)
$axis = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(160, 160, 160)), 1
$grid = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(230, 230, 230)), 1
$text = [System.Drawing.Brushes]::Black
$panels = @(
    @{ Name = 'WorkingSetMB'; Title = 'Working set, MB'; Color = [System.Drawing.Color]::FromArgb(37, 99, 235) },
    @{ Name = 'PrivateMB'; Title = 'Private bytes, MB'; Color = [System.Drawing.Color]::FromArgb(22, 163, 74) },
    @{ Name = 'Handles'; Title = 'Handles'; Color = [System.Drawing.Color]::FromArgb(217, 119, 6) },
    @{ Name = 'Threads'; Title = 'Threads'; Color = [System.Drawing.Color]::FromArgb(147, 51, 234) }
)
$maxMinute = [math]::Max(1, [double]$last.Minute)
for ($i = 0; $i -lt $panels.Count; $i++) {
    $panel = $panels[$i]
    $left = 70 + ($i % 2) * 590; $top = 40 + [math]::Floor($i / 2) * 380
    $plotW = 500; $plotH = 300
    $values = @($samples | ForEach-Object { [double]$_.($panel.Name) })
    $min = ($values | Measure-Object -Minimum).Minimum
    $max = ($values | Measure-Object -Maximum).Maximum
    if ($max -eq $min) { $max = $min + 1 }
    $low = [math]::Max(0, $min - ($max - $min) * 0.2); $high = $max + ($max - $min) * 0.2
    $graphics.DrawString($panel.Title, $titleFont, $text, $left, $top - 30)
    for ($g = 0; $g -le 4; $g++) {
        $y = $top + $plotH - $plotH * $g / 4
        $graphics.DrawLine($grid, $left, $y, $left + $plotW, $y)
        $label = ($low + ($high - $low) * $g / 4).ToString('0.#', $invariant)
        $graphics.DrawString($label, $font, $text, $left - 60, $y - 8)
    }
    $graphics.DrawRectangle($axis, $left, $top, $plotW, $plotH)
    $graphics.DrawString("0", $font, $text, $left - 4, $top + $plotH + 4)
    $graphics.DrawString("$([math]::Round($maxMinute)) min", $font, $text, $left + $plotW - 40, $top + $plotH + 4)
    if ($base) {
        $x15 = $left + $plotW * [double]$base.Minute / $maxMinute
        $graphics.DrawLine((New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(200, 200, 200)), 1), $x15, $top, $x15, $top + $plotH)
    }
    $pen = New-Object System.Drawing.Pen $panel.Color, 2
    $points = @()
    foreach ($s in $samples) {
        $x = $left + $plotW * [double]$s.Minute / $maxMinute
        $y = $top + $plotH - $plotH * ([double]$s.($panel.Name) - $low) / ($high - $low)
        $points += New-Object System.Drawing.PointF ([float]$x), ([float]$y)
    }
    if ($points.Count -ge 2) { $graphics.DrawLines($pen, [System.Drawing.PointF[]]$points) }
    foreach ($point in $points) { $graphics.FillEllipse((New-Object System.Drawing.SolidBrush $panel.Color), $point.X - 3, $point.Y - 3, 6, 6) }
}
$bitmap.Save($png, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose(); $bitmap.Dispose()
Write-Host "soak: $csv, $png, $summary"

if ($problem) { Fail $problem }
if ($grew.Count -gt 0) {
    Write-Host "soak: grew above 10 %: $($grew -join ', ')" -ForegroundColor Yellow
    exit 2
}
exit 0
