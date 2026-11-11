$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = "$machinePath;$userPath"
    $existing = Get-Command shrimply -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($env:SHRIMPLY_CLI_ACTION -eq 'status') {
        if ($existing) { Write-Output $existing.Source }
        exit 0
    }
    if ($env:SHRIMPLY_CLI_ACTION -ne 'install') { throw 'Unknown CLI setup action' }
    $launcher = $env:SHRIMPLY_CLI_LAUNCHER
    if (!(Test-Path -LiteralPath $launcher -PathType Leaf)) {
        throw "Shrimply launcher not found: $launcher"
    }
    $directory = [IO.Path]::GetDirectoryName($launcher)
    $env:Path = $machinePath
    $systemCommand = Get-Command shrimply -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($systemCommand -and $systemCommand.Source -ine $launcher) {
        throw "The system PATH already provides $($systemCommand.Source). Remove that command from the system PATH before installing this copy for your user."
    }
    $entries = @($userPath -split ';' | Where-Object {
        $_ -and $_.Trim('"').TrimEnd('\') -ine $directory.TrimEnd('\')
    })
    $updatedPath = (@($directory) + $entries) -join ';'
    # The User target persists the value and broadcasts WM_SETTINGCHANGE.
    [Environment]::SetEnvironmentVariable('Path', $updatedPath, 'User')
    Write-Output "Installed $launcher on your user PATH. Open a new terminal to use shrimply."
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
