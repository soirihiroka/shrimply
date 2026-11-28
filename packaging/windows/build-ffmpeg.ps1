[CmdletBinding()]
param(
    [string]$VcpkgRoot = $env:VCPKG_ROOT,
    [string]$InstalledDir = $env:VCPKG_INSTALLED_DIR,
    [string]$BinarySources = $env:VCPKG_BINARY_SOURCES
)

$ErrorActionPreference = "Stop"

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    ![Environment]::Is64BitOperatingSystem) {
    throw "The Windows FFmpeg build requires 64-bit Windows"
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$manifestRoot = Join-Path $PSScriptRoot "vcpkg"
$configuration = Get-Content (Join-Path $manifestRoot "vcpkg.json") -Raw | ConvertFrom-Json
$revision = $configuration.'builtin-baseline'
if (!$revision -or $revision -notmatch '^[0-9a-f]{40}$') {
    throw "The FFmpeg vcpkg manifest does not contain a pinned baseline"
}
if (!$VcpkgRoot) {
    $VcpkgRoot = Join-Path $root "target/windows-deps/vcpkg"
}
if (!$InstalledDir) {
    $InstalledDir = Join-Path $root "target/windows-deps/installed"
}
if (!$BinarySources) {
    $binaryCache = Join-Path $root "target/windows-deps/binary-cache"
    New-Item -ItemType Directory -Force $binaryCache | Out-Null
    $BinarySources = "clear;files,$binaryCache,readwrite"
}
$VcpkgRoot = [IO.Path]::GetFullPath($VcpkgRoot)
$InstalledDir = [IO.Path]::GetFullPath($InstalledDir)

if (!(Test-Path $VcpkgRoot)) {
    New-Item -ItemType Directory -Force (Split-Path $VcpkgRoot -Parent) | Out-Null
    & git.exe clone --filter=blob:none https://github.com/microsoft/vcpkg.git $VcpkgRoot
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & git.exe -C $VcpkgRoot checkout --detach $revision
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} elseif (!(Test-Path (Join-Path $VcpkgRoot ".git"))) {
    throw "VCPKG_ROOT is not a Git checkout: $VcpkgRoot"
}

$actualRevision = (& git.exe -C $VcpkgRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
if ($actualRevision -ne $revision) {
    throw "VCPKG_ROOT is at $actualRevision; expected pinned revision $revision"
}

$vcpkg = Join-Path $VcpkgRoot "vcpkg.exe"
if (!(Test-Path $vcpkg)) {
    & (Join-Path $VcpkgRoot "bootstrap-vcpkg.bat") -disableMetrics
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

$env:VCPKG_DEFAULT_TRIPLET = "x64-windows"
$env:VCPKG_INSTALLED_DIR = $InstalledDir
$env:VCPKG_ROOT = $VcpkgRoot
$env:VCPKGRS_DYNAMIC = "1"
$env:VCPKG_BINARY_SOURCES = $BinarySources

$overlay = Join-Path $root "target/windows-deps/ports"
& uv.exe run --no-project --python 3.13 python (Join-Path $root "packaging/windows-ffmpeg.py") $VcpkgRoot $overlay
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $vcpkg install `
    --overlay-ports $overlay `
    --triplet x64-windows `
    --x-manifest-root $manifestRoot `
    --x-install-root $InstalledDir
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$ffmpegRoot = Join-Path $InstalledDir "x64-windows"
& (Join-Path $PSScriptRoot "verify-ffmpeg.ps1") -FfmpegRoot $ffmpegRoot -DevelopmentFilesOnly

Write-Host "Pinned vcpkg revision: $revision"
Write-Host "FFmpeg installation: $ffmpegRoot"
