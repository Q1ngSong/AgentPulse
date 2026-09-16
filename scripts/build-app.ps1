param(
    [string]$Bundles = "nsis",
    [switch]$Debug,
    [switch]$VerboseBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot
. (Join-Path $PSScriptRoot "env.ps1")

if (-not (Test-Path "node_modules")) {
    pnpm install --frozen-lockfile
}

# Tauri 使用 cargo build --bins，自动包含主程序和 Hook；Windows externalBin 置空，避免 MSI 重复安装 Hook。
$args = @("tauri", "build", "--bundles", $Bundles, "--config", "src-tauri/tauri.windows.conf.json")
if ($Debug) { $args += "--debug" }
if ($VerboseBuild) { $args += "--verbose" }
& pnpm @args
if ($LASTEXITCODE -ne 0) { throw "Tauri 打包失败（退出码 $LASTEXITCODE）" }
Write-Host "产物：src-tauri\target\release\bundle\"
