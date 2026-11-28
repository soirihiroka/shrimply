[CmdletBinding()]
param(
    [string]$FfmpegRoot = $env:FFMPEG_DIR,
    [string]$OutputDir,
    [switch]$RequireHardware,
    [switch]$DevelopmentFilesOnly
)

$ErrorActionPreference = "Stop"
$ffmpegVersion = (Get-Content (Join-Path $PSScriptRoot "../../external/ffmpeg/RELEASE") -Raw).Trim()

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    ![Environment]::Is64BitOperatingSystem) {
    throw "FFmpeg validation requires 64-bit Windows"
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
if (!$FfmpegRoot) {
    $FfmpegRoot = Join-Path $root "target/windows-deps/installed/x64-windows"
}
$FfmpegRoot = [IO.Path]::GetFullPath($FfmpegRoot)
foreach ($path in @("include/libavutil/ffversion.h", "lib/avutil.lib", "lib/avcodec.lib", "lib/avformat.lib")) {
    if (!(Test-Path (Join-Path $FfmpegRoot $path))) {
        throw "Missing FFmpeg development file: $path"
    }
}
$versionHeader = Get-Content (Join-Path $FfmpegRoot "include/libavutil/ffversion.h")
if ($versionHeader -notcontains "#define FFMPEG_VERSION `"$ffmpegVersion`"") {
    throw "Expected FFmpeg $ffmpegVersion in $FfmpegRoot. Run packaging/windows/build-ffmpeg.ps1."
}
if ($DevelopmentFilesOnly) { return }
if (!$OutputDir) {
    $OutputDir = Join-Path $root "target/windows-deps/validation"
}
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
New-Item -ItemType Directory -Force $OutputDir | Out-Null

$ffmpeg = @(
    (Join-Path $FfmpegRoot "tools/ffmpeg/ffmpeg.exe"),
    (Join-Path $FfmpegRoot "bin/ffmpeg.exe")
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (!$ffmpeg) {
    throw "The pinned installation does not contain ffmpeg.exe: $FfmpegRoot"
}
$ffmpeg = (Resolve-Path $ffmpeg).Path
$expectedPrefix = $FfmpegRoot.TrimEnd('\') + '\'
if (!$ffmpeg.StartsWith($expectedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "FFmpeg resolved outside the pinned installation: $ffmpeg"
}
$ffmpegBin = Join-Path $FfmpegRoot "bin"
$env:PATH = "$ffmpegBin;$env:PATH"

function Invoke-PinnedFfmpeg {
    param(
        [string]$Name,
        [string[]]$Arguments
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & $ffmpeg @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $text = ($output | Out-String).TrimEnd()
    [IO.File]::WriteAllText(
        (Join-Path $OutputDir "$Name.log"),
        "$ffmpeg $($Arguments -join ' ')`r`n$text`r`n",
        [Text.UTF8Encoding]::new($false)
    )
    if ($exitCode -ne 0) {
        throw "$Name failed with exit code $exitCode`n$text"
    }
    return $text
}

function Assert-Contains {
    param(
        [string]$Text,
        [string]$Pattern,
        [string]$Description
    )

    if ($Text -notmatch $Pattern) {
        throw "Pinned FFmpeg is missing $Description"
    }
}

$statusPath = Join-Path (Split-Path $FfmpegRoot -Parent) "vcpkg/status"
if (!(Test-Path $statusPath)) {
    throw "The pinned installation has no vcpkg status file: $statusPath"
}
$status = Get-Content $statusPath -Raw
foreach ($package in @(
    @{ Name = "ffmpeg"; Version = $ffmpegVersion },
    @{ Name = "ffnvcodec"; Version = "13.0.19.0" },
    @{ Name = "opus"; Version = "1.6.1" }
)) {
    $blocks = $status -split '(?:\r?\n){2,}' | Where-Object {
        $_ -match "(?m)^Package: $([regex]::Escape($package.Name))\r?$" -and
        $_ -match "(?m)^Version: $([regex]::Escape($package.Version))\r?$" -and
        $_ -match '(?m)^Architecture: x64-windows\r?$'
    }
    if (!$blocks) {
        throw "Pinned package is missing: $($package.Name) $($package.Version) x64-windows"
    }
}

$version = Invoke-PinnedFfmpeg "version" @("-hide_banner", "-version")
Assert-Contains $version "(?m)^ffmpeg version $([regex]::Escape($ffmpegVersion))(?:\s|$)" "FFmpeg $ffmpegVersion"
foreach ($option in @(
    "--toolchain=msvc",
    "--enable-shared",
    "--disable-static",
    "--enable-libmp3lame",
    "--enable-libopus",
    "--enable-libvorbis",
    "--enable-libx264",
    "--enable-libx265",
    "--enable-cuda",
    "--enable-ffnvcodec",
    "--enable-nvenc",
    "--enable-nvdec",
    "--enable-cuvid"
)) {
    Assert-Contains $version ([regex]::Escape($option)) "configure option $option"
}
$encoders = Invoke-PinnedFfmpeg "encoders" @("-hide_banner", "-encoders")
foreach ($encoder in @("aac", "libmp3lame", "libopus", "libvorbis", "h264_nvenc", "hevc_nvenc", "libx264", "libx265")) {
    Assert-Contains $encoders "(?m)\b$encoder\b" "encoder $encoder"
}

$hwaccels = Invoke-PinnedFfmpeg "hwaccels" @("-hide_banner", "-hwaccels")
Assert-Contains $hwaccels '(?m)^cuda\r?$' "CUDA hardware acceleration"

Invoke-PinnedFfmpeg "aac" @(
    "-hide_banner", "-loglevel", "verbose",
    "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000",
    "-t", "1", "-c:a", "aac", "-b:a", "128k",
    "-y", (Join-Path $OutputDir "aac.m4a")
) | Out-Null
Invoke-PinnedFfmpeg "libopus" @(
    "-hide_banner", "-loglevel", "verbose",
    "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000",
    "-t", "1", "-c:a", "libopus", "-b:a", "128k",
    "-y", (Join-Path $OutputDir "libopus.opus")
) | Out-Null

$nvidiaSmi = Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue
$hasNvidiaGpu = $false
if ($nvidiaSmi) {
    $gpuOutput = & $nvidiaSmi.Source --query-gpu=name,driver_version --format=csv,noheader 2>&1
    $hasNvidiaGpu = $LASTEXITCODE -eq 0 -and ($gpuOutput | Out-String).Trim()
    [IO.File]::WriteAllText(
        (Join-Path $OutputDir "nvidia-smi.log"),
        ($gpuOutput | Out-String),
        [Text.UTF8Encoding]::new($false)
    )
}
if (!$hasNvidiaGpu) {
    if ($RequireHardware) {
        throw "NVIDIA hardware validation was required, but nvidia-smi found no usable GPU"
    }
    Write-Warning "No usable NVIDIA GPU was detected; NVENC and CUDA-frame runtime tests were not run"
} else {
    Invoke-PinnedFfmpeg "h264-nvenc" @(
        "-hide_banner", "-loglevel", "verbose",
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30",
        "-frames:v", "30", "-c:v", "h264_nvenc", "-preset", "p4",
        "-y", (Join-Path $OutputDir "h264-nvenc.mp4")
    ) | Out-Null
    Invoke-PinnedFfmpeg "hevc-nvenc" @(
        "-hide_banner", "-loglevel", "verbose",
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30",
        "-frames:v", "30", "-c:v", "hevc_nvenc", "-preset", "p4",
        "-y", (Join-Path $OutputDir "hevc-nvenc.mp4")
    ) | Out-Null
    Invoke-PinnedFfmpeg "cuda-frames-h264-nvenc" @(
        "-hide_banner", "-loglevel", "verbose",
        "-init_hw_device", "cuda=cuda:0", "-filter_hw_device", "cuda",
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30",
        "-vf", "format=nv12,hwupload", "-frames:v", "30",
        "-c:v", "h264_nvenc", "-preset", "p4",
        "-y", (Join-Path $OutputDir "cuda-frames-h264-nvenc.mp4")
    ) | Out-Null
}

Write-Host "Validated FFmpeg executable: $ffmpeg"
Write-Host "Validation outputs: $OutputDir"
