<#
.SYNOPSIS
  The Windows release artifact of Shell++: a portable zip of the release
  build (run 4, plan 9.5). No signing, no publishing.

.DESCRIPTION
  Builds shellpp.exe with the dist profile (fat LTO; Cargo.toml, unless
  -NoBuild) and writes
  <OutDir>\shellpp-<version>-windows-x64.zip with:
  - shellpp.exe (its version resource comes from Cargo.toml);
  - LICENSE and THIRD_PARTY_NOTICES.md;
  - README-portable.txt: where the data lives and how to keep it next to
    the program (SHELLPP_DATA_DIR).
  and <zip>.sha256 next to it, and shellpp.pdb (line tables, for crash
  reports; not in the zip). The MSI waits for the owner's choice of an
  installer tool (docs/decisions.md, D82).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1 -NoBuild -OutDir D:\out
#>
param(
    # Output folder; default <repo>\target\package.
    [string] $OutDir,
    # Use the existing target\dist\shellpp.exe.
    [switch] $NoBuild
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'target\package' }
# A relative folder is the caller's, not the repository's.
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path (Get-Location).Path $OutDir }
$OutDir = [System.IO.Path]::GetFullPath($OutDir)

function Fail([string] $message) {
    Write-Host "package-windows: $message" -ForegroundColor Red
    exit 1
}

Push-Location $repo
try {
    $metadata = & cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { Fail 'cargo metadata failed' }
    $package = $metadata.packages | Where-Object { $_.name -eq 'shellpp-desktop' } | Select-Object -First 1
    if (-not $package) { Fail 'no shellpp-desktop package' }
    $version = $package.version

    if (-not $NoBuild) {
        Write-Host 'package-windows: cargo build --locked --profile dist -p shellpp-desktop'
        & cargo build --locked --profile dist -p shellpp-desktop
        if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
    }
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
    $exe = Join-Path $targetDir 'dist\shellpp.exe'
    $pdb = Join-Path $targetDir 'dist\shellpp.pdb'
    if (-not (Test-Path -LiteralPath $exe)) { Fail "no $exe" }

    $name = "shellpp-$version-windows-x64"
    $staging = Join-Path $OutDir $name
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $staging | Out-Null
    Copy-Item -LiteralPath $exe -Destination $staging
    Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination $staging
    Copy-Item -LiteralPath (Join-Path $repo 'THIRD_PARTY_NOTICES.md') -Destination $staging
    $readme = @"
Shell++ $version for Windows (x64), portable

Run shellpp.exe. Nothing is installed. The app writes only its data
folder, and, while "Remember on this device" is on, its device key in the
Windows Credential Manager of this user.

Data (vaults, recordings, backups): %LOCALAPPDATA%\shellpp\
Diagnostic logs: %LOCALAPPDATA%\shellpp\logs\

To keep the data next to the program instead (for example on a USB drive),
start it with the SHELLPP_DATA_DIR environment variable, for example from
a .cmd file in this folder:

    set SHELLPP_DATA_DIR=%~dp0data
    start "" "%~dp0shellpp.exe"

The data is encrypted with your master password; keep the recovery key
apart from it.
"@
    Set-Content -LiteralPath (Join-Path $staging 'README-portable.txt') -Value $readme -Encoding utf8

    $zip = Join-Path $OutDir "$name.zip"
    if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
    Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zip
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$zip.sha256" -Value "$hash  $name.zip" -Encoding ascii
    Remove-Item -LiteralPath $staging -Recurse -Force

    if (Test-Path -LiteralPath $pdb) { Copy-Item -LiteralPath $pdb -Destination $OutDir -Force }
    $exeSize = (Get-Item -LiteralPath $exe).Length
    $zipSize = (Get-Item -LiteralPath $zip).Length
    Write-Host ("package-windows: {0} ({1:N1} MB; shellpp.exe {2:N1} MB)" -f $zip, ($zipSize / 1MB), ($exeSize / 1MB))
}
finally {
    Pop-Location
}
