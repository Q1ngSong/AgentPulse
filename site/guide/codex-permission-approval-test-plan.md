# Codex 权限提醒回归方案

## 验收目标

- 自动审批、内部审核以及已经执行中的工具：不发权限提醒，运行多久都一样。
- Codex 真正在等待用户批准的请求：确认待审批状态后立即提醒一次。
- 批准、拒绝、取消导致请求消失：结束桌宠的权限等待，取消该请求尚未发送的远程提醒。
- 不把 `permission_mode: default`、缺少 PostToolUse 或时间经过当成人工审批证据。

[2026-09-16 的旧版实测](codex-permission-test-results-2026-09-16.md) 已确认 90 秒观察窗口会误报，保留为修复前证据，不再使用其时间阈值作为验收标准。

## 当前事件链

1. `PermissionRequest` Hook 只保存上下文并唤醒会话观察器，所有 permission_mode 使用同一路径。
2. 观察器通过本机 Codex 桌面 IPC 发现该会话拥有者，订阅会话快照及增量状态。
3. 仅处理 `requests` 中的以下待处理请求，并要求 session/thread 和 turn 均匹配：
   - `item/commandExecution/requestApproval`
   - `item/fileChange/requestApproval`
   - `item/permissions/requestApproval`
4. 请求身份按会话、轮次、RPC request ID 去重。`item/autoApprovalReview/*` 和工具过程不作为人工审批。
5. 请求从权威状态移除后，清理对应桌宠和延迟队列，不依赖工具执行完成。
6. 接口缺失、断连、状态版本不支持、增量丢失时不推测提醒，写入 `hook-errors.log`。没有按时间触发权限提醒的兜底。

代码：[状态订阅](../../src-tauri/src/core/codex_approval.rs) · [Hook 观察与分发](../../src-tauri/src/core/codex_permission.rs)

### 支持边界

本轮适配的是 macOS Codex 桌面程序随附 `codex-cli 0.154.0-alpha.6.2` 的本地会话 IPC v11。它不是稳定公开的第三方通知接口，版本更新后必须复测。当前 Windows 命名管道、纯 CLI / IDE 中不由此桌面进程拥有的会话不支持此人工审批观察；这些情况下抑制权限提醒，任务完成提醒仍按原 Hook 工作。

观察器只发初始化、拥有者查询和订阅/取消订阅消息，不批准或拒绝任何请求，不恢复或修改任务，不修改 Hook 信任。会话快照可能包含正文，但观察器只保留待审批数组，不写入会话正文。

## 自动化验证

```sh
source scripts/env.sh
pnpm build:renderer
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

测试使用临时目录、模拟 IPC 和假渠道，不读取真实用户配置，不发送真实通知。

| 用例 | 预期 |
| --- | --- |
| 内部审核 started/completed，或工具运行超过 110 秒而没有 PostToolUse | 零权限提醒 |
| 三种真实 requestApproval 方法 | 可被识别；非审批问答事件不触发 |
| 真实请求加入 requests | 立即分发一次，无 90 秒等待 |
| 重复快照、重放请求、重建观察器内存 | 不重复分发 |
| 错误 session / turn / owner 的请求或状态 | 不借用其他请求上下文 |
| 请求移除、批准或拒绝 | 对应桌宠状态和远程队列清理；同会话其他请求保留 |
| 部分帧到达、读取超时 | 不丢帧；后续可继续解析 |
| 协议版本变化、revision 跳跃、断连 | 停止观察，不生成超时提醒 |
| PreToolUse 到达 | 不能据此判断批准完成 |
| 工具完成、任务结束、24 小时资源过期 | 只能清理观察器资源，不能触发提醒 |
| Hook 退出 | exit 0，stdout 为空 |

## 本机真实回归

### 准备

1. 编译并安装同一份 App，记录安装版 Hook 的 SHA-256，避免其他并行任务更新安装包。
2. 记录当前 Codex 版本、来源 App、session_id、turn_id，以及当前审批模式；不要把模式名直接当结果。
3. 在提醒卡片先测试渠道是否可见，然后分别验证真实审批链路。卡片测试不等价于真实 Hook。
4. 查看数据目录中的 `activity.jsonl`、`events.jsonl`、`hook-errors.log`、`codex-permission-pending.json`。

证据说明：`CodexApprovalObserverConnected` 表示观察器已接入会话；权限分发记录的 `payload.hook_event_name` 必须为 `CodexApprovalRequested`，并含真实 request ID 和方法。`results.ok: true` 只是渠道接受，不是用户看到了横幅的证据。

### A：内部批准的短请求

在“帮我批准”模式下触发安全的只读沙箱外请求。确认捕获到了 PermissionRequest、命令实际执行，且用户没有点击批准。期望零 AgentPulse 权限提醒。

### B：内部批准的长请求

运行安全只读检查并等待 110 秒，打印开始和结束时间。用户不手动批准。全程及完成后 10 秒内都应没有权限提醒；90 秒附近出现提示即失败，不再解释为窗口太短。

### C：确实需要本人批准

切换到人工审批模式，触发安全只读请求。记录 Codex 批准按钮出现时间和 AgentPulse 提醒时间，先等待约 10 秒不点。期望收到真实 requestApproval 后立即提醒，而不是等 90 秒。分别记录系统通知/悬浮窗的实际可见性；两种窗口不能混淆。

### D：批准与拒绝后的清理

分别做一次批准、一次拒绝。期望请求从 requests 移除后桌宠结束权限等待、该请求的远程队列取消。同一 App 的其他待审批请求不应被清空。屏幕通知仍遵守既有约 5 秒自动收起规则。

### E：并行与不支持的环境

两个会话、同一轮多个请求分别测试隔离和去重。关闭 Codex 或使用无桌面拥有者的纯 CLI 会话时，记录观察不可用的日志，并确认没有延迟补弹。不要通过重新启动任务或改变审批策略伪造成功。

## 记录模板

| 用例 | 安装 Hook SHA-256 | 会话/轮次 | 是否本人批准 | Codex 按钮时间 | 真实 request 时间 | AgentPulse 分发时间 | 用户实际看到的窗口 | 结果 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| A | | | | | | | | |
| B | | | | | | | | |
| C | | | | | | | | |
| D | | | | | | | | |

记录不复制用户配置、认证令牌或整段会话。真实 UI 无法观察时明确标为待验收，不用合成请求或成功回执代替视觉证据。
