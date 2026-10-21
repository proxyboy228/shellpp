<#
.SYNOPSIS
  Terminal and session measurements of Shell++ (owner's stage 2a, step 4).

.DESCRIPTION
  Builds the release binary with the `dev-tools` feature and the russh test
  server (`shellpp-test-sshd`), then measures, one app launch each:

  1. RSS idle: `shellpp --demo` (300 hosts), no session;
  2. RSS with 1 SSH session and with 10 SSH sessions:
     `shellpp --demo --bench-sessions N` opens N session tabs in the main
     window (as Connect does) to the test server, waits until all are
     connected and at their prompt, prints `bench: ready` and idles until
     the script sends `quit` on stdin;
  3. output throughput: `shellpp --demo --bench-output` times
     `seq 1 3000000` (-SeqLines) five ways and quits, so that the cost of our
     terminal can be told apart from the PTY and the shell:
     - ssh: typed into an SSH session (our terminal);
     - local-direct: seq itself as the program of a local terminal, no
       shell (our terminal);
     - local-shell: typed into the default local shell, Windows PowerShell
       (our terminal);
     - bare-direct, bare-shell: the same two through the same PTY layer
       (ConPTY on Windows) read by a bare reader without a terminal
       emulator or window: the baseline.
     On Windows seq is Git's seq.exe when it exists (never downloaded);
     without it the shell runs use a PowerShell loop and the direct runs are
     skipped. The app prints what it used. The table adds "our terminal"
     rows: local-direct minus bare-direct, local-shell minus bare-shell;
  4. input latency: `shellpp --demo --bench-latency` sends 200 keystrokes
     (-LatencyKeys) each to a local terminal and to an SSH session and prints
     p50/p95 from the key event to the first painted frame that shows the
     echoed glyph, then quits;
  5. splits (stage 2b, step 4): `shellpp --demo --bench-splits` opens 4
     session tabs in the main window and splits each into 4 panes (2 x 2),
     every pane an SSH session to the test server: RSS with 16 sessions,
     read like the session measurements;
  6. flood (stage 2b, step 4): `shellpp --demo --bench-flood` runs
     `yes | head -n 5000000` again and again in one SSH session for
     -FloodSeconds (default 600; `seq 1 5000000` on the russh test server,
     whose shell has no `yes`). At each `bench: flood sample` line of the app
     (start, every -FloodSampleSeconds, end) the script reads RSS and private
     bytes into -FloodCsv (phase, seconds, working set bytes, private bytes,
     children) and prints start, peak and end. The app then prints the
     number of runs and the terminal's scrollback line count against its cap
     (more than the cap fails the run); RSS is read once more -IdleSeconds
     after that ("after the flood").

  RSS (working set) and private bytes are read -IdleSeconds after the app
  reported ready, for the app process and its child processes (local shells),
  never counting the russh test server, which the app starts as a child
  process when no OpenSSH server is used.

  Test server (-Server):
  - auto (default): OpenSSH in Docker (tests/openssh, started with Git's
    bash and removed afterwards) when a Docker daemon for Linux containers
    answers, else the russh test server;
  - openssh: OpenSSH in Docker, required;
  - russh: the russh test server.
  The line "server: ..." says which one ran.

  Every launch gets SHELLPP_TEST=1 and SHELLPP_DATA_DIR, TEMP, TMP, APPDATA
  and LOCALAPPDATA in a fresh temporary folder (the demo profile, the test
  server's files and the local PowerShell's command history live there), so
  the permanent profile, the OS keychain, ~/.ssh, the SSH agent and the
  user's PowerShell history are never used. The folder is removed afterwards.

  Output: one line per measurement and a summary table; on GitHub Actions
  the table also goes to the job summary.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\measure-terminal.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\measure-terminal.ps1 -Server russh -NoBuild -Only output,latency
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\measure-terminal.ps1 -BuildProfile debug -SeqLines 20000 -LatencyKeys 20 -IdleSeconds 2 -FloodSeconds 20 -FloodSampleSeconds 5
  (a quick smoke run of every measurement)
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\measure-terminal.ps1 -Server openssh -Only splits,flood
  (stage 2b, step 4: 16 sessions in 4 tabs x 4 panes, the 10-minute flood)
#>
[CmdletBinding()]
param(
    [ValidateSet('auto', 'openssh', 'russh')]
    [string] $Server = 'auto',
    # Cargo profile of the measured binaries.
    [ValidateSet('release', 'debug')]
    [string] $BuildProfile = 'release',
    # Idle time after the app reported ready, before RSS is read.
    [int] $IdleSeconds = 10,
    # Time allowed for one launch (window, sessions, a benchmark); the
    # output launch runs five seq runs, four of them local (on Windows each
    # local run of 3000000 lines takes minutes, in ConPTY itself).
    [int] $TimeoutSeconds = 2400,
    # Numbers of the output benchmark (seq 1 N).
    [ValidateRange(1, 10000000)]
    [int] $SeqLines = 3000000,
    # Keystrokes per terminal of the latency benchmark.
    [ValidateRange(1, 2000)]
    [int] $LatencyKeys = 200,
    # Length of the flood (seconds; a run that started goes on to its end).
    [ValidateRange(1, 7200)]
    [int] $FloodSeconds = 600,
    # Interval of the flood's memory samples (seconds).
    [ValidateRange(1, 7200)]
    [int] $FloodSampleSeconds = 30,
    # The flood's memory samples; default <target dir>\measure-flood-rss.csv.
    [string] $FloodCsv,
    # Measurements to run: idle, sessions1, sessions10, splits, flood, output,
    # latency.
    [string[]] $Only = @('idle', 'sessions1', 'sessions10', 'splits', 'flood', 'output', 'latency'),
    # Use the existing binaries.
    [switch] $NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$exe = Join-Path $targetDir "$BuildProfile\shellpp.exe"
$sshdExe = Join-Path $targetDir "$BuildProfile\shellpp-test-sshd.exe"
if (-not $FloodCsv) { $FloodCsv = Join-Path $targetDir 'measure-flood-rss.csv' }
$Only = @($Only | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
foreach ($name in $Only) {
    if ($name -notin @('idle', 'sessions1', 'sessions10', 'splits', 'flood', 'output', 'latency')) { throw "unknown measurement '$name'" }
}

function Fail([string] $message) {
    Write-Host "measure-terminal: $message" -ForegroundColor Red
    exit 1
}

# Culture-invariant numbers ("55.6 MB" also in a ru-RU console).
$invariant = [System.Globalization.CultureInfo]::InvariantCulture
function MB([long] $bytes) { ($bytes / 1MB).ToString('N1', $invariant) + ' MB' }

# Git's bash (never WSL's bash.exe, which cannot read Windows paths).
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

function Test-DockerLinux {
    $docker = Get-Command docker -ErrorAction SilentlyContinue
    if (-not $docker) { return 'Docker is not installed' }
    $os = & docker info --format '{{.OSType}}' 2>$null
    if ($LASTEXITCODE -ne 0) { return 'Docker is not running (docker info failed)' }
    if ("$os".Trim() -ne 'linux') { return "the Docker daemon runs $os containers" }
    return $null
}

Push-Location $repo
try {
    if (-not $NoBuild) {
        $cargoArgs = @('build', '--locked', '-p', 'shellpp-desktop', '--features', 'dev-tools', '-p', 'shellpp-test-sshd')
        if ($BuildProfile -eq 'release') { $cargoArgs += '--release' }
        Write-Host "measure-terminal: cargo $($cargoArgs -join ' ')"
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
    }
}
finally {
    Pop-Location
}
if (-not (Test-Path -LiteralPath $exe)) { Fail "binary not found: $exe" }
if (-not (Test-Path -LiteralPath $sshdExe)) { Fail "binary not found: $sshdExe" }

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('shellpp-measure-terminal-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempRoot | Out-Null

$bash = $null
$opensshDir = $null
$serverJson = $null
$serverLabel = $null
$results = New-Object System.Collections.Generic.List[object]
$problems = New-Object System.Collections.Generic.List[string]

function Remove-Folder([string] $path) {
    for ($attempt = 0; $attempt -lt 20 -and (Test-Path -LiteralPath $path); $attempt++) {
        Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $path) { Start-Sleep -Milliseconds 250 }
    }
}

# Working set and private bytes of the app and its children, except the
# russh test server (a child process of the app when it runs).
function Get-AppMemory([System.Diagnostics.Process] $process) {
    $process.Refresh()
    $set = $process.WorkingSet64
    $private = $process.PrivateMemorySize64
    $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($process.Id)" -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -ne 'shellpp-test-sshd.exe' })
    foreach ($child in $children) {
        $childProcess = Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue
        if ($childProcess) {
            $set += $childProcess.WorkingSet64
            $private += $childProcess.PrivateMemorySize64
        }
    }
    return @{ Set = $set; Private = $private; Children = $children.Count }
}

# One launch of the app with these arguments. Collects the "bench: " lines
# of stdout. With -WaitReady, reads RSS -IdleSeconds after "bench: ready" (or
# the window, for the plain idle launch) and closes the window; otherwise
# waits for the app to quit by itself. With -FloodSamples, also reads RSS at
# every "bench: flood sample <phase> <seconds>" line (returned as Samples).
# -Timeout replaces -TimeoutSeconds for this launch.
function Invoke-App([string] $label, [string[]] $appArgs, [switch] $WaitReady, [switch] $WaitWindow, [switch] $FloodSamples, [int] $Timeout = 0) {
    $runDir = Join-Path $tempRoot ([guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $runDir | Out-Null
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $exe
    $info.Arguments = ($appArgs | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }) -join ' '
    $info.UseShellExecute = $false
    $info.WorkingDirectory = $repo
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    # `--bench-sessions` idles until a `quit` line arrives here (closing its
    # window would ask first while sessions run, UI 4.6).
    $info.RedirectStandardInput = $true
    $info.EnvironmentVariables['SHELLPP_TEST'] = '1'
    $info.EnvironmentVariables['SHELLPP_DATA_DIR'] = (Join-Path $runDir 'data')
    $info.EnvironmentVariables['TEMP'] = $runDir
    $info.EnvironmentVariables['TMP'] = $runDir
    # The local terminal's PowerShell keeps its command history (PSReadLine)
    # under APPDATA: the benchmark commands go to the temporary folder, not
    # into the user's history.
    New-Item -ItemType Directory -Path (Join-Path $runDir 'appdata'), (Join-Path $runDir 'localappdata') | Out-Null
    $info.EnvironmentVariables['APPDATA'] = (Join-Path $runDir 'appdata')
    $info.EnvironmentVariables['LOCALAPPDATA'] = (Join-Path $runDir 'localappdata')
    $info.EnvironmentVariables['SHELLPP_TEST_SSHD_EXE'] = $sshdExe
    if ($bash) { $info.EnvironmentVariables['SHELLPP_BASH'] = $bash }
    $lines = New-Object System.Collections.Generic.List[string]
    $samples = New-Object System.Collections.Generic.List[object]
    $memory = $null
    $problem = $null
    $limit = if ($Timeout -gt 0) { $Timeout } else { $TimeoutSeconds }
    $process = [System.Diagnostics.Process]::Start($info)
    $stderr = $process.StandardError.ReadToEndAsync()
    try {
        $deadline = (Get-Date).AddSeconds($limit)
        $pending = $process.StandardOutput.ReadLineAsync()
        $ready = $false
        while ($true) {
            if ($pending.Wait(100)) {
                $line = $pending.Result
                if ($null -eq $line) {
                    if (-not $process.HasExited) { $process.WaitForExit(5000) | Out-Null }
                    break
                }
                if ($line.StartsWith('bench: ')) {
                    $lines.Add($line)
                    Write-Host "  $line"
                }
                if ($FloodSamples -and $line -match '^bench: flood sample (\S+) (\d+)$') {
                    $sample = Get-AppMemory $process
                    $samples.Add([pscustomobject]@{ Phase = $Matches[1]; Seconds = [int]$Matches[2]; Set = $sample.Set; Private = $sample.Private; Children = $sample.Children })
                }
                if ($line -eq 'bench: ready') { $ready = $true }
                if ($line.StartsWith('bench: failed')) { $problem = $line; break }
                $pending = $process.StandardOutput.ReadLineAsync()
            }
            if ($WaitWindow -and -not $ready) {
                $process.Refresh()
                if ($process.MainWindowHandle -ne [IntPtr]::Zero) { $ready = $true }
            }
            if (($WaitReady -or $WaitWindow) -and $ready) {
                Start-Sleep -Seconds $IdleSeconds
                if ($process.HasExited) { $problem = "the app exited while idle (code $($process.ExitCode))"; break }
                $memory = Get-AppMemory $process
                break
            }
            if ($process.HasExited -and $pending.IsCompleted -and $null -eq $pending.Result) { break }
            if ((Get-Date) -gt $deadline) { $problem = "no result after $limit s"; break }
        }
    }
    finally {
        if (-not $process.HasExited) {
            # A normal end: the app closes its sessions, quits and deletes
            # its demo profile. `quit` on stdin for --bench-sessions, closing
            # the window otherwise.
            $quitSent = $false
            if ($WaitReady) {
                try {
                    $process.StandardInput.WriteLine('quit')
                    $process.StandardInput.Flush()
                    $quitSent = $true
                }
                catch { }
            }
            if (-not $quitSent -or -not $process.WaitForExit(10000)) {
                $null = $process.CloseMainWindow()
                if (-not $process.WaitForExit(20000)) {
                    $process.Kill()
                    $process.WaitForExit()
                }
            }
        }
        Remove-Folder $runDir
    }
    if (-not $problem -and -not ($WaitReady -or $WaitWindow) -and $process.ExitCode -ne 0) {
        $problem = "exit code $($process.ExitCode)"
    }
    if ($problem) {
        # Warnings and errors of the app (its log never holds secrets, hosts
        # or terminal data).
        $stderr.Result -split "`r?`n" |
            ForEach-Object { $_ -replace "$([char]27)\[[0-9;]*m", '' } |
            Where-Object { $_ -and $_ -notmatch '^\d{4}-\d\d-\d\dT\S+\s+(TRACE|DEBUG|INFO)\s' } |
            Select-Object -Last 20 |
            ForEach-Object { Write-Host "    $_" }
        $problems.Add("${label}: $problem")
    }
    return @{ Lines = $lines; Memory = $memory; Samples = $samples }
}

try {
    # The test server.
    $reason = $null
    if ($Server -ne 'russh') {
        $reason = Test-DockerLinux
        if (-not $reason) {
            $bash = Find-GitBash
            if (-not $bash) { $reason = "Git's bash was not found" }
        }
        if ($reason -and $Server -eq 'openssh') { Fail "OpenSSH needs Docker and Git's bash: $reason" }
    }
    if ($Server -ne 'russh' -and -not $reason) {
        $opensshDir = Join-Path $tempRoot 'openssh'
        New-Item -ItemType Directory -Path $opensshDir | Out-Null
        $startScript = Join-Path $repo 'tests\openssh\start.sh'
        $serverJson = (& $bash $startScript $opensshDir | Select-Object -Last 1)
        if ($LASTEXITCODE -ne 0 -or -not $serverJson) { Fail "tests/openssh/start.sh failed (exit $LASTEXITCODE)" }
        $serverJson = $serverJson.Trim()
        $serverLabel = 'OpenSSH in Docker (tests/openssh)'
    }
    else {
        $serverLabel = 'russh test server (tests/sshd, child process of the app)'
        if ($reason) { $serverLabel += " - $reason" }
    }
    Write-Host "measure-terminal: server: $serverLabel"
    $serverArgs = @()
    if ($serverJson) { $serverArgs = @('--test-server', $serverJson) }

    if ('idle' -in $Only) {
        Write-Host 'measure-terminal: idle, 300 hosts, no session'
        $run = Invoke-App 'idle' @('--demo', '--theme', 'dark') -WaitWindow
        if ($run.Memory) { $results.Add([pscustomobject]@{ Measurement = 'RSS idle, 300 hosts'; Value = (MB $run.Memory.Set); Detail = "private $(MB $run.Memory.Private), children $($run.Memory.Children)" }) }
    }
    foreach ($count in @(1, 10)) {
        if ("sessions$count" -notin $Only) { continue }
        Write-Host "measure-terminal: $count SSH session(s)"
        $run = Invoke-App "sessions$count" (@('--demo', '--theme', 'dark', '--bench-sessions', "$count") + $serverArgs) -WaitReady
        if ($run.Memory) { $results.Add([pscustomobject]@{ Measurement = "RSS, 300 hosts, $count SSH session(s)"; Value = (MB $run.Memory.Set); Detail = "private $(MB $run.Memory.Private), children $($run.Memory.Children)" }) }
    }
    if ('splits' -in $Only) {
        Write-Host 'measure-terminal: 4 tabs x 4 panes (16 SSH sessions)'
        $run = Invoke-App 'splits' (@('--demo', '--theme', 'dark', '--bench-splits') + $serverArgs) -WaitReady
        $connected = ''
        foreach ($line in $run.Lines) {
            if ($line -match '^bench: splits connected (\d+) sessions .* in (\d+) ms$') { $connected = "; $($Matches[1]) sessions connected in $($Matches[2]) ms" }
        }
        if ($run.Memory) { $results.Add([pscustomobject]@{ Measurement = 'RSS, 300 hosts, 4 tabs x 4 panes (16 SSH sessions)'; Value = (MB $run.Memory.Set); Detail = "private $(MB $run.Memory.Private), children $($run.Memory.Children)$connected" }) }
    }
    if ('flood' -in $Only) {
        Write-Host "measure-terminal: flood, $FloodSeconds s, a sample every $FloodSampleSeconds s"
        $floodArgs = @('--demo', '--theme', 'dark', '--bench-flood', '--flood-seconds', "$FloodSeconds", '--sample-seconds', "$FloodSampleSeconds") + $serverArgs
        $run = Invoke-App 'flood' $floodArgs -WaitReady -FloodSamples -Timeout ([Math]::Max($TimeoutSeconds, $FloodSeconds + 900))
        # (@() over a generic List fails in Windows PowerShell 5.1.)
        $samples = $run.Samples.ToArray()
        if ($samples.Count -gt 0) {
            $csv = @('phase,seconds,working_set_bytes,private_bytes,children')
            foreach ($sample in $samples) { $csv += "$($sample.Phase),$($sample.Seconds),$($sample.Set),$($sample.Private),$($sample.Children)" }
            $csvDir = Split-Path -Parent $FloodCsv
            if ($csvDir -and -not (Test-Path -LiteralPath $csvDir)) { New-Item -ItemType Directory -Path $csvDir | Out-Null }
            [System.IO.File]::WriteAllText($FloodCsv, (($csv -join "`n") + "`n"))
            $start = $samples[0]
            $end = $samples[$samples.Count - 1]
            $peak = $samples | Sort-Object -Property Set -Descending | Select-Object -First 1
            Write-Host "measure-terminal: flood RSS start $(MB $start.Set), peak $(MB $peak.Set) at $($peak.Seconds) s, end $(MB $end.Set) ($($samples.Count) samples in $FloodCsv)"
            $results.Add([pscustomobject]@{ Measurement = 'RSS, flood, start (connected, before the first run)'; Value = (MB $start.Set); Detail = "private $(MB $start.Private), 0 s" })
            $results.Add([pscustomobject]@{ Measurement = 'RSS, flood, peak'; Value = (MB $peak.Set); Detail = "private $(MB $peak.Private), at $($peak.Seconds) s ($($peak.Phase))" })
            $results.Add([pscustomobject]@{ Measurement = 'RSS, flood, end (after the last run)'; Value = (MB $end.Set); Detail = "private $(MB $end.Private), at $($end.Seconds) s; $($samples.Count) samples: $FloodCsv" })
        }
        else {
            $problems.Add('flood: no memory samples')
        }
        foreach ($line in $run.Lines) {
            if ($line -match '^bench: flood runs (\d+) in (\d+) s \((\d+) lines\)$') {
                $results.Add([pscustomobject]@{ Measurement = 'flood, runs'; Value = $Matches[1]; Detail = "$($Matches[3]) lines in $($Matches[2]) s" })
            }
            if ($line -match '^bench: flood scrollback (\d+) lines, cap (\d+) \((.+)\)$') {
                $results.Add([pscustomobject]@{ Measurement = 'flood, scrollback lines at the end'; Value = $Matches[1]; Detail = "cap $($Matches[2]), $($Matches[3])" })
            }
        }
        if ($run.Memory) { $results.Add([pscustomobject]@{ Measurement = "RSS, after the flood ($IdleSeconds s idle)"; Value = (MB $run.Memory.Set); Detail = "private $(MB $run.Memory.Private), children $($run.Memory.Children)" }) }
    }
    if ('output' -in $Only) {
        Write-Host "measure-terminal: seq 1 $SeqLines"
        $run = Invoke-App 'output' (@('--demo', '--theme', 'dark', '--bench-output', '--seq-lines', "$SeqLines") + $serverArgs)
        $names = @{
            'ssh'          = 'SSH session (our terminal)'
            'local-direct' = 'local, seq as the PTY program (our terminal)'
            'local-shell'  = 'local, typed into the shell (our terminal)'
            'bare-direct'  = 'local, seq as the PTY program (bare PTY reader, baseline)'
            'bare-shell'   = 'local, typed into the shell (bare PTY reader, baseline)'
        }
        $seconds = @{}
        foreach ($line in $run.Lines) {
            if ($line -match '^bench: output (\S+) (.+): ([0-9.]+) s(.*)$') {
                # Kept before the next -match, which replaces $Matches.
                $kind = $Matches[1]
                $detail = $Matches[2]
                $time = $Matches[3]
                $mark = $Matches[4]
                $name = if ($names.ContainsKey($kind)) { $names[$kind] } else { $kind }
                # The app marks a run during which a frame saw the window
                # inactive or minimized: its time is not comparable.
                if ($mark -match '\(invalid: ([^)]*)\)') {
                    $value = "invalid ($time s)"
                    Write-Host "measure-terminal: $kind is invalid ($time s): $($Matches[1])" -ForegroundColor Yellow
                }
                else {
                    $value = "$time s"
                    $seconds[$kind] = [double]::Parse($time, $invariant)
                }
                $results.Add([pscustomobject]@{ Measurement = "seq 1 $SeqLines, $name"; Value = $value; Detail = "$detail$mark" })
            }
        }
        # Our terminal's share: the same run through our terminal minus the
        # bare reader (the PTY, conhost and the shell stay in both).
        foreach ($pair in @(@('local-direct', 'bare-direct', 'seq as the PTY program'), @('local-shell', 'bare-shell', 'typed into the shell'))) {
            if ($seconds.ContainsKey($pair[0]) -and $seconds.ContainsKey($pair[1])) {
                $difference = ($seconds[$pair[0]] - $seconds[$pair[1]]).ToString('0.00', $invariant)
                $results.Add([pscustomobject]@{ Measurement = "seq 1 $SeqLines, our terminal's cost, $($pair[2])"; Value = "$difference s"; Detail = "$($pair[0]) minus $($pair[1])" })
            }
        }
    }
    if ('latency' -in $Only) {
        Write-Host "measure-terminal: input latency, $LatencyKeys keystrokes"
        $run = Invoke-App 'latency' (@('--demo', '--theme', 'dark', '--bench-latency', '--keys', "$LatencyKeys") + $serverArgs)
        foreach ($line in $run.Lines) {
            if ($line -match '^bench: latency (\S+) p50=([0-9.]+) ms p95=([0-9.]+) ms(.*)$') {
                $results.Add([pscustomobject]@{ Measurement = "input latency, $($Matches[1])"; Value = "p50 $($Matches[2]) ms, p95 $($Matches[3]) ms"; Detail = $Matches[4].Trim() })
            }
        }
    }
}
finally {
    if ($opensshDir) {
        & $bash (Join-Path $repo 'tests\openssh\stop.sh') $opensshDir
    }
    Remove-Folder $tempRoot
}

Write-Host ''
Write-Host "Server: $serverLabel; build: $BuildProfile"
$results | Format-Table -AutoSize | Out-String -Width 300 | Write-Host
if ($env:GITHUB_STEP_SUMMARY) {
    $summary = @("**Terminal measurements (informational, $BuildProfile, $serverLabel)**", '', '| Measurement | Value | Detail |', '|---|---|---|')
    foreach ($row in $results) { $summary += "| $($row.Measurement) | $($row.Value) | $($row.Detail) |" }
    Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Encoding utf8 -Value ($summary -join "`n")
}
if (Test-Path -LiteralPath $tempRoot) { Write-Host "measure-terminal: temporary folder not removed: $tempRoot" -ForegroundColor Yellow }
if ($problems.Count -gt 0) {
    foreach ($problem in $problems) { Write-Host "measure-terminal: $problem" -ForegroundColor Red }
    exit 1
}
exit 0
