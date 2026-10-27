<#
.SYNOPSIS
  The Linux app itself in containers (D225, item 3): the package in a clean
  Ubuntu 22.04 and the app's scenario under Xvfb. Outside the full
  criterion, like the walks of the Android app.

.DESCRIPTION
  Two layers, each in tests/linux/desktop.Dockerfile (Ubuntu 22.04 with only
  the run-time libraries README lists, Xvfb, Mesa's OpenGL through EGL
  (llvmpipe), a D-Bus session and GNOME Keyring as the Secret Service,
  OpenSSH as the test server):

  1. The package (tests/linux/desktop-check.sh): the tar.gz of
     scripts/package-linux.sh, -Package or built here in the package image
     (tests/linux/package.Dockerfile); desktop-file-validate and ldd of its
     files; the executable started and driven with xdotool: first run, the
     device key in the Secret Service, lock and unlock, a restart that opens
     with the device key, a quick connect to OpenSSH and a command, the
     SFTP panel listing the server's folder; then without a Secret Service
     and without D-Bus.
  2. Both themes (tests/linux/screens-check.sh): the build with developer
     tools, made in the package image, on demo data with --theme light and
     --theme dark (the released app follows the system's theme, which a
     container cannot set).

  Mesa's software Vulkan driver (lavapipe) is left out on purpose: under
  Xvfb the frames of GPUI's renderer on it never reach the screen (run 17).
  The screenshots and logs land in <repo>\target\linux-desktop\package and
  ...\screens; the builds keep their target folder in the Docker volume
  shellpp-package-target. Needs Docker with Linux containers and the
  network for the first builds.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify-linux-desktop.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify-linux-desktop.ps1 -Package target\release-dry-run\dist\shellpp-1.0.0-rc.1-linux-x64.tar.gz
#>
param(
    # A tar.gz of scripts/package-linux.sh; without it the package is built.
    [string] $Package,
    # Only the first layer.
    [switch] $NoScreens
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$outRoot = Join-Path $repo 'target\linux-desktop'
$logs = Join-Path $outRoot 'logs'
$run = if ($env:SHELLPP_TEST_RUN) { $env:SHELLPP_TEST_RUN } else { 'local' }
$packageImage = 'shellpp-package:local'
$desktopImage = 'shellpp-desktop-check:local'
$tests = Join-Path $repo 'tests\linux'

. (Join-Path $PSScriptRoot 'process.ps1')

function Fail([string] $message) {
    Write-Host "verify-linux-desktop: $message" -ForegroundColor Red
    exit 1
}

function Step([string] $name, [string[]] $arguments) {
    Write-Host "verify-linux-desktop: $name"
    $log = Join-Path $logs "$name.log"
    $code = Invoke-Logged 'docker' $arguments $log "$log.err"
    return @{ Code = $code; Log = $log }
}

foreach ($dir in @((Join-Path $outRoot 'package'), (Join-Path $outRoot 'screens'), $logs)) {
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

$info = Step 'docker-info' @('info', '--format', '{{.OSType}}')
if ($info.Code -ne 0) { Fail "Docker does not answer ($($info.Log))" }
foreach ($image in @(@($packageImage, 'package.Dockerfile'), @($desktopImage, 'desktop.Dockerfile'))) {
    $built = Step "image-$($image[1])" @('build', '-f', (Join-Path $tests $image[1]), '-t', $image[0], $tests)
    if ($built.Code -ne 0) { Fail "the image $($image[1]) did not build ($($built.Log))" }
}

if (-not $Package) {
    $pkgOut = Join-Path $outRoot 'built'
    if (Test-Path -LiteralPath $pkgOut) { Remove-Item -LiteralPath $pkgOut -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $pkgOut | Out-Null
    $built = Step 'package' @('run', '--rm', '--label', "shellpp-run=$run", '-v', "${repo}:/src:ro", '-v', "${pkgOut}:/out",
        '-v', 'shellpp-package-target:/target', '-v', 'shellpp-package-cargo:/usr/local/cargo/registry',
        $packageImage, 'bash', 'scripts/package-linux.sh', '/out')
    if ($built.Code -ne 0) { Fail "the package did not build ($($built.Log))" }
    $Package = (Get-ChildItem -LiteralPath $pkgOut -Filter 'shellpp-*-linux-x64.tar.gz' | Select-Object -First 1).FullName
}
if (-not [System.IO.Path]::IsPathRooted($Package)) { $Package = Join-Path (Get-Location).Path $Package }
if (-not (Test-Path -LiteralPath $Package)) { Fail "no package $Package" }
$pkgDir = Split-Path -Parent $Package
$pkgName = Split-Path -Leaf $Package

$failed = @()
$layer1 = Step 'layer1-package' @('run', '--rm', '--label', "shellpp-run=$run", '-v', "${pkgDir}:/pkg:ro",
    '-v', "$(Join-Path $outRoot 'package'):/out", '-v', "${tests}:/check:ro",
    $desktopImage, 'bash', '/check/desktop-check.sh', "/pkg/$pkgName", '/out')
Get-Content -LiteralPath (Join-Path $outRoot 'package\results.txt') -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "  $_" }
if ($layer1.Code -ne 0) { $failed += 'package' }

if (-not $NoScreens) {
    $build = Step 'screens-build' @('run', '--rm', '--label', "shellpp-run=$run", '-v', "${repo}:/src:ro",
        '-v', 'shellpp-package-target:/target', '-v', 'shellpp-package-cargo:/usr/local/cargo/registry',
        $packageImage, 'cargo', 'build', '--locked', '-p', 'shellpp-desktop', '--features', 'dev-tools')
    if ($build.Code -ne 0) {
        $failed += 'screens-build'
    } else {
        $layer2 = Step 'layer2-screens' @('run', '--rm', '--label', "shellpp-run=$run", '--user', 'tester', '-e', 'HOME=/home/tester',
            '-v', 'shellpp-package-target:/target:ro', '-v', "$(Join-Path $outRoot 'screens'):/out", '-v', "${tests}:/check:ro",
            $desktopImage, 'bash', '/check/screens-check.sh', '/target/debug/shellpp', '/out')
        Get-Content -LiteralPath $layer2.Log | ForEach-Object { Write-Host "  $_" }
        if ($layer2.Code -ne 0) { $failed += 'screens' }
    }
}

if ($failed.Count -gt 0) { Fail "failed: $($failed -join ', '); logs in $logs, screenshots in $outRoot" }
Write-Host "verify-linux-desktop: done; screenshots in $outRoot" -ForegroundColor Green
