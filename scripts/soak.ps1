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

  Every -SampleMinutes (or -SampleSeconds), from the moment the load runs:
  the app's working set and private bytes, its handle and thread counts,
  whether its window is minimized (D53: a minimized window draws no
  frames), the Rust heap the app counts (live bytes and their peak since
  the sample before, `bench: soak heap`, run 12) and the run's own counts,
  into target\soak\soak.csv. At the end: target\soak\soak.png (a chart of
  four values) and target\soak\soak-summary.txt.

  The summary compares windows, not single samples (run 12: under the SFTP
  load private bytes swing by up to 110 MB for seconds, and one sample
  that met a swing read as growth): the first window after the warm-up
  (-WarmupMinutes) with the last window of the run, each -WindowMinutes
  long. Memory (working set, private bytes, heap) is compared by the lowest
  value of each window, handles and threads by the median. Growth above
  10 % is marked and the script exits with 2; a window with fewer than 10
  samples gives no verdict and the script exits with 3. The heap columns
  are the app's last heap line (at most 5 s old).

  -Analyze <soak.csv> only writes the summary of an earlier run's samples.

  -Load passes SHELLPP_SOAK_LOAD to the app: tabs, forward or sftp
  (OpenSSH) runs that part of the load only (the forwarding rules exist in
  every run; they carry downloads with the whole load and with forward);
  direct (russh only) downloads without the forwards.

  The container is removed at the end. Docker Desktop must be running.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\soak.ps1 -Minutes 120
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\soak.ps1 -Minutes 30 -SampleSeconds 5 -Load sftp
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\soak.ps1 -Analyze docs\overnight\run11-soak\soak.csv
#>
param(
    # Length of the load.
    [ValidateRange(1, 600)]
    [int] $Minutes = 120,
    # A sample every so many minutes (and at the start and the end).
    [ValidateRange(1, 60)]
    [int] $SampleMinutes = 5,
    # A sample every so many seconds (0: every -SampleMinutes instead). The
    # summary's windows need at least 10 samples each.
    [ValidateRange(0, 3600)]
    [int] $SampleSeconds = 15,
    # The minutes before the first window of the summary.
    [ValidateRange(0, 600)]
    [int] $WarmupMinutes = 15,
    # The length of the summary's windows.
    [ValidateRange(1, 600)]
    [int] $WindowMinutes = 10,
    # One part of the load only (SHELLPP_SOAK_LOAD): tabs, forward, sftp.
    [ValidateSet('', 'tabs', 'forward', 'sftp', 'direct')]
    [string] $Load = '',
    # Only the summary of this soak.csv of an earlier run.
    [string] $Analyze,
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

# A number of soak.csv, written with this machine's decimal comma or point.
function ConvertTo-Number($value) {
    if ($null -eq $value -or "$value" -eq '') { return $null }
    return [double]::Parse(("$value" -replace ',', '.'), [System.Globalization.CultureInfo]::InvariantCulture)
}

# The lowest value or the median of a window's samples.
function Get-WindowValue([double[]] $values, [string] $how) {
    $sorted = @($values | Sort-Object)
    if ($sorted.Count -eq 0) { return $null }
    if ($how -eq 'min') { return $sorted[0] }
    $middle = [math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$middle] }
    return ($sorted[$middle - 1] + $sorted[$middle]) / 2
}

# The window comparison of the summary: lines, and the values that grew
# above 10 %.
function Get-Growth($samples) {
    $lines = New-Object System.Collections.Generic.List[string]
    $grew = @()
    $lastMinute = (ConvertTo-Number $samples[$samples.Count - 1].Minute)
    # Fewer samples than this in a window give no verdict: its lowest value
    # is then too likely a swing (review of run 12).
    $minSamples = 10
    $warmup = [math]::Min([double]$WarmupMinutes, $lastMinute / 4)
    $window = [math]::Min([double]$WindowMinutes, ($lastMinute - $warmup) / 2)
    if ($window -le 0) {
        $lines.Add('The run is too short for two windows: no growth to compare.')
        return @{ Lines = $lines; Grew = $grew; Inconclusive = $true }
    }
    $first = @($samples | Where-Object { $m = ConvertTo-Number $_.Minute; $m -ge $warmup -and $m -lt $warmup + $window })
    $last = @($samples | Where-Object { (ConvertTo-Number $_.Minute) -ge $lastMinute - $window })
    $lines.Add(("Windows: minutes {0:0.#}-{1:0.#} ({2} samples) and {3:0.#}-{4:0.#} ({5} samples); memory by the lowest value of each window, handles and threads by the median." -f
        $warmup, ($warmup + $window), $first.Count, ($lastMinute - $window), $lastMinute, $last.Count))
    if ($first.Count -lt $minSamples -or $last.Count -lt $minSamples) {
        $lines.Add("Fewer than $minSamples samples in a window: no verdict. Sample more often (-SampleSeconds) or run longer.")
        return @{ Lines = $lines; Grew = $grew; Inconclusive = $true }
    }
    $names = @('WorkingSetMB', 'PrivateMB', 'Handles', 'Threads')
    if ($samples[0].PSObject.Properties.Name -contains 'HeapLiveMB') { $names += 'HeapLiveMB' }
    foreach ($name in $names) {
        $how = if ($name -in 'Handles', 'Threads') { 'median' } else { 'min' }
        $fromValues = @($first | ForEach-Object { ConvertTo-Number $_.$name } | Where-Object { $null -ne $_ })
        $toValues = @($last | ForEach-Object { ConvertTo-Number $_.$name } | Where-Object { $null -ne $_ })
        if ($fromValues.Count -eq 0 -or $toValues.Count -eq 0) { continue }
        $from = Get-WindowValue $fromValues $how
        $to = Get-WindowValue $toValues $how
        $percent = if ($from -gt 0) { ($to - $from) / $from * 100 } else { 0 }
        $mark = if ($percent -gt 10) { ' ABOVE 10 %' } else { '' }
        if ($percent -gt 10) { $grew += $name }
        $lines.Add(("- {0} ({1}): {2} -> {3} ({4:+0.0;-0.0;0.0} %){5}" -f $name, $how, $from, $to, $percent, $mark))
    }
    return @{ Lines = $lines; Grew = $grew; Inconclusive = $false }
}

if ($Analyze) {
    if (-not (Test-Path -LiteralPath $Analyze)) { Fail "no $Analyze" }
    $old = @(Import-Csv -LiteralPath $Analyze)
    if ($old.Count -lt 2) { Fail "$Analyze has fewer than two samples" }
    $growth = Get-Growth $old
    Write-Host "soak: $Analyze, $($old.Count) samples"
    $growth.Lines | ForEach-Object { Write-Host $_ }
    if ($growth.Grew.Count -gt 0) {
        Write-Host "soak: grew above 10 %: $($growth.Grew -join ', ')" -ForegroundColor Yellow
        exit 2
    }
    if ($growth.Inconclusive) { exit 3 }
    exit 0
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
    # ValidateSet lets any letter case through; the app knows lower case.
    if ($Load) { $info.EnvironmentVariables['SHELLPP_SOAK_LOAD'] = $Load.ToLowerInvariant() }
    $process = [System.Diagnostics.Process]::Start($info)
    $stderr = $process.StandardError.ReadToEndAsync()
    Write-Host "soak: app pid $($process.Id)"

    $counts = 'http 0 socks 0 http-failed 0 sftp 0 sftp-failed 0'
    $heapLive = $null
    $heapPeak = $null
    $sampleEvery = if ($SampleSeconds -gt 0) { $SampleSeconds } else { $SampleMinutes * 60 }
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
            HeapLiveMB   = $(if ($null -ne $heapLive) { [math]::Round($heapLive / 1MB, 1) })
            HeapPeakMB   = $(if ($null -ne $heapPeak) { [math]::Round($heapPeak / 1MB, 1) })
            Http         = $values['Http']
            Socks        = $values['Socks']
            HttpFailed   = $values['HttpFailed']
            Sftp         = $values['Sftp']
            SftpFailed   = $values['SftpFailed']
        }
        $samples.Add($row)
        # The next sample's peak starts from the heap of now.
        $script:heapPeak = $heapLive
        $samples | Export-Csv -LiteralPath $csv -NoTypeInformation -Encoding UTF8
        Write-Host ("soak: sample {0} min: working set {1} MB, private {2} MB, handles {3}, threads {4}{5}" -f
            $row.Minute, $row.WorkingSetMB, $row.PrivateMB, $row.Handles, $row.Threads, $(if ($minimized) { ', MINIMIZED' } else { '' }))
    }

    while ($true) {
        if ($pending.Wait(1000)) {
            $line = $pending.Result
            if ($null -eq $line) { $problem = "the app ended (code $(if ($process.HasExited) { $process.ExitCode } else { '?' }))"; break }
            if ($line -match '^bench: soak heap live (\d+) peak (\d+)$') {
                $heapLive = [double]$Matches[1]
                $heapPeak = [math]::Max([double]$Matches[2], $(if ($null -ne $heapPeak) { $heapPeak } else { 0 }))
            }
            elseif ($line.StartsWith('bench: ')) { Write-Host "  $line" }
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
            $nextSample = $nextSample.AddSeconds($sampleEvery)
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

# The growth: windows, not single samples (see the help).
$last = $samples[$samples.Count - 1]
$lines = New-Object System.Collections.Generic.List[string]
$serverName = if ($useOpenSsh) { 'OpenSSH in Docker' } else { 'russh test server (no SFTP, no htop)' }
$every = if ($SampleSeconds -gt 0) { "$SampleSeconds s" } else { "$SampleMinutes min" }
$loadName = if ($Load) { ", load: $Load only" } else { '' }
$lines.Add("Soak run: $Minutes min on $serverName$loadName, a sample every $every, $($samples.Count) samples.")
$lines.Add("Last counts: $counts")
$lines.Add("The window was found minimized and shown again $restored times.")
$lines.Add('')
$lines.Add('| Minute | Working set, MB | Private, MB | Handles | Threads | Minimized | Heap live, MB | Heap peak, MB |')
$lines.Add('|---|---|---|---|---|---|---|---|')
foreach ($s in $samples) { $lines.Add("| $($s.Minute) | $($s.WorkingSetMB) | $($s.PrivateMB) | $($s.Handles) | $($s.Threads) | $($s.Minimized) | $($s.HeapLiveMB) | $($s.HeapPeakMB) |") }
$lines.Add('')
$growth = Get-Growth $samples
$growth.Lines | ForEach-Object { $lines.Add($_) }
$grew = $growth.Grew
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
    # The end of the warm-up, where the first window of the summary starts.
    $xWarm = $left + $plotW * [math]::Min([double]$WarmupMinutes, $maxMinute / 4) / $maxMinute
    $graphics.DrawLine((New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(200, 200, 200)), 1), $xWarm, $top, $xWarm, $top + $plotH)
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
if ($growth.Inconclusive) {
    Write-Host 'soak: too few samples for a verdict' -ForegroundColor Yellow
    exit 3
}
exit 0
