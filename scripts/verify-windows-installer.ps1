# 在独立目录静默安装，再核对主程序、Hook 和版本；不启动 App，不发通知。
param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = 'Stop'
$installer = (Resolve-Path "release-assets/v$Version/windows-x64/AgentPulse_${Version}_x64-setup.exe").Path
$installDir = Join-Path $env:RUNNER_TEMP "AgentPulseSmoke-$([guid]::NewGuid().ToString('N'))"
try {
    $process = Start-Process -FilePath $installer -ArgumentList '/S', "/D=$installDir" -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode)" }
    $main = Join-Path $installDir 'AgentPulse.exe'
    $hook = Join-Path $installDir 'agentpulse-hook.exe'
    foreach ($binary in @($main, $hook)) {
        if (-not (Test-Path $binary)) { throw "Missing installed binary: $binary" }
        $bytes = [System.IO.File]::ReadAllBytes($binary)
        $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
        if ([BitConverter]::ToUInt16($bytes, $peOffset + 4) -ne 0x8664) { throw "Not an x64 binary: $binary" }
    }
    $installedVersion = (Get-Item $main).VersionInfo.ProductVersion
    if ($installedVersion -ne $Version -and $installedVersion -ne "$Version.0") {
        throw "Installed version mismatch: $installedVersion != $Version"
    }
    Write-Host "PASS: NSIS installed main App and Hook (x64, $installedVersion)"
    Get-AuthenticodeSignature $main | Select-Object Status, Path
} finally {
    $uninstaller = Join-Path $installDir 'uninstall.exe'
    if (Test-Path $uninstaller) {
        $process = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Uninstaller failed: $($process.ExitCode)" }
    }
}
