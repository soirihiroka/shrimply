[CmdletBinding()]
param(
    [string]$FfmpegRoot = $env:FFMPEG_DIR,
    [string]$OutputDir,
    [switch]$RequireHardware
)

$ErrorActionPreference = "Stop"

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    ![Environment]::Is64BitOperatingSystem) {
    throw "FFmpeg validation requires 64-bit Windows"
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
if (!$FfmpegRoot) {
    $FfmpegRoot = Join-Path $root "target/windows-deps/installed/x64-windows"
}
if (!$OutputDir) {
    $OutputDir = Join-Path $root "target/windows-deps/validation"
}
$FfmpegRoot = [IO.Path]::GetFullPath($FfmpegRoot)
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
    @{ Name = "ffmpeg"; Version = "9.0.1" },
    @{ Name = "fdk-aac"; Version = "2.0.3" },
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
Assert-Contains $version '(?m)^ffmpeg version 9\.0\.1(?:\s|$)' "FFmpeg 9.0.1"
foreach ($option in @(
    "--toolchain=msvc",
    "--enable-shared",
    "--disable-static",
    "--enable-nonfree",
    "--enable-libfdk-aac",
    "--enable-libopus",
    "--enable-cuda",
    "--enable-ffnvcodec",
    "--enable-nvenc",
    "--enable-nvdec",
    "--enable-cuvid"
)) {
    Assert-Contains $version ([regex]::Escape($option)) "configure option $option"
}
foreach ($option in @("--enable-libx264", "--enable-libx265", "--enable-libopenh264")) {
    if ($version -match [regex]::Escape($option)) {
        throw "Pinned FFmpeg unexpectedly contains configure option $option"
    }
}

$encoders = Invoke-PinnedFfmpeg "encoders" @("-hide_banner", "-encoders")
foreach ($encoder in @("libfdk_aac", "libopus", "h264_nvenc", "hevc_nvenc")) {
    Assert-Contains $encoders "(?m)\b$encoder\b" "encoder $encoder"
}
foreach ($encoder in @("libx264", "libx265", "libopenh264")) {
    if ($encoders -match "(?m)\b$encoder\b") {
        throw "Pinned FFmpeg unexpectedly contains software encoder $encoder"
    }
}

$hwaccels = Invoke-PinnedFfmpeg "hwaccels" @("-hide_banner", "-hwaccels")
Assert-Contains $hwaccels '(?m)^cuda\r?$' "CUDA hardware acceleration"

Invoke-PinnedFfmpeg "libfdk-aac" @(
    "-hide_banner", "-loglevel", "verbose",
    "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000",
    "-t", "1", "-c:a", "libfdk_aac", "-b:a", "128k",
    "-y", (Join-Path $OutputDir "libfdk-aac.m4a")
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
