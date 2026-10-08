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
$manifest = Join-Path $PSScriptRoot "vcpkg/ffmpeg"
$configuration = Get-Content (Join-Path $manifest "vcpkg.json") -Raw | ConvertFrom-Json
$revision = $configuration.'builtin-baseline'
if (!$revision -or $revision -notmatch '^[0-9a-f]{40}$') {
    throw "The FFmpeg vcpkg manifest does not contain a pinned baseline"
}
$ffmpegDependencies = @($configuration.dependencies | Where-Object { $_.name -eq "ffmpeg" })
if ($ffmpegDependencies.Count -ne 1) {
    throw "The FFmpeg vcpkg manifest must contain exactly one FFmpeg dependency"
}
$ffmpegDependency = $ffmpegDependencies[0]
if ($ffmpegDependency.'default-features' -ne $false) {
    throw "The FFmpeg vcpkg dependency must disable default features"
}
$ffmpegFeatures = @("core") + @($ffmpegDependency.features)
$ffmpegPackage = "ffmpeg[$($ffmpegFeatures -join ',')]:x64-windows"

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

Push-Location $VcpkgRoot
try {
    & $vcpkg install `
        $ffmpegPackage `
        --triplet x64-windows `
        --x-install-root $InstalledDir
    $vcpkgExitCode = $LASTEXITCODE
} finally {
    Pop-Location
}
if ($vcpkgExitCode -ne 0) { exit $vcpkgExitCode }

$ffmpegRoot = Join-Path $InstalledDir "x64-windows"
foreach ($path in @(
    "include/libavutil/avutil.h",
    "lib/avutil.lib",
    "lib/avcodec.lib",
    "lib/avformat.lib"
)) {
    if (!(Test-Path (Join-Path $ffmpegRoot $path))) {
        throw "Pinned FFmpeg installation is missing $path"
    }
}

Write-Host "Pinned vcpkg revision: $revision"
Write-Host "FFmpeg installation: $ffmpegRoot"
