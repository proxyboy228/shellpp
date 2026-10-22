<#
.SYNOPSIS
  Screenshots of the Shell++ window (owner's stage 1, step 5.2, stages 2a
  and 2b, step 5.1, and the Keychain of stage 3).

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

  The terminal states (stage 2a) need an SSH test server, never a real host
  (-Server):
  - auto (default): OpenSSH in Docker (tests/openssh: bash with
    `ls --color`, htop, mc), started with Git's bash and removed afterwards,
    when a Docker daemon for Linux containers answers; else the russh test
    server (tests/sshd), which the app starts as a child process;
  - openssh: OpenSSH in Docker, required;
  - russh: the russh test server.
  The app gets the OpenSSH server's description through
  SHELLPP_TEST_SERVER_JSON. The line "server: ..." says which one ran. The
  session layer of the run has no key files and no agent (the test hosts
  have a password); the local terminal state runs the default shell with
  APPDATA, LOCALAPPDATA, TEMP and TMP in the temporary folder, so its
  commands never reach the user's PowerShell history.

  States per size (src/dev/screenshots.rs, `states_for`):
  - 1920x1200: root, group, host-panel;
  - 1280x800: root, group, host-panel, context-menu, search-address
    (root@10.0.0.5:2222, Connect active), search-results, search-no-results,
    three-selected, empty, list-view, new-host, quick-connect-toast,
    tag-filter, tag-chips (two tags on and the host panel open: the chips
    collapse into "+N"), show-more, known-hosts, first-run, unlock, and the
    terminal states: terminal-ls-color (OpenSSH; terminal-colors on the
    russh server, whose shell has no ls), terminal-htop (OpenSSH;
    terminal-alt-screen on the russh server), terminal-tofu,
    terminal-host-key-changed, terminal-drop, terminal-local, terminal-tabs,
    and those of stage 2b: terminal-connecting, terminal-splits,
    split-picker, tab-menu, new-tab, new-tab-search, sidebar-tools,
    terminal-scrollback, broadcast-tabs, broadcast-marks, sidebar-snippets,
    sidebar-history, sidebar-appearance, sidebar-font,
    and those of stage 9: telnet-tab, serial-tab (and, without a session,
    host-panel-telnet and serial-dialog);
    and those of stage 3: keychain, keychain-key-panel, keychain-generate,
    keychain-import-unknown, keychain-empty;
  - 960x600 (the minimum window size): root, host-panel, first-run;
  - any other size: root, group, host-panel.

  Stage 3, the data transfer dialogs (1280x800, after new-host; temporary
  files only, deleted as soon as the dialog has read them):
  - export-data: Export Data as it opens, encrypted with a password (the
    default);
  - export-data-plain: Export Data with encryption off, the clear-text
    warning;
  - export-openssh: Export Data as OpenSSH files (key files, passphrase);
  - import-data: the summary of Import Data after a plain export of the
    demo itself (every record already here, nothing added);
  - import-ssh-config: the import dialog's preview of a sample OpenSSH
    config (never ~/.ssh/config): new hosts, a key file with a passphrase,
    a missing one, a ProxyCommand that is not imported.

  Stage 2b, step 5.1 (-Stage2b selects these states, both themes, 1280x800):
  - known-hosts: the Known Hosts screen after an import of a temporary
    known_hosts file (plain, [host]:port, hashed and revoked entries; never
    ~/.ssh/known_hosts), the details panel open;
  - terminal-connecting: a tab while it connects (the test server holds new
    connections: tests/openssh control.sh freeze, or the russh server's
    hold), its warning dot; terminal-drop: a dropped session, its danger dot;
  - terminal-splits: a tab of 4 panes (Ctrl+Shift+D / Ctrl+Shift+E and Enter
    in the new pane); split-picker: the new pane's host list, the split
    pane's host first; tab-menu: the menu of a tab with two panes;
  - new-tab, new-tab-search: the new tab page without and with a search;
  - sidebar-tools (a search with its matches highlighted);
    terminal-scrollback (seq 1 10000, a search scrolled back to a line in
    the compact scrollback, D85); broadcast-tabs, broadcast-marks (the tab
    list of Broadcast input and its towers in the tab strip, D86);
    sidebar-snippets,
    sidebar-history (typed commands and the hint card), sidebar-appearance
    (the theme list), sidebar-font (the Font page);
  - terminal-htop: in the light theme, after the minimum contrast change.

  Stage 3, the Keychain (-Stage3 selects these states, both themes,
  1280x800; the demo profile's keys are generated when it is seeded, and
  ~/.ssh is never read):
  - keychain: the Keys and Identities sections with the demo keys;
  - keychain-key-panel: the key panel of the first key, the private key
    hidden (type, SHA256 fingerprint and the note instead);
  - keychain-generate: the Generate key form with a label typed;
  - keychain-import-unknown: the import form with a public key pasted: the
    type "unknown" with its reason;
  - keychain-empty: the Keychain of an empty profile (Generate, Import,
    Import from ~/.ssh).

  -States limits a run to some states (a smoke run; the app gets them as
  SHELLPP_SCREENSHOT_STATES): each listed state at the sizes that have it,
  both themes. Only the PNGs of those states at the run's sizes are
  replaced in the output folder.

  Output: target\ui-screens\<W>x<H>-<theme>-<state>.png (180 files for the
  default sizes). Exits with 1 when the build, the run or a file check fails.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -NoBuild
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -States terminal-ls-color,terminal-drop
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -Stage2b -Server openssh
  (the screenshots of stage 2b, step 5.1)
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -Stage3
  (the Keychain screenshots of stage 3)
#>
[CmdletBinding()]
param(
    # Window content sizes in logical pixels; all of them in one app launch.
    [string[]] $Sizes = @('1920x1200', '1280x800', '960x600'),
    # Output folder; default <repo>\target\ui-screens.
    [string] $OutDir,
    # A launch that takes longer is killed and counts as a failure. A full
    # run of every state takes well over 10 minutes; the app limits each
    # state itself (120 s, one retry) and gives up after 300 s without
    # progress (D72).
    [int] $TimeoutSeconds = 3600,
    # SSH test server of the terminal states.
    [ValidateSet('auto', 'openssh', 'russh')]
    [string] $Server = 'auto',
    # Only these states (names as in the file names, e.g. terminal-drop).
    [string[]] $States = @(),
    # The states of stage 2b, step 5.1 (added to -States).
    [switch] $Stage2b,
    # The Keychain states of stage 3 (added to -States).
    [switch] $Stage3,
    # Use the existing debug binaries.
    [switch] $NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'target\ui-screens' }
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$exe = Join-Path $targetDir 'debug\shellpp.exe'
$sshdExe = Join-Path $targetDir 'debug\shellpp-test-sshd.exe'

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

# $null when a Docker daemon for Linux containers answers, else the reason.
function Test-DockerLinux {
    $docker = Get-Command docker -ErrorAction SilentlyContinue
    if (-not $docker) { return 'Docker is not installed' }
    $os = & docker info --format '{{.OSType}}' 2>$null
    if ($LASTEXITCODE -ne 0) { return 'Docker is not running (docker info failed)' }
    if ("$os".Trim() -ne 'linux') { return "the Docker daemon runs $os containers" }
    return $null
}

# The states of a size (src/dev/screenshots.rs, `states_for`), with the
# terminal states named as on OpenSSH.
$allStates = @('root', 'group', 'host-panel', 'context-menu', 'search-address', 'search-results',
    'search-no-results', 'three-selected', 'empty', 'list-view', 'new-host', 'tag-filter', 'tag-chips',
    'show-more', 'agent-forwarding', 'known-hosts', 'keychain', 'keychain-key-panel', 'keychain-generate', 'keychain-import-unknown',
    'keychain-empty', 'first-run', 'unlock', 'terminal-tofu', 'terminal-ls-color', 'terminal-htop',
    'terminal-drop', 'terminal-local', 'terminal-tabs', 'terminal-connecting', 'terminal-splits',
    'split-picker', 'tab-menu', 'new-tab', 'new-tab-search', 'sidebar-tools', 'terminal-scrollback', 'broadcast-tabs', 'broadcast-marks', 'sidebar-snippets',
    'sidebar-history', 'sidebar-appearance', 'sidebar-font', 'quick-connect-toast', 'terminal-host-key-changed',
    'host-panel-telnet', 'serial-dialog', 'telnet-tab', 'serial-tab')
# Stage 3, the data transfer dialogs (ShotState::ExportData and the two
# imports, captured after new-host).
$allStates += @('export-data', 'export-data-plain', 'export-openssh', 'import-data', 'import-ssh-config')
# Stage 4 (run 3): the SFTP tab and the Keychain's Export to host and
# certificate (ShotState::Sftp*, Keychain{Export,Certificate}).
$stage4States = @('sftp-local', 'sftp-picker', 'sftp-queue', 'sftp-actions', 'sftp-delete', 'sftp-permissions',
    'keychain-export', 'keychain-certificate')
$allStates += $stage4States
# Stage 5 (run 3): Port Forwarding and Snippets (ShotState::Forwarding*,
# Snippet*).
$stage5States = @('forwarding-empty', 'forwarding-cards', 'forwarding-form', 'snippets', 'snippet-panel',
    'snippet-variables', 'snippet-confirm', 'snippet-results', 'shell-history')
# Stage 6 (run 3): the Logs screen and the Logs settings (ShotState::Logs,
# SettingsLogs).
$stage6States = @('logs', 'settings-logs', 'settings-shortcuts')
# Run 4 (D68 and the rest of run 3).
$run4States = @('snippet-leave', 'forwarding-wizard-type', 'forwarding-wizard-remote',
    'forwarding-wizard-dynamic', 'forwarding-wizard-host', 'forwarding-wizard-listen', 'forwarding-wizard-dest',
    'forwarding-wizard-summary')
$allStates += $run4States
# Run 4, plan 9.2: full-screen programs and Unicode (ShotState::Terminal{Tmux,
# TmuxResize,Mc,Vim,Nano,Less,Emacs,Vttest,Unicode}); OpenSSH only, left out
# on the russh test server.
$programStates = @('terminal-tmux', 'terminal-tmux-resize', 'terminal-mc', 'terminal-vim', 'terminal-nano',
    'terminal-less', 'terminal-emacs', 'terminal-vttest', 'terminal-unicode')
$allStates += $programStates
$allStates += $stage6States
$allStates += $stage5States
# Stage 2b, step 5.1 (src/dev/screenshots.rs, ShotState::STAGE_2B, plus
# htop in the light theme after the contrast change and the drop's dot).
$stage2bStates = @('known-hosts', 'terminal-connecting', 'terminal-splits', 'split-picker', 'tab-menu',
    'new-tab', 'new-tab-search', 'sidebar-tools', 'sidebar-snippets', 'sidebar-history', 'sidebar-appearance',
    'sidebar-font', 'terminal-htop', 'terminal-drop')
# Stage 3, the Keychain (src/dev/screenshots.rs, ShotState::STAGE_3).
$stage3States = @('keychain', 'keychain-key-panel', 'keychain-generate', 'keychain-import-unknown',
    'keychain-empty')
function Get-SizeStates([string] $size) {
    if ($size -eq '1280x800') { return $allStates }
    if ($size -eq '960x600') { return @('root', 'host-panel', 'first-run') }
    return @('root', 'group', 'host-panel')
}
# The file label of a state on the russh test server (no ls, no htop).
$russhLabels = @{ 'terminal-ls-color' = 'terminal-colors'; 'terminal-htop' = 'terminal-alt-screen' }

# Accept "-Sizes 1920x1200,1280x800" as well as separate values.
$Sizes = @($Sizes | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
foreach ($size in $Sizes) {
    if ($size -notmatch '^\d{3,4}x\d{3,4}$') { Fail "bad size '$size' (expected WxH, e.g. 1920x1200)" }
}
if (@($Sizes | Select-Object -Unique).Count -ne $Sizes.Count) { Fail 'a size is listed twice' }
if ($Stage2b) { $States = @($States) + $stage2bStates }
if ($Stage3) { $States = @($States) + $stage3States }
$States = @($States | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ } | Select-Object -Unique)
foreach ($state in $States) {
    $known = $allStates + @($russhLabels.Values)
    if ($state -notin $known) { Fail "unknown state '$state'" }
}
# A russh file label names the same state.
$States = @($States | ForEach-Object {
        $name = $_
        $original = $russhLabels.Keys | Where-Object { $russhLabels[$_] -eq $name }
        if ($original) { $original } else { $name }
    } | Select-Object -Unique)

Push-Location $repo
try {
    if (-not $NoBuild) {
        Write-Host 'ui-screens: building shellpp (features: screenshots) and shellpp-test-sshd...'
        & cargo build -p shellpp-desktop --features screenshots -p shellpp-test-sshd
        if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
    }
}
finally {
    Pop-Location
}
if (-not (Test-Path -LiteralPath $exe)) { Fail "binary not found: $exe" }
if (-not (Test-Path -LiteralPath $sshdExe)) { Fail "binary not found: $sshdExe" }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
if ($States.Count -gt 0) {
    # A smoke run replaces only the PNGs of its states at its sizes.
    foreach ($state in $States) {
        $labels = @($state)
        if ($russhLabels.ContainsKey($state)) { $labels += $russhLabels[$state] }
        foreach ($label in $labels) {
            foreach ($size in $Sizes) {
                Get-ChildItem -LiteralPath $OutDir -Filter "$size-*-$label.png" -File | Remove-Item -Force
            }
        }
    }
}
else {
    Get-ChildItem -LiteralPath $OutDir -Filter '*.png' -File | Remove-Item -Force
}

$failures = @()
$saved = @()
# A temporary data folder for this launch only; removed afterwards.
$dataDir = Join-Path ([System.IO.Path]::GetTempPath()) ('shellpp-ui-screens-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dataDir | Out-Null
$process = $null
$bash = $null
$opensshDir = $null
$serverJson = $null
try {
    $reason = $null
    # A run limited to states without sessions needs no test server.
    $terminalOnly = @('terminal-tofu', 'terminal-ls-color', 'terminal-htop', 'terminal-drop', 'terminal-local',
        'terminal-tabs', 'quick-connect-toast', 'terminal-host-key-changed', 'terminal-connecting',
        'terminal-splits', 'split-picker', 'tab-menu', 'new-tab', 'new-tab-search', 'sidebar-tools',
        'terminal-scrollback', 'broadcast-tabs', 'broadcast-marks', 'telnet-tab', 'serial-tab', 'sidebar-snippets',
        'sidebar-history', 'sidebar-appearance', 'sidebar-font') + $programStates
    $needsServer = ($States.Count -eq 0) -or (@($States | Where-Object { $_ -in $terminalOnly }).Count -gt 0)
    if (-not $needsServer) {
        Write-Host 'ui-screens: server: none (no terminal state)'
    }
    elseif ($Server -ne 'russh') {
        $reason = Test-DockerLinux
        if (-not $reason) {
            $bash = Find-GitBash
            if (-not $bash) { $reason = "Git's bash was not found" }
        }
        if ($reason -and $Server -eq 'openssh') { Fail "OpenSSH needs Docker and Git's bash: $reason" }
    }
    if ($needsServer -and $Server -ne 'russh' -and -not $reason) {
        $opensshDir = Join-Path $dataDir 'openssh'
        New-Item -ItemType Directory -Path $opensshDir | Out-Null
        # The program states need the image with the terminal programs
        # (tests/openssh/programs.Dockerfile, D83); the others the main one.
        $needsPrograms = ($States.Count -eq 0) -or (@($States | Where-Object { $_ -in $programStates }).Count -gt 0)
        $env:SHELLPP_OPENSSH_PROGRAMS = if ($needsPrograms) { '1' } else { '' }
        try {
            $serverJson = (& $bash (Join-Path $repo 'tests\openssh\start.sh') $opensshDir | Select-Object -Last 1)
        }
        finally {
            Remove-Item Env:\SHELLPP_OPENSSH_PROGRAMS -ErrorAction SilentlyContinue
        }
        if ($LASTEXITCODE -ne 0 -or -not $serverJson) { Fail "tests/openssh/start.sh failed (exit $LASTEXITCODE)" }
        $serverJson = $serverJson.Trim()
        $image = if ($needsPrograms) { 'with the terminal programs' } else { 'main image' }
        Write-Host "ui-screens: server: OpenSSH in Docker (tests/openssh, $image)"
    }
    elseif ($needsServer) {
        $note = if ($reason) { " - $reason" } else { '' }
        Write-Host "ui-screens: server: russh test server (tests/sshd)$note"
    }
    $sizeList = $Sizes -join ','
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $exe
    $info.Arguments = "--screenshots `"$OutDir`" --size $sizeList"
    $info.UseShellExecute = $false
    $info.WorkingDirectory = $repo
    $info.EnvironmentVariables['SHELLPP_DATA_DIR'] = $dataDir
    $info.EnvironmentVariables['SHELLPP_TEST'] = '1'
    $info.EnvironmentVariables['SHELLPP_TEST_SSHD_EXE'] = $sshdExe
    if ($bash) { $info.EnvironmentVariables['SHELLPP_BASH'] = $bash }
    if ($serverJson) { $info.EnvironmentVariables['SHELLPP_TEST_SERVER_JSON'] = $serverJson }
    if ($States.Count -gt 0) { $info.EnvironmentVariables['SHELLPP_SCREENSHOT_STATES'] = ($States -join ',') }
    # The local terminal state types commands into Windows PowerShell, which
    # keeps its history (PSReadLine) under APPDATA: they go to the temporary
    # folder, never into the user's history.
    foreach ($name in @('appdata', 'localappdata', 'tmp')) {
        New-Item -ItemType Directory -Path (Join-Path $dataDir $name) | Out-Null
    }
    $info.EnvironmentVariables['APPDATA'] = (Join-Path $dataDir 'appdata')
    $info.EnvironmentVariables['LOCALAPPDATA'] = (Join-Path $dataDir 'localappdata')
    $info.EnvironmentVariables['TEMP'] = (Join-Path $dataDir 'tmp')
    $info.EnvironmentVariables['TMP'] = (Join-Path $dataDir 'tmp')
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
    if ($opensshDir) { & $bash (Join-Path $repo 'tests\openssh\stop.sh') $opensshDir }
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
# host-panel in both themes, the default size every terminal state. A run
# limited by -States has exactly its states at the sizes that have them.
$terminalStates = @('terminal-tofu', 'terminal-host-key-changed', 'terminal-drop', 'terminal-local', 'terminal-tabs',
    'terminal-ls-color', 'terminal-htop') + $stage2bStates + $stage3States + $programStates
# (PowerShell names ignore case: a `$states` here would overwrite -States.)
$expectedCount = 0
foreach ($size in $Sizes) {
    if ($States.Count -gt 0) {
        $sizeStates = @(Get-SizeStates $size | Where-Object { $_ -in $States })
        if (-not $serverJson) { $sizeStates = @($sizeStates | Where-Object { $_ -notin $programStates }) }
        $expectedCount += 2 * $sizeStates.Count
    }
    else {
        $sizeStates = @('root', 'host-panel')
        if ($size -eq '1280x800') { $sizeStates += $terminalStates }
        if (-not $serverJson) { $sizeStates = @($sizeStates | Where-Object { $_ -notin $programStates }) }
    }
    foreach ($theme in @('dark', 'light')) {
        foreach ($state in $sizeStates) {
            $label = $state
            if (-not $serverJson -and $russhLabels.ContainsKey($state)) { $label = $russhLabels[$state] }
            $file = Join-Path $OutDir "$size-$theme-$label.png"
            if (-not (Test-Path -LiteralPath $file)) { $failures += "missing $file" }
        }
    }
}
if ($States.Count -gt 0) {
    if ($expectedCount -eq 0) { $failures += "no size of '$($Sizes -join ',')' has the states '$($States -join ',')'" }
    elseif ($saved.Count -ne $expectedCount) { $failures += "$($saved.Count) screenshots saved, expected $expectedCount" }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Host "ui-screens: $failure" -ForegroundColor Red }
    exit 1
}
Write-Host "ui-screens: done, $($saved.Count) screenshots in $OutDir"
exit 0
