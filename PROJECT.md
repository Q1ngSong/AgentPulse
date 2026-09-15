# AgentPulse

给 Agent 的项目说明；安装和使用见 [README.md](README.md)。

## 目标与改动边界

为 Claude Code / Codex 提供本机提醒和延迟远程推送。保持项目小而清楚：满足当前用途，优先删除无用代码、复用已有实现，不为未来需求预设接口、配置或分支。

配置只保留当前使用的结构，不维护版本字段和历史格式迁移。

精简保留现有功能与外部协议；删除前核对调用方、Tauri 命令注册、Hook / IPC 入口、回调及测试。零调用者索引结果只是线索。功能取舍另行由用户确定。

## 代码分工

```text
src-tauri/src/core/       配置、事件、渠道、延迟队列、活动判定、Hook 接入与资源
src-tauri/src/bin/hook.rs Claude / Codex 调用入口，也负责 --queue-worker
src-tauri/src/commands.rs 前端的 Tauri 命令
src-tauri/src/ipc.rs      Hook → App 的 Unix socket
src-tauri/src/overlay*    悬浮窗与 macOS 面板
src-tauri/src/notifications/ macOS 系统通知与来源跳转
src-tauri/src/lib.rs      App、托盘、主窗口
src-tauri/tests/queue.rs  注入假运行环境的队列集成测试
src/                     React 面板与独立 overlay 页面；lib/api.ts 封装 invoke
scripts/                 工具链环境与打包入口
```

依赖方向：前端 → commands → core；bin/hook → core。Hook 通过 socket 请求 App 显示通知。核心不依赖前端或 App 的窗口实现；平台代码留在现有平台模块。

## 必须保留的行为

- Hook 永远 exit 0；事件处理与队列 worker 保持 stdout 静默，仅 `--install` / `--uninstall` 子命令打印接入结果。
- 改 `~/.claude/settings.json`、`~/.codex/hooks.json` 前备份，只改 AgentPulse 条目；JSON 解析失败时中止。
- 配置密钥保存在数据目录的 `config.json`（默认 `~/.agentpulse/`，可由 `AGENTPULSE_HOME` 改写），微信访问令牌另存缓存；敏感文件使用 0600。前端读取脱敏值，保存时还原未改动的密钥。
- 保留文件锁、原子写、外部输入校验、队列合并与重试。测试使用临时目录和替身，不读写真实用户配置、不发真实通知。
- 延迟队列按来源 App 隔离，聚焦只清理该 App 的提醒；合并、活动判定和注意力状态不跨 App。同一 App 内保留各工具的提醒配置。

## 验证与产物

```bash
source scripts/env.sh
pnpm build:renderer
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

UI 行为变化还需实际窗口验证；macOS 系统通知用打包后的 `.app` 验证。开发入口是 `pnpm dev`，打包入口是 `scripts/build-app.sh`。

`.toolchain/`、`.venv/`、`node_modules/`、`dist/`、`src-tauri/target/`、`src-tauri/binaries/` 和 `reference-projs/` 是本地工具、产物或参考源码，不入库。

## DCLH

`.comment-standard.json` 启用 DCLH，采用默认索引范围与现有 `.gitignore`。插件装在项目外；设 `DCLH_ROOT` 为插件根目录后刷新：

```bash
python3 "$DCLH_ROOT/scripts/refresh.py" . --base HEAD
```

随后用 `codegraph explore <符号>` / `codegraph callers <符号>` 辅助查调用，修改代码后重新刷新。索引和机械报告在 `.codegraph/`，不入库。

**适用范围：** DCLH 基于 AST 的注释检查、Python 函数变更检查及 `empty_layer_dirs` 诊断面向 Python；图规则可读取多语言索引，但可能漏掉 Tauri invoke、宏注册、React 回调等调用。R1 零调用者与退出码 0 都不代表本项目通过完整审计，必须结合源码、入口、编译和测试判断。`layers` 仅接受顶层目录，无法表达 `src-tauri/src/core` 内外的真实边界，暂不声明；依赖方向以上文为准。保持现有目录，不添加实验框架或复制插件脚本。
