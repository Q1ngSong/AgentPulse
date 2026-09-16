# Windows 构建环境：在仓库根目录执行 `. .\scripts\env.ps1` 后使用 cargo / pnpm。
$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$env:AGENTPULSE_ROOT = $repoRoot

# 与 scripts/env.sh 对齐：有仓库内工具链时优先使用它，否则沿用 rustup 的默认安装。
$localRustup = Join-Path $repoRoot ".toolchain\rustup"
$localCargo = Join-Path $repoRoot ".toolchain\cargo"
if (Test-Path $localCargo) {
    $env:CARGO_HOME = $localCargo
    $env:RUSTUP_HOME = $localRustup
    $env:Path = (Join-Path $localCargo "bin") + ";" + $env:Path
}

foreach ($command in @("node", "pnpm", "rustc", "cargo")) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        throw "找不到 $command，请先安装 Node.js、pnpm 和 Rust（rustup）。"
    }
}

Write-Host "AgentPulse Windows 环境已加载：$repoRoot"
