$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$dist = Join-Path $root "dist"
$stage = Join-Path $dist "shrimply-windows-x86_64"
$archive = Join-Path $dist "shrimply-windows-x86_64.zip"
$release = Join-Path $root "target/release"
$vcpkg = if ($env:VCPKG_INSTALLED_DIR) {
    Join-Path $env:VCPKG_INSTALLED_DIR "x64-windows/bin"
} else {
    Join-Path $root "vcpkg_installed/x64-windows/bin"
}

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
if (Test-Path $archive) { Remove-Item -Force $archive }
New-Item -ItemType Directory -Force $stage | Out-Null

foreach ($name in @("shrimply-qt.exe", "shrimply-editor-qt.exe")) {
    $source = Join-Path $release $name
    if (!(Test-Path $source)) { throw "Missing release executable: $source" }
    Copy-Item $source $stage
    & windeployqt.exe --release --qmldir $root (Join-Path $stage $name)
    if ($LASTEXITCODE -ne 0) { throw "windeployqt failed for $name" }
}

if (!(Test-Path $vcpkg)) { throw "Missing vcpkg runtime directory: $vcpkg" }
Copy-Item (Join-Path $vcpkg "*.dll") $stage

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
