$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$archive = Join-Path $root "dist/shrimply-windows-x86_64.zip"
$checksum = "$archive.sha256"
$stage = Join-Path $root "dist/shrimply-windows-x86_64"
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root "target" }
$release = Join-Path $target "release"
$dumpbin = (Get-Command dumpbin.exe -ErrorAction Stop).Source
$env:QT_QPA_PLATFORM = "offscreen"

function Assert-PeX64([string]$path) {
    $headers = & $dumpbin /headers $path 2>&1
    if ($LASTEXITCODE -ne 0 -or !($headers -match "8664 machine \(x64\)")) {
        throw "$path is not an x64 PE binary"
    }
}

function Inspect-Imports([string]$directory) {
    $binaries = Get-ChildItem $directory -Recurse -File | Where-Object {
        $_.Extension -in @(".exe", ".dll")
    }
    if (!$binaries) { throw "No PE binaries found in $directory" }
    $packaged = @{}
    foreach ($binary in $binaries) { $packaged[$binary.Name.ToLowerInvariant()] = $true }
    $external = @{ "nvcuda.dll" = $true }
    foreach ($binary in $binaries) {
        $imports = & $dumpbin /dependents $binary.FullName
        if ($LASTEXITCODE -ne 0) { throw "Could not inspect imports for $($binary.FullName)" }
        foreach ($line in $imports) {
            $dependency = $line.Trim()
            if ($dependency -notmatch '^[A-Za-z0-9_.-]+\.dll$') { continue }
            $name = $dependency.ToLowerInvariant()
            if ($packaged.ContainsKey($name) -or $external.ContainsKey($name)) { continue }
            if ($name -like "api-ms-*.dll" -or $name -like "ext-ms-*.dll") { continue }
            if (Test-Path (Join-Path "$env:SystemRoot\System32" $dependency)) { continue }
            throw "$($binary.FullName) imports missing library $dependency"
        }
    }
}

function Use-PackagedRuntime {
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
    foreach ($name in @("QT_PLUGIN_PATH", "QML2_IMPORT_PATH", "QML_IMPORT_PATH")) {
        Remove-Item "Env:$name" -ErrorAction SilentlyContinue
    }
}

function Assert-ArgumentFailures([string]$directory) {
    $launcher = Join-Path $directory "shrimply-qt.exe"
    & $launcher (Join-Path $env:RUNNER_TEMP "missing.shrimp")
    if ($LASTEXITCODE -eq 0) { throw "Launcher accepted a missing project" }
    & $launcher first.shrimp second.shrimp
    if ($LASTEXITCODE -eq 0) { throw "Launcher accepted excess project arguments" }
}

function Assert-QmlStartup([string]$directory) {
    $stdout = Join-Path $env:RUNNER_TEMP "shrimply-windows-startup.stdout"
    $stderr = Join-Path $env:RUNNER_TEMP "shrimply-windows-startup.stderr"
    $process = Start-Process (Join-Path $directory "shrimply-qt.exe") `
        -WorkingDirectory $directory -PassThru `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    Start-Sleep -Seconds 8
    if ($process.HasExited) {
        $output = (Get-Content $stdout, $stderr -ErrorAction SilentlyContinue) -join "`n"
        throw "Launcher exited before its QML root remained loaded (code $($process.ExitCode))`n$output"
    }
    if (!$process.CloseMainWindow()) { Stop-Process -Id $process.Id }
    elseif (!$process.WaitForExit(2000)) { Stop-Process -Id $process.Id }
}

foreach ($name in @("shrimply-qt.exe", "shrimply-editor-qt.exe")) {
    Assert-PeX64 (Join-Path $release $name)
}
if (!(Test-Path $archive) -or !(Test-Path $checksum)) { throw "Windows archive is missing" }
$expected = ((Get-Content $checksum -Raw) -split '\s+')[0]
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "Windows archive checksum mismatch" }

Inspect-Imports $stage
Use-PackagedRuntime
Assert-ArgumentFailures $stage
Assert-QmlStartup $stage

$unpacked = Join-Path $env:RUNNER_TEMP "shrimply-windows-unpacked"
if (Test-Path $unpacked) { Remove-Item -Recurse -Force $unpacked }
Expand-Archive $archive $unpacked
Inspect-Imports $unpacked
Assert-ArgumentFailures $unpacked
Assert-QmlStartup $unpacked
