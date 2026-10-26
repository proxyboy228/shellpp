<#
.SYNOPSIS
  Builds the signed release APK of the Android app (docs/mobile/release.md;
  D218, D223).

.DESCRIPTION
  The release signing key is the owner's: the path of its file and its
  alias come from a properties file outside the repository,
  %USERPROFILE%\.shellpp\android-signing.properties, or the file
  -SigningProperties names:

      storeFile=C:/Users/<you>/keys/shellpp-release.jks
      keyAlias=shellpp

  (forward slashes, or doubled backslashes). No password goes in it: the
  script asks for the keystore's password and the key's (hidden; Enter for
  the same one) and gives them to Gradle only in this process's
  environment, which ends with it; nothing is written to disk. Gradle runs
  without its daemon and without the configuration cache, so the passwords
  stay in neither.

  When SHELLPP_ANDROID_STORE_PASSWORD (and SHELLPP_ANDROID_KEY_PASSWORD) are
  already set, nothing is asked: verify-android.ps1 -Release builds this way
  with a throwaway key of its own, outside the repository.

  Prints the SHA-256 of the signing certificate (compare it with the one you
  noted when you made the key) and the path of the APK. Finds the Android
  SDK and the JDK as verify-android.ps1 does, for this process only.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\build-android-release.ps1
#>
param(
    # The signing file; default: %USERPROFILE%\.shellpp\android-signing.properties.
    [string] $SigningProperties = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
$android = Join-Path $repo 'apps\android'
$guide = 'docs\mobile\release.md'

if (-not $SigningProperties) {
    $SigningProperties = Join-Path $env:USERPROFILE '.shellpp\android-signing.properties'
}
if (-not (Test-Path -LiteralPath $SigningProperties -PathType Leaf)) {
    Write-Host "build-android-release: no signing file at $SigningProperties." -ForegroundColor Red
    Write-Host "Make the key and the file first, as $guide says (section 1)." -ForegroundColor Red
    exit 1
}
$SigningProperties = (Resolve-Path -LiteralPath $SigningProperties).Path

# The keystore the file names must be there (its path is not a secret).
$store = $null
foreach ($line in Get-Content -LiteralPath $SigningProperties -Encoding UTF8) {
    if ($line -match '^\s*storeFile\s*[=:]\s*(.+?)\s*$') { $store = $Matches[1] -replace '\\\\', '\' }
}
if (-not $store -or -not (Test-Path -LiteralPath $store -PathType Leaf)) {
    Write-Host "build-android-release: $SigningProperties names no keystore that exists (storeFile)." -ForegroundColor Red
    Write-Host "See $guide, section 1." -ForegroundColor Red
    exit 1
}

function Find-Sdk {
    foreach ($sdk in $env:ANDROID_HOME, $env:ANDROID_SDK_ROOT) {
        if ($sdk -and (Test-Path -LiteralPath (Join-Path $sdk 'platform-tools'))) { return $sdk }
    }
    if ($env:LOCALAPPDATA) {
        $sdk = Join-Path $env:LOCALAPPDATA 'Android\Sdk'
        if (Test-Path -LiteralPath (Join-Path $sdk 'platform-tools')) { return $sdk }
    }
    return $null
}

function Find-Jdk {
    if ($env:JAVA_HOME -and (Test-Path -LiteralPath (Join-Path $env:JAVA_HOME 'bin\java.exe'))) { return $env:JAVA_HOME }
    foreach ($root in $env:ProgramFiles, ${env:ProgramFiles(x86)}) {
        if (-not $root) { continue }
        $jbr = Join-Path $root 'Android\Android Studio\jbr'
        if (Test-Path -LiteralPath (Join-Path $jbr 'bin\java.exe')) { return $jbr }
    }
    return $null
}

# A password read hidden, as plain text for this process's environment only.
function Read-Secret([string] $Prompt) {
    $secure = Read-Host -Prompt $Prompt -AsSecureString
    $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try { return [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer) }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer) }
}

$sdk = Find-Sdk
$jdk = Find-Jdk
if (-not $sdk) { Write-Host 'build-android-release: no Android SDK (set ANDROID_HOME)' -ForegroundColor Red; exit 1 }
if (-not $jdk) { Write-Host 'build-android-release: no JDK (set JAVA_HOME or install Android Studio)' -ForegroundColor Red; exit 1 }
$env:ANDROID_HOME = $sdk
$env:JAVA_HOME = $jdk

$asked = $false
try {
    if (-not $env:SHELLPP_ANDROID_STORE_PASSWORD) {
        $asked = $true
        $env:SHELLPP_ANDROID_STORE_PASSWORD = Read-Secret 'The keystore password'
        $keyPassword = Read-Secret 'The key password (Enter: the same)'
        $env:SHELLPP_ANDROID_KEY_PASSWORD = if ($keyPassword) { $keyPassword } else { $env:SHELLPP_ANDROID_STORE_PASSWORD }
        $keyPassword = $null
    } elseif (-not $env:SHELLPP_ANDROID_KEY_PASSWORD) {
        $env:SHELLPP_ANDROID_KEY_PASSWORD = $env:SHELLPP_ANDROID_STORE_PASSWORD
    }
    $env:SHELLPP_ANDROID_SIGNING = $SigningProperties

    $apk = Join-Path $android 'app\build\outputs\apk\release\app-release.apk'
    if (Test-Path -LiteralPath $apk) { Remove-Item -Force -LiteralPath $apk }
    Push-Location $android
    try {
        & (Join-Path $android 'gradlew.bat') 'assembleRelease' '--no-daemon' '--no-configuration-cache' '--console=plain'
        $built = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    if ($built -ne 0) { Write-Host "build-android-release: Gradle failed (exit $built)" -ForegroundColor Red; exit 1 }
    if (-not (Test-Path -LiteralPath $apk)) {
        Write-Host 'build-android-release: no signed APK was made (check the key, its alias and the passwords)' -ForegroundColor Red
        exit 1
    }

    # The signing certificate, as Android sees it.
    $apksigner = Get-ChildItem -LiteralPath (Join-Path $sdk 'build-tools') -Directory |
        Sort-Object { try { [version]$_.Name } catch { [version]'0.0' } } -Descending |
        ForEach-Object { Join-Path $_.FullName 'apksigner.bat' } |
        Where-Object { Test-Path -LiteralPath $_ } |
        Select-Object -First 1
    if (-not $apksigner) { Write-Host 'build-android-release: no apksigner in the SDK build-tools' -ForegroundColor Red; exit 1 }
    # apksigner's JVM warns on stderr; PowerShell 5.1 would make that an
    # error, so stderr goes to a file of its own.
    $ErrorActionPreference = 'Continue'
    $warnings = [IO.Path]::GetTempFileName()
    $certificate = & $apksigner 'verify' '--print-certs' $apk 2> $warnings
    $verified = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $warnings
    if ($verified -ne 0) { Write-Host "build-android-release: the APK's signature does not verify: $certificate" -ForegroundColor Red; exit 1 }
    $sha256 = ($certificate | Select-String -Pattern 'SHA-256 digest:\s*([0-9a-fA-F]+)' | Select-Object -First 1).Matches[0].Groups[1].Value
    Write-Host ''
    Write-Host "Signed APK:         $apk" -ForegroundColor Green
    Write-Host "Certificate SHA-256: $sha256" -ForegroundColor Green
    exit 0
} finally {
    # The passwords leave this process's environment (the child processes
    # that had them have ended).
    if ($asked) {
        Remove-Item -ErrorAction SilentlyContinue Env:SHELLPP_ANDROID_STORE_PASSWORD
        Remove-Item -ErrorAction SilentlyContinue Env:SHELLPP_ANDROID_KEY_PASSWORD
    }
    Remove-Item -ErrorAction SilentlyContinue Env:SHELLPP_ANDROID_SIGNING
}
