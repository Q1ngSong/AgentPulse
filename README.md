# AgentPulse

<p align="center"><img src="assets/banner.svg" alt="AgentPulse：Claude Code / Codex 的本机提醒与延迟推送" width="100%"></p>

<div align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-blue.svg"></a>
</div>

AgentPulse 是 Claude Code / Codex 的 macOS 提醒工具。任务完成或请求权限时，通过弹窗、声音、飞书或微信通知你。远程推送可设置延迟。

当前面向 **macOS**，打包最低系统版本为 13.0；Windows 已提供构建入口，支持主面板、托盘、悬浮窗、系统通知和 Hook。Windows 的来源前台检测与系统声音播放仍未实现。下面的配图是功能与布局示意，实际材质随系统外观变化。

## 弹窗提醒

<p align="center"><img src="assets/notification-demo.svg" alt="当前悬浮窗布局：小图标与标题同行，正文通栏显示；分别展示权限请求和任务完成" width="100%"></p>

悬浮窗使用透明窗口和 macOS 原生毛玻璃材质。窗口在屏幕右上角置顶，不抢焦点。

图标与标题同行，正文最多三行。长标题自动省略。图标依次使用：上传的图片、来源 App 图标、AgentPulse 默认图标。

### 三种屏幕提醒模式

| 模式 | 行为 |
|---|---|
| **强制悬浮窗** | 由 AgentPulse 自己显示窗口，不经过系统通知中心，专注模式下也可显示 |
| **系统通知** | 使用 macOS 通知中心，受系统通知权限和专注模式控制 |
| **两者都发** | 同时显示悬浮窗并发送系统通知 |

点击动作可选「返回来源」或「只关闭」。来源可能是 Claude / Codex App，也可能是运行任务的终端或 IDE。

- **返回来源**：切回对应 App，并清除该来源当时已有的悬浮窗和已送达的系统通知。通过 Dock、Cmd+Tab 或点击窗口手动返回，也会触发这次清理。
- **保留其他提醒**：其他来源的提醒，以及返回后新到达的提醒都会保留。无法识别来源时，不会批量清除。
- **单条关闭**：右上角关闭按钮只移除当前提醒，后面的提醒自动补位。「只关闭」也不执行来源跳转。
- **停留时间**：屏幕悬浮窗在显示后约 5 秒自动清除，每条独立计时，后来出现的悬浮窗仍有各自的 5 秒。macOS 系统通知停留多久由「系统设置 → 通知 → AgentPulse」的提醒样式决定：「临时」几秒后收进通知中心，「持续」一直留在屏幕上。两者都可提前关闭或返回来源。

系统通知的正文和「返回来源」动作也可切回来源 App；「只关闭」不跳转。macOS 系统通知需要使用打包后的 `.app` 验证。

<p align="center"><img src="assets/effects.svg" alt="返回来源后的清理示意：清除该 App 已有悬浮窗，保留其他来源与随后到达的新提醒" width="100%"></p>

## 提醒方式

### 配置卡片

<p align="center"><img src="assets/channels-light.svg" alt="当前可添加的四类提醒方式：屏幕弹窗、提示音、飞书和微信测试号" width="100%"></p>

| 提醒方式 | 配置与用途 |
|---|---|
| **屏幕弹窗** | 上面的三种模式；每个工具最多一张配置卡片 |
| **提示音** | 请求权限、任务完成可分别选声音；支持上传音效和调整音量 |
| **飞书** | 群机器人 Webhook；开启签名校验时填写签名密钥 |
| **微信测试号** | 填写 appID、appsecret、openid 和模板 ID；模板写法见下方 |

微信只显示模板里「名称：{{字段.DATA}}」这样的行，不带名称的行会被丢掉。新增测试模板时，内容照下面填：

```
设备：{{device.DATA}}
应用：{{app.DATA}}
项目：{{project.DATA}}
标题：{{title.DATA}}
内容：{{body.DATA}}
时间：{{time.DATA}}
```

Claude Code 与 Codex 的提醒方式分别配置，也可以把一张卡片复制到另一个工具。每张卡片独立选择接收「请求权限」「任务完成」或两者。

飞书使用 Webhook 地址；目前没有独立的通用 HTTP Webhook 渠道。

## 延迟推送

<p align="center"><img src="assets/overview.svg" alt="提醒分发流程：按工具、事件和启用状态筛选；立即发送与延迟队列分开处理，延迟期间检测用户是否已返回" width="100%"></p>

每张提醒卡片单独设置发送时机。飞书和微信可立即发送，也可延迟 1、3、5、10、15、30 分钟。新建飞书默认立即发送，微信测试号默认延迟 5 分钟。

例如，保留本机弹窗和声音，把微信设为延迟 5 分钟：

1. 事件到来后，本机先提醒，微信进入等待队列。
2. 等待期间，如果检测到你切回来源 App 或在其中操作，就清空该 App 的待发提醒。同一 App 内发出新消息、批准对应权限或同一会话继续产生 Hook 事件，也会取消该 App 内对应工具的待发提醒。
3. 到期仍未检测到处理行为，才发送远程消息；可选择合并多条提醒或逐条发送。

延迟队列按来源 App 分开：Terminal、VS Code、Claude / Codex 桌面 App 各自排队。聚焦 VS Code 只清空 VS Code 的队列，Terminal 和其他 App 的提醒保留。同一 App 内仍使用 Claude Code / Codex 各自的提醒配置，消息不跨 App 合并。

悬浮窗也按来源 App 清理。无法识别来源 App 的提醒，不根据其他 App 的焦点或活动自动清空。

队列约每分钟检查一次，取消结果可能稍后反映到面板。停用或删除提醒方式后，队列不再向它发送；延迟发送失败最多尝试三次。**关闭一条悬浮窗本身不代表任务已处理，也不会直接取消远程队列。**

## 使用步骤

<p align="center"><img src="assets/config-flow.svg" alt="使用步骤：在设置中接入工具，分别配置提醒，再测试并新开 Claude Code / Codex 会话" width="100%"></p>

先从 [GitHub Release](https://github.com/Q1ngSong/AgentPulse/releases/latest) 下载 Apple Silicon 安装包，或从源码构建。图文指引见 [使用指南](https://q1ngsong.github.io/AgentPulse/guide/)。

安装后：

1. 将 `AgentPulse.app` 放到固定位置（例如 `/Applications`）并打开。
2. 在「设置 → 工具接入」中接入 Claude Code 或 Codex。接入会写入当前 App 内 Hook 程序的路径，因此移动 App 后需要重新接入。**Codex 还需信任 Hook**：接入时会展示 AgentPulse 的命令和事件，请确认「信任」。已有接入可点击「重新接入」完成信任；取消后保留配置，未信任的 Hook 暂不能发送提醒，也可在 Codex CLI 输入 `/hooks` 手动信任 AgentPulse 条目。
3. 返回首页，选择工具，编辑屏幕弹窗与提示音；需要远程提醒时，再添加飞书或微信测试号。
4. 点击卡片上的「测试」或编辑页的测试按钮。**测试会真实发送，并忽略延迟设置**。
5. **新开一个 Claude Code / Codex 会话**，让工具重新读取 Hook 配置，并用真实任务检查提醒。「已配置」及事件勾选仅表示 Hook 已写入，不代表已验证真实消息接收。

「设置 → 通用 → 模拟触发」也会真实发送。它使用当前工具的已保存配置，保留延迟规则。

发送结果、失败原因和延迟状态见「提醒记录」。

### 退出界面与卸载提醒

- 退出 App 后，已接入的工具仍会发送提醒。收到消息时只启动后台通知进程，不打开主界面或 Dock 图标；主动打开 AgentPulse 后才恢复主界面。
- 「设置 → 通用 → 卸载提醒」会备份并移除 Claude Code / Codex 中 AgentPulse 的 Hook，保留其他 Hook 和已保存的提醒配置。已有会话需重启后生效；已进入延迟队列的消息不受此操作影响。

macOS 上始终只显示菜单栏图标，打开或关闭主面板都不占用 Dock。关闭主面板后，App 仍可在后台接收提醒；通过菜单栏图标重新打开面板或退出。开机自启可在「设置 → 通用」中开启。

## 从源码构建

### 准备环境

需要 macOS、Xcode Command Line Tools、Node.js 22、pnpm 10，以及 PATH 中可用的 `rustup`。命令均在仓库根目录执行。

`scripts/env.sh` 会把 Rust 环境指向仓库内的 `.toolchain/`，它不会下载工具链。首次克隆需要初始化这个环境：

```bash
pnpm install --frozen-lockfile
source scripts/env.sh
rustup toolchain install stable --profile minimal
rustup default stable
```

Windows 使用 PowerShell（需要安装 Node.js、pnpm、rustup，以及 WebView2 Runtime）：

```powershell
. .\scripts\env.ps1
pnpm install --frozen-lockfile
rustup toolchain install stable --profile minimal
```

### 构建本机 App

```bash
scripts/build-app.sh --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
open src-tauri/target/release/bundle/macos/AgentPulse.app
```

脚本先编译并放置 `agentpulse-hook`，再打包 App。本机构建关闭自动更新产物生成，不需要更新签名密钥；本地 App 使用 ad-hoc 签名，未做 Apple 公证。

如需同时生成 DMG：

```bash
scripts/build-app.sh --bundles app,dmg --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

构建产物位于 `src-tauri/target/release/bundle/`。

Windows 构建 NSIS 安装包：

```powershell
. .\scripts\env.ps1
.\scripts\build-app.ps1
```

安装包位于 `src-tauri\target\release\bundle\nsis\`。也可以传 `-Bundles msi` 生成 MSI，或传 `-Debug` 构建调试包。

### 开发与检查

首次先完成上面的 App 构建，生成开发环境也需要的 Hook 文件；之后可运行：

```bash
source scripts/env.sh
pnpm dev
```

Vite 提供前端热更新，Rust 修改会触发重编译。开发模式可以检查悬浮窗；系统通知请使用打包后的 `.app`。

```bash
source scripts/env.sh
pnpm build:renderer
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

测试使用临时目录和替身，不发送真实通知。部分通信测试需要允许创建本地 Unix socket；受限沙箱可能阻止这些测试运行。

## 数据与实现

Tauri 2 + React / Vite / Tailwind，提醒核心使用 Rust，macOS 窗口与系统通知通过 AppKit 桥接。运行 App 不需要 Python 或 Swift 环境。

```text
src/                         React 面板与悬浮窗页面
src-tauri/src/
  core/                      配置、事件、渠道、延迟队列、活动判定
  bin/hook.rs                Claude / Codex 的 Hook 入口
  commands.rs                面板命令，测试与模拟发送在后台执行
  ipc.rs                     Hook → App 的本地 Unix socket
  overlay.rs / overlay/      悬浮窗、来源清理与 macOS 面板
  notifications/             macOS 系统通知与点击响应
scripts/env.sh               仓库内 Rust 环境
scripts/build-app.sh         Hook 与 App 构建入口
```

- 数据默认保存在 `~/.agentpulse/`；配置密钥位于 `config.json`（权限 0600），前端读取脱敏值。`AGENTPULSE_HOME` 可覆盖数据目录。
- 修改 Claude / Codex 的 Hook 配置前先备份，只改 AgentPulse 条目；JSON 解析失败时中止。
- Hook 事件处理与队列 worker 保持 stdout 静默，程序始终 `exit 0`；仅安装、卸载子命令打印接入结果。批准权限仍在来源工具中完成。
- App 日志位于 `logs/app.log`，Hook 异常位于 `hook-errors.log`，提醒记录位于 `events.jsonl`。
- `.toolchain/`、`.venv/`、`node_modules/`、构建产物及 `.codegraph/` 等本地索引不进入 Git。

## 网站与发布

[网站首页](https://q1ngsong.github.io/AgentPulse/) · [使用指南](https://q1ngsong.github.io/AgentPulse/guide/) · [下载](https://github.com/Q1ngSong/AgentPulse/releases/latest)

网站源码在 `site/`，只有首页和使用指南。`pnpm dev:site` 本地预览，`pnpm build:site` 输出到 `dist-site/`；GitHub Pages 工作流部署这个目录，网站资源不进入 App。

当前 Release 提供 macOS 13+ / Apple Silicon 安装包。App 使用 ad-hoc 签名，尚未经过 Apple 公证；更新包使用已有的 Tauri 密钥签名。图标保留原始质量。

### 一次构建两个平台

打开 GitHub [Actions → Release](https://github.com/Q1ngSong/AgentPulse/actions/workflows/release.yml)，点击 **Run workflow**：

1. 选择包含待发布代码的分支（通常为 `main`），先确保四处版本号一致：`package.json`、`src-tauri/tauri.conf.json`、`Cargo.toml` 和 `Cargo.lock`。
2. `tag` 留空时按应用版本生成 `v版本号`；也可以填写包含此工作流的已有 tag。已发布的版本不能覆盖。
3. 默认生成 macOS ARM64 的 DMG 和 Windows x64 的 EXE 安装包；勾选 `include_msi` 可同时生成 MSI。

两个系统在 GitHub 的独立机器上并行构建，同一提交的安装包通过测试、DMG 挂载验证、Windows 静默安装验证后，汇总到**同一个 Release 草稿**。上传后再次下载核对 SHA-256，不自动公开发布。无需配置签名 Secrets；macOS 使用 ad-hoc 签名，Windows 安装包未签名。

安装包同时保存在该次运行的 `AgentPulse-v版本号-all-installers` Artifact（保留 30 天），可以整包下载。草稿中有安装包、`BUILD-INFO.json` 和 `SHA256SUMS.txt`。公开发布前需在真实设备上验证通知与 Hook 接入。

产物统一按版本和平台归档，`release-assets/` 不进入 Git：

```text
release-assets/v0.1.1/
  macos-arm64/   # DMG 与该平台构建信息
  windows-x64/   # EXE、可选 MSI 与该平台构建信息
  all/           # 汇总安装包、构建信息和 SHA256SUMS.txt
```

### 本机签名更新包

Actions 当前只生成手动安装包；自动更新仍需使用本机私钥生成更新包与 `latest.json`，验证后上传到对应 Release。私钥保留在仓库外：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$HOME/.tauri/agentpulse.key"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=''
scripts/build-app.sh --bundles app
scripts/package-release.sh
```

`release-assets/v版本号/macos-arm64/` 包含 DMG、更新包及签名、`latest.json` 和 `SHA256SUMS.txt`。打包脚本会自动核对哈希和签名；也可运行 `node scripts/verify-release.mjs` 单独验证。仅需 DMG 时，先用 `scripts/build-app.sh --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'` 构建，再运行 `scripts/package-release.sh --installer-only`。

贡献与代码约定见 [PROJECT.md](PROJECT.md)。

## 许可证

[MIT](LICENSE)。

## 桌宠

在「桌宠」页添加独立编号的桌宠，勾选需要同步的 Agent 和来源 App。一只桌宠可以汇聚多个来源；按住宠物拖动，单击返回消息来源，右键可以关闭，关闭会同步停用列表开关。空闲时不显示状态提示。

默认猫咪的图片**不进入 Git 或安装包**。首次启用或预览时从 GitHub 下载约 16.6 MiB 的无损动画图集，校验 SHA-256 后缓存到数据目录的 `pets/v1/`（默认 `~/.agentpulse/pets/v1/`）。之后可离线使用；失败可以重试。自定义动作仍可使用本地序号 PNG 文件夹。

下载清单和哈希保存在 `src-tauri/pet-assets.json`；素材作为独立文件附加到已有 `v0.1.0` Release，资源地址不依赖当前 App 版本。`assets/pet/` 是忽略的本地制作目录，200 张逐帧 PNG 只用于制作和检查。开发者可从历史 `a349737` 取出原画，安装 `scripts/pet-animation-requirements.txt`，运行 `scripts/build-pet-animation.py` 重建素材。常规 App 构建不需要任何桌宠图片或图像处理依赖。
