$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$dist = Join-Path $root "dist"
$stage = Join-Path $dist "shrimply-windows-x86_64"
$archive = Join-Path $dist "shrimply-windows-x86_64.zip"
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root "target" }
$release = Join-Path $target "release"
$installed = if ($env:VCPKG_INSTALLED_DIR) {
    [IO.Path]::GetFullPath($env:VCPKG_INSTALLED_DIR)
} else {
    Join-Path $root "target/windows-deps/installed"
}
$vcpkgRoot = Join-Path $installed "x64-windows"
$ffmpegRoot = if ($env:FFMPEG_DIR) {
    [IO.Path]::GetFullPath($env:FFMPEG_DIR)
} else {
    $vcpkgRoot
}
if ($ffmpegRoot -ne $vcpkgRoot) {
    throw "FFMPEG_DIR must match the active vcpkg installation: $vcpkgRoot"
}
$vcpkgBin = Join-Path $vcpkgRoot "bin"
$ffmpeg = Join-Path $ffmpegRoot "tools/ffmpeg/ffmpeg.exe"
$ffmpegDlls = @(
    "avcodec-63.dll",
    "avdevice-63.dll",
    "avfilter-12.dll",
    "avformat-63.dll",
    "avutil-61.dll",
    "swresample-7.dll",
    "swscale-10.dll",
    "fdk-aac.dll",
    "opus.dll"
)

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
if (Test-Path $archive) { Remove-Item -Force $archive }
New-Item -ItemType Directory -Force $stage | Out-Null

$qmlDirs = @{
    "shrimply-qt.exe" = @(
        "crates/binaries/launcher-qt/qml"
    )
    "shrimply-editor-qt.exe" = @(
        "crates/binaries/editor-qt/qml"
        "crates/ui/components/components-qt/qml"
        "crates/ui/export/export-qt/qml"
        "crates/ui/inspector/inspector-qt/qml"
        "crates/ui/preferences/preferences-qt/qml"
    )
}

foreach ($name in @("shrimply-qt.exe", "shrimply-editor-qt.exe")) {
    $source = Join-Path $release $name
    if (!(Test-Path $source)) { throw "Missing release executable: $source" }
    Copy-Item $source $stage
    foreach ($qmlDir in $qmlDirs[$name]) {
        $qmlPath = Join-Path $root $qmlDir
        & windeployqt.exe --release --qmldir $qmlPath (Join-Path $stage $name)
        if ($LASTEXITCODE -ne 0) {
            throw "windeployqt failed for $name while scanning $qmlPath"
        }
    }
}

if (!(Test-Path $vcpkgBin)) { throw "Missing vcpkg runtime directory: $vcpkgBin" }
foreach ($name in $ffmpegDlls) {
    if (!(Test-Path (Join-Path $vcpkgBin $name))) {
        throw "Pinned FFmpeg runtime is missing $name"
    }
}
Copy-Item (Join-Path $vcpkgBin "*.dll") $stage

if (!(Test-Path $ffmpeg)) { throw "Missing pinned FFmpeg executable: $ffmpeg" }
$previousErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = "Continue"
    $configuration = & $ffmpeg -hide_banner -version 2>&1
    $ffmpegExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($ffmpegExitCode -ne 0) { throw "Could not record the pinned FFmpeg configuration" }

$licenses = Join-Path $stage "licenses"
New-Item -ItemType Directory -Force $licenses | Out-Null
foreach ($license in @(
    @{ Package = "ffmpeg"; Name = "ffmpeg.txt" },
    @{ Package = "fdk-aac"; Name = "fdk-aac.txt" },
    @{ Package = "ffnvcodec"; Name = "nv-codec-headers.txt" },
    @{ Package = "opus"; Name = "opus.txt" }
)) {
    $source = Join-Path $ffmpegRoot "share/$($license.Package)/copyright"
    if (!(Test-Path $source)) { throw "Missing license file: $source" }
    Copy-Item $source (Join-Path $licenses $license.Name)
}
[IO.File]::WriteAllText(
    (Join-Path $licenses "ffmpeg-build-configuration.txt"),
    "$(($configuration | Out-String).TrimEnd())`r`n",
    [Text.UTF8Encoding]::new($false)
)

$uv = (Get-Command uv.exe -ErrorAction Stop).Source
Copy-Item $uv (Join-Path $stage "uv.exe")
$manimSource = Join-Path $root "crates/media/visual/manim/manim-bridge/python"
$manim = Join-Path $stage "resources/manim-worker"
New-Item -ItemType Directory -Force $manim | Out-Null
Copy-Item (Join-Path $manimSource "shrimply_manim") $manim -Recurse
Copy-Item (Join-Path $manimSource ".python-version") $manim
$revision = (& git -C (Join-Path $root "external/manim") rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw "Could not resolve bundled Manim revision" }
$project = Get-Content (Join-Path $manimSource "pyproject.toml") -Raw
$project = $project -replace 'manimgl = \{ path = .*', "manimgl = { url = `"https://github.com/3b1b/manim/archive/$revision.tar.gz`" }"
[IO.File]::WriteAllText(
    (Join-Path $manim "pyproject.toml"),
    $project,
    [Text.UTF8Encoding]::new($false)
)
& $uv lock --python 3.14 --project $manim
if ($LASTEXITCODE -ne 0) { throw "Could not lock bundled Manim environment" }

Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $archive -CompressionLevel Optimal
$hash = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
$checksum = "$hash  $([IO.Path]::GetFileName($archive))`n"
[IO.File]::WriteAllText("$archive.sha256", $checksum, [Text.UTF8Encoding]::new($false))
Write-Host "Windows archive: $archive"
