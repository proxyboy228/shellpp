<#
.SYNOPSIS
  Builds locally what the release workflow builds, without signing or
  publishing anything (D225; docs/release.md, "A dry run").

.DESCRIPTION
  From a clean checkout of one commit (a detached worktree of -Commit, by
  default HEAD, in <OutDir>\src) it builds what .github/workflows/release.yml
  builds:
  - shellpp-<version>-windows-x64.zip and shellpp.pdb
    (scripts\package-windows.ps1);
  - shellpp-<version>-linux-x64.tar.gz (scripts/package-linux.sh in the image
    of tests/linux/package.Dockerfile, the workflow's image);
  - shellpp-server-<version>-docker-image.tar.gz and
    shellpp-server-<version>-linux-x64.tar.gz (server/Dockerfile,
    scripts/package-server.sh);
  - latest.json and SHA256SUMS, and notes.md, the draft's notes
    (scripts/release-notes.sh). No .minisig: nothing is signed.

  It checks what the workflow checks (the tag v<version> would match both
  versions, release/minisign.pub holds a key, CHANGELOG.md has the notes),
  that every archive holds exactly the files of the table "What a release
  holds" of docs/release.md (the lists below follow that table, which must
  name every file), and that the server image keeps its license and
  notices.

  Unlike the workflow, the builds keep their target folders between runs
  (<OutDir>\target-windows and the Docker volumes shellpp-dryrun-*): -Fresh
  empties them first. The image is tagged shellpp-server:<version> and
  shellpp-server:latest as in the workflow, and both tags are removed at
  the end. Needs Docker with Linux containers and the network (images,
  crates). The files land in <OutDir>\dist, the logs in <OutDir>\logs.

.EXAMPLE
  powershell -NoProfile -File scripts\release-dry-run.ps1
.EXAMPLE
  powershell -NoProfile -File scripts\release-dry-run.ps1 -Commit run17/work -Fresh
#>
param(
    # The commit to build (anything git rev-parse takes).
    [string] $Commit = 'HEAD',
    # Output folder; default <repo>\target\release-dry-run.
    [string] $OutDir,
    # Empty the target folders and the Docker volumes of earlier runs first.
    [switch] $Fresh
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'target\release-dry-run' }
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path (Get-Location).Path $OutDir }
$OutDir = [System.IO.Path]::GetFullPath($OutDir)
$src = Join-Path $OutDir 'src'
$dist = Join-Path $OutDir 'dist'
$logs = Join-Path $OutDir 'logs'
$work = Join-Path $OutDir 'work'
$windowsTarget = Join-Path $OutDir 'target-windows'
$packageImage = 'shellpp-package:dryrun'
$linuxTarget = 'shellpp-dryrun-linux-target'
$linuxCargo = 'shellpp-dryrun-linux-cargo'
$tar = Join-Path $env:SystemRoot 'System32\tar.exe'

. (Join-Path $PSScriptRoot 'process.ps1')

function Fail([string] $message) {
    Write-Host "release-dry-run: $message" -ForegroundColor Red
    exit 1
}

# Runs a program with its output in <logs>\<name>.log (and .err); a failure
# stops the dry run.
function Step([string] $name, [string] $program, [string[]] $arguments) {
    Write-Host "release-dry-run: $name"
    $log = Join-Path $logs "$name.log"
    $code = Invoke-Logged $program $arguments $log "$log.err"
    if ($code -ne 0) { Fail "$name failed (exit $code; $log, $log.err)" }
    return $log
}

function Write-Utf8([string] $path, [string] $text) {
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
}

# The names in an archive, folders left out ("./" and a trailing "/" off).
function Get-TarNames([string] $archive) {
    $names = & $tar -tzf $archive
    if ($LASTEXITCODE -ne 0) { Fail "tar could not list $archive" }
    return @($names | ForEach-Object { $_ -replace '^\./', '' } | Where-Object { $_ -ne '' -and -not $_.EndsWith('/') } | Sort-Object)
}

function Assert-Names([string] $what, [string[]] $actual, [string[]] $expected) {
    $actual = @($actual | Sort-Object)
    $expected = @($expected | Sort-Object)
    $diff = Compare-Object -ReferenceObject $expected -DifferenceObject $actual
    if ($diff) {
        $missing = @($diff | Where-Object { $_.SideIndicator -eq '<=' } | ForEach-Object { $_.InputObject })
        $extra = @($diff | Where-Object { $_.SideIndicator -eq '=>' } | ForEach-Object { $_.InputObject })
        Fail "$what holds the wrong files: missing [$($missing -join ', ')], extra [$($extra -join ', ')]"
    }
    Write-Host "release-dry-run: $what holds the files of docs/release.md ($($actual.Count))"
}

New-Item -ItemType Directory -Force -Path $OutDir, $logs | Out-Null
foreach ($dir in @($dist, $work)) {
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}
Get-ChildItem -LiteralPath $logs -File | Remove-Item -Force

$sha = (& git -C $repo rev-parse --verify "$Commit^{commit}")
if ($LASTEXITCODE -ne 0 -or -not $sha) { Fail "no commit $Commit" }
$sha = $sha.Trim()
Write-Host "release-dry-run: commit $sha"

$serverTags = @()
try {
    # A clean checkout of the commit.
    if (Test-Path -LiteralPath $src) {
        $null = Invoke-Logged 'git' @('-C', $repo, 'worktree', 'remove', '--force', $src) (Join-Path $logs 'worktree-old.log') (Join-Path $logs 'worktree-old.log.err')
        if (Test-Path -LiteralPath $src) { Remove-Item -LiteralPath $src -Recurse -Force }
        $null = Invoke-Logged 'git' @('-C', $repo, 'worktree', 'prune') (Join-Path $logs 'worktree-prune.log') (Join-Path $logs 'worktree-prune.log.err')
    }
    $null = Step 'worktree' 'git' @('-C', $repo, 'worktree', 'add', '--detach', $src, $sha)
    $epoch = ((& git -C $src log -1 --format=%ct) | Out-String).Trim()

    # What the workflow's first job checks.
    $cargoToml = Get-Content -LiteralPath (Join-Path $src 'Cargo.toml') -Raw
    $serverToml = Get-Content -LiteralPath (Join-Path $src 'server\Cargo.toml') -Raw
    $appVersion = if ($cargoToml -match '(?ms)^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"') { $Matches[1] } else { $null }
    $serverVersion = if ($serverToml -match '(?ms)^\[package\][^\[]*?^version\s*=\s*"([^"]+)"') { $Matches[1] } else { $null }
    if (-not $appVersion -or $appVersion -ne $serverVersion) { Fail "the app is $appVersion and the server is ${serverVersion}: a tag cannot match both" }
    $version = $appVersion
    Write-Host "release-dry-run: version $version (tag v$version)"
    $keyLines = @(Get-Content -LiteralPath (Join-Path $src 'release\minisign.pub'))
    $keyOk = $false
    if ($keyLines.Count -ge 2 -and $keyLines[0].StartsWith('untrusted comment:')) {
        try {
            $bytes = [Convert]::FromBase64String($keyLines[1].Trim())
            $keyOk = $bytes.Length -eq 42 -and $bytes[0] -eq 0x45 -and $bytes[1] -eq 0x64
        } catch { $keyOk = $false }
    }
    if (-not $keyOk) { Fail 'release/minisign.pub holds no minisign public key (docs/release.md, "The signing key")' }

    $null = Step 'docker-info' 'docker' @('info', '--format', '{{.OSType}}')
    if ($Fresh) {
        if (Test-Path -LiteralPath $windowsTarget) { Remove-Item -LiteralPath $windowsTarget -Recurse -Force }
        $null = Invoke-Logged 'docker' @('volume', 'rm', $linuxTarget, $linuxCargo) (Join-Path $logs 'fresh.log') (Join-Path $logs 'fresh.log.err')
    }
    $null = Step 'package-image' 'docker' @('build', '-f', (Join-Path $src 'tests\linux\package.Dockerfile'), '-t', $packageImage, (Join-Path $src 'tests\linux'))

    # The draft's notes.
    $notesLog = Step 'notes' 'docker' @('run', '--rm', '-v', "${src}:/src:ro", $packageImage, 'bash', 'scripts/release-notes.sh', $version)
    Copy-Item -LiteralPath $notesLog -Destination (Join-Path $OutDir 'notes.md') -Force

    # Windows: the workflow's script, in the checkout, with a target folder
    # of its own.
    $windowsOut = Join-Path $work 'windows'
    $savedTarget = $env:CARGO_TARGET_DIR
    $env:CARGO_TARGET_DIR = $windowsTarget
    try {
        $null = Step 'windows' 'powershell' @('-NoProfile', '-File', (Join-Path $src 'scripts\package-windows.ps1'), '-OutDir', $windowsOut)
    } finally {
        $env:CARGO_TARGET_DIR = $savedTarget
    }
    Copy-Item -LiteralPath (Join-Path $windowsOut "shellpp-$version-windows-x64.zip") -Destination $dist
    Copy-Item -LiteralPath (Join-Path $windowsOut 'shellpp.pdb') -Destination $dist

    # Linux: the workflow's image and script.
    $linuxOut = Join-Path $work 'linux'
    New-Item -ItemType Directory -Force -Path $linuxOut | Out-Null
    $null = Step 'linux' 'docker' @('run', '--rm', '--label', 'shellpp-run=release-dry-run', '-e', "SOURCE_DATE_EPOCH=$epoch",
        '-v', "${src}:/src:ro", '-v', "${linuxOut}:/out", '-v', "${linuxTarget}:/target", '-v', "${linuxCargo}:/usr/local/cargo/registry",
        $packageImage, 'bash', 'scripts/package-linux.sh', '/out')
    Copy-Item -LiteralPath (Join-Path $linuxOut "shellpp-$version-linux-x64.tar.gz") -Destination $dist

    # The server: the image, its archive, and the binary taken from it.
    $serverOut = Join-Path $work 'server'
    New-Item -ItemType Directory -Force -Path (Join-Path $serverOut 'bin') | Out-Null
    $serverTags = @("shellpp-server:$version", 'shellpp-server:latest')
    $null = Step 'server-image' 'docker' @('build', '-f', (Join-Path $src 'server\Dockerfile'), '-t', $serverTags[0], '-t', $serverTags[1], $src)
    $docLog = Step 'server-image-docs' 'docker' @('run', '--rm', '--entrypoint', 'ls', $serverTags[0], '/usr/share/doc/shellpp-server')
    Assert-Names 'the server image (/usr/share/doc/shellpp-server)' @(Get-Content -LiteralPath $docLog | Where-Object { $_ }) @('LICENSE', 'THIRD_PARTY_NOTICES.md')
    $imageTar = Join-Path $serverOut "shellpp-server-$version-docker-image.tar"
    $null = Step 'server-save' 'docker' @('save', '-o', $imageTar, $serverTags[0], $serverTags[1])
    $null = Step 'server-gzip' 'docker' @('run', '--rm', '-v', "${serverOut}:/out", $packageImage, 'gzip', '-n', '-9', "/out/shellpp-server-$version-docker-image.tar")
    $idLog = Step 'server-create' 'docker' @('create', $serverTags[0])
    $id = (Get-Content -LiteralPath $idLog | Select-Object -First 1).Trim()
    try {
        $null = Step 'server-cp' 'docker' @('cp', "${id}:/usr/local/bin/shellpp-server", (Join-Path $serverOut 'bin\shellpp-server'))
    } finally {
        $null = Invoke-Logged 'docker' @('rm', $id) (Join-Path $logs 'server-rm.log') (Join-Path $logs 'server-rm.log.err')
    }
    $null = Step 'server-archive' 'docker' @('run', '--rm', '-e', "SOURCE_DATE_EPOCH=$epoch", '-v', "${src}:/src:ro", '-v', "${serverOut}:/out",
        $packageImage, 'bash', 'scripts/package-server.sh', '/out/bin/shellpp-server', $version, '/out')
    Copy-Item -LiteralPath "$imageTar.gz" -Destination $dist
    Copy-Item -LiteralPath (Join-Path $serverOut "shellpp-server-$version-linux-x64.tar.gz") -Destination $dist

    # The update manifest and the checksums, as the workflow writes them.
    $files = @(Get-ChildItem -LiteralPath $dist -File | Sort-Object Name | ForEach-Object {
            [ordered]@{ name = $_.Name; sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant(); size = $_.Length }
        })
    $manifest = [ordered]@{
        version   = $version
        url       = "https://github.com/proxyboy228/shellpp/releases/tag/v$version"
        published = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
        files     = $files
    }
    Write-Utf8 (Join-Path $dist 'latest.json') (($manifest | ConvertTo-Json -Depth 4) + "`n")
    $sums = Get-ChildItem -LiteralPath $dist -File | Sort-Object Name | ForEach-Object {
        "$((Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant())  $($_.Name)"
    }
    Write-Utf8 (Join-Path $dist 'SHA256SUMS') (($sums -join "`n") + "`n")

    # What each archive holds, against the table of docs/release.md.
    $releaseDoc = Get-Content -LiteralPath (Join-Path $src 'docs\release.md') -Raw
    foreach ($name in @('shellpp-<version>-windows-x64.zip', 'shellpp-<version>-linux-x64.tar.gz', 'shellpp.pdb',
            'shellpp-server-<version>-docker-image.tar.gz', 'shellpp-server-<version>-linux-x64.tar.gz', 'latest.json', 'SHA256SUMS')) {
        if (-not $releaseDoc.Contains("| ``$name`` |")) { Fail "the table of docs/release.md has no row for $name" }
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead((Join-Path $dist "shellpp-$version-windows-x64.zip"))
    try { $zipNames = @($zip.Entries | ForEach-Object { $_.FullName }) } finally { $zip.Dispose() }
    Assert-Names 'the Windows zip' $zipNames @('shellpp.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'README-portable.txt')
    $linuxDir = "shellpp-$version-linux-x64"
    Assert-Names 'the Linux package' (Get-TarNames (Join-Path $dist "$linuxDir.tar.gz")) @(
        'shellpp', 'shellpp.desktop', 'shellpp.png', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'README-linux.txt' | ForEach-Object { "$linuxDir/$_" })
    $deploy = @(& git -C $src ls-files server/deploy | ForEach-Object { $_ -replace '^server/', '' })
    Assert-Names 'the server archive' (Get-TarNames (Join-Path $dist "shellpp-server-$version-linux-x64.tar.gz")) (
        @('shellpp-server', 'LICENSE', 'README.md', 'THIRD_PARTY_NOTICES.md') + $deploy)
    $imageManifest = & $tar -xzOf (Join-Path $dist "shellpp-server-$version-docker-image.tar.gz") manifest.json
    if ($LASTEXITCODE -ne 0) { Fail 'the server image archive has no manifest.json' }
    $repoTags = @((($imageManifest | Out-String) | ConvertFrom-Json) | ForEach-Object { $_.RepoTags } | Where-Object { $_ })
    Assert-Names 'the server image archive (tags)' $repoTags $serverTags

    Write-Host ''
    Get-ChildItem -LiteralPath $dist -File | Sort-Object Name | ForEach-Object {
        Write-Host ("  {0,-48} {1,12:N0} bytes" -f $_.Name, $_.Length)
    }
    Write-Host "release-dry-run: done; unsigned files in $dist, the notes in $(Join-Path $OutDir 'notes.md')" -ForegroundColor Green
} finally {
    foreach ($tag in $serverTags) {
        $null = Invoke-Logged 'docker' @('image', 'rm', $tag) (Join-Path $logs 'server-untag.log') (Join-Path $logs 'server-untag.log.err')
    }
    if (Test-Path -LiteralPath $src) {
        $null = Invoke-Logged 'git' @('-C', $repo, 'worktree', 'remove', '--force', $src) (Join-Path $logs 'worktree-remove.log') (Join-Path $logs 'worktree-remove.log.err')
    }
}
