[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$VcpkgRoot,
    [Parameter(Mandatory = $true)]
    [string]$InstalledDir,
    [string]$Qmake = "qmake.exe",
    [Parameter(Mandatory = $true)]
    [string]$RustToolchain,
    [Parameter(Mandatory = $true)]
    [string]$CargoTargetDir,
    [Parameter(Mandatory = $true)]
    [string]$CudaHome,
    [Parameter(Mandatory = $true)]
    [string]$CudaImageFormat,
    [Parameter(Mandatory = $true)]
    [string]$CudaTarget,
    [Parameter(Mandatory = $true)]
    [string]$CudaPtxTarget,
    [Parameter(Mandatory = $true)]
    [string]$CudaHostCxx,
    [string]$CudaAllowUnsupportedCompiler,
    [Parameter(Mandatory = $true)]
    [string]$OptixRoot,
    [Parameter(Mandatory = $true)]
    [string]$QtEditorPackage,
    [Parameter(Mandatory = $true)]
    [string]$QtLauncherPackage
)

$ErrorActionPreference = "Stop"

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    ![Environment]::Is64BitOperatingSystem) {
    throw "The Windows release build requires 64-bit Windows"
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
function Resolve-ProjectPath([string]$Path) {
    if ([IO.Path]::IsPathRooted($Path)) {
        return [IO.Path]::GetFullPath($Path)
    }
    return [IO.Path]::GetFullPath((Join-Path $root $Path))
}

$VcpkgRoot = [IO.Path]::GetFullPath($VcpkgRoot)
$InstalledDir = [IO.Path]::GetFullPath($InstalledDir)
$CargoTargetDir = Resolve-ProjectPath $CargoTargetDir
$CudaHome = [IO.Path]::GetFullPath($CudaHome)
$OptixRoot = [IO.Path]::GetFullPath($OptixRoot)
$ffmpegRoot = Join-Path $InstalledDir "x64-windows"
$pkgConfig = Join-Path $ffmpegRoot "tools/pkgconf/pkgconf.exe"
$openCvDir = Join-Path $ffmpegRoot "share/opencv4"

foreach ($path in @(
    (Join-Path $VcpkgRoot ".vcpkg-root"),
    $pkgConfig,
    (Join-Path $ffmpegRoot "include/opencv4/opencv2/core/version.hpp"),
    (Join-Path $openCvDir "OpenCVConfig.cmake"),
    (Join-Path $ffmpegRoot "share/ffmpeg/vcpkg-cmake-wrapper.cmake")
)) {
    if (!(Test-Path $path)) {
        throw "The isolated Windows dependency tree is missing: $path"
    }
}

$compiler = (Get-Command cl.exe -ErrorAction Stop).Source
$linker = (Get-Command link.exe -ErrorAction Stop).Source
$linkerInfo = (Get-Item -LiteralPath $linker).VersionInfo
if ($linkerInfo.CompanyName -notlike "Microsoft*") {
    throw "MSVC link.exe not found: $linker"
}
$nvcc = (Get-Command nvcc.exe -ErrorAction Stop).Source
$qmakePath = (Get-Command $Qmake -ErrorAction Stop).Source
$qtVersion = (& $qmakePath -query QT_VERSION).Trim()
if ($LASTEXITCODE -ne 0 -or $qtVersion -notmatch '^6\.') {
    throw "$qmakePath selected unsupported Qt $qtVersion; Qt 6 is required"
}

switch ($CudaImageFormat) {
    "cubin" {
        if ($CudaTarget -notmatch '^sm_[0-9]+$') {
            throw "CUDA_TARGET=$CudaTarget must be a physical SM architecture"
        }
    }
    "ptx" {
        if ($CudaPtxTarget -notmatch '^compute_[0-9]+$') {
            throw "CUDA_PTX_TARGET=$CudaPtxTarget must be a virtual compute architecture"
        }
    }
    default {
        throw "CUDA_IMAGE_FORMAT=$CudaImageFormat is unsupported; expected cubin or ptx"
    }
}

$env:VCPKG_ROOT = $VcpkgRoot
$env:VCPKG_INSTALLED_DIR = $InstalledDir
$env:VCPKGRS_DYNAMIC = "1"
$env:FFMPEG_DIR = $ffmpegRoot
$env:PKG_CONFIG = $pkgConfig
$env:PKG_CONFIG_PATH = "$(Join-Path $ffmpegRoot 'lib/pkgconfig');$(Join-Path $ffmpegRoot 'share/pkgconfig')"
$env:PATH = "$(Join-Path $ffmpegRoot 'bin');$env:PATH"
$env:OPENCV_DISABLE_PROBES = "environment,pkg_config,cmake,vcpkg"
$cmakeInstalledDir = $InstalledDir.Replace('\', '/')
$cmakeOpenCvDir = $openCvDir.Replace('\', '/')
$env:OPENCV_CMAKE_ARGS = "-DVCPKG_INSTALLED_DIR=$cmakeInstalledDir -DVCPKG_TARGET_TRIPLET=x64-windows -DOpenCV_DIR=$cmakeOpenCvDir"
$opencvLibraries = @(
    Get-ChildItem -LiteralPath (Join-Path $ffmpegRoot "lib") -Filter "opencv_*.lib"
)
if ($opencvLibraries.Count -eq 0) {
    throw "No OpenCV import libraries found in $ffmpegRoot\lib"
}
$env:OPENCV_LINK_LIBS = (
    $opencvLibraries |
        Sort-Object Name |
        ForEach-Object { "dylib=$($_.BaseName)" }
) -join ","
$env:CUDA_HOME = $CudaHome
$env:CUDA_TOOLKIT_PATH = $CudaHome
$env:CUDA_IMAGE_FORMAT = $CudaImageFormat
$env:CUDA_TARGET = $CudaTarget
$env:CUDA_PTX_TARGET = $CudaPtxTarget
$env:CUDA_HOST_CXX = $CudaHostCxx
$env:CUDA_ALLOW_UNSUPPORTED_COMPILER = $CudaAllowUnsupportedCompiler
$env:OPTIX_ROOT = $OptixRoot
$env:CARGO_TARGET_DIR = $CargoTargetDir
$env:CARGO_TERM_COLOR = "always"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $linker
$env:QMAKE = $qmakePath

& $pkgConfig --exists libavcodec libavformat libavfilter libavdevice libswresample libswscale
if ($LASTEXITCODE -ne 0) {
    foreach ($path in @(
        "include/libavutil/avutil.h",
        "lib/avutil.lib",
        "lib/avcodec.lib",
        "lib/avformat.lib"
    )) {
        if (!(Test-Path (Join-Path $ffmpegRoot $path))) {
            throw "Missing FFmpeg development file: $path"
        }
    }
}

& $pkgConfig --exists poppler-glib pango pangocairo rubberband
if ($LASTEXITCODE -ne 0) {
    throw "The isolated dependency tree is missing required application libraries"
}

$rustup = (Get-Command rustup.exe -ErrorAction Stop).Source
function Invoke-Cargo([string[]]$Arguments) {
    & $rustup run $RustToolchain cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Write-Host "MSVC compiler: $compiler"
Write-Host "MSVC linker: $linker"
Write-Host "CUDA compiler: $nvcc"
Write-Host "Qt: $qtVersion via $qmakePath"
Write-Host "Windows dependencies: $ffmpegRoot"

Push-Location $root
try {
    Invoke-Cargo @("build", "-p", "shrimply-render-kernels-cuda")
    Invoke-Cargo @(
        "build", "--release",
        "-p", $QtEditorPackage,
        "-p", $QtLauncherPackage
    )
} finally {
    Pop-Location
}
