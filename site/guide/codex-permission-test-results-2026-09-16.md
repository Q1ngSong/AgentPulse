# Codex 权限提醒实测记录 · 2026-09-16

**最新完整复测（16:19 之后）**：170 项 Rust 测试全部通过，前端构建和本地 App 签名检查通过。覆盖安装及发行版 110 秒隔离复测被自动审批容量错误阻止，本机仍为旧版；真实人工审批弹窗尚未验收。详见文末“再次完整测试”。

修复前一轮验收未通过。已确认：Codex 自动批准后的长任务会被 AgentPulse 误报为“需要授权”。人工审批用例确实等待用户操作，AgentPulse 在 90 秒后投递了通知，但用户最后明确反馈没有看到 AgentPulse 通知，显示环节不能判为通过。

关联：[测试方案](codex-permission-approval-test-plan.md) · [脱敏时间线](codex-permission-test-evidence-2026-09-16.json)

## 测试环境与证据范围

- 时间：2026-09-16，Asia/Shanghai（UTC+8）。
- 使用当前真实 Codex 对话调用 `exec_command`，并非手工伪造 Hook 来充当真实审批。
- A、B：`approval_policy = on-request`，`approvals_reviewer = auto_review`。用户确认两例均未手动批准。
- C：用户临时切换为人工审批；工具请求保持等待，用户确认 Codex 批准按钮可见，最终请求被用户拒绝，测试命令未执行。
- AgentPulse 的 Codex 桌面渠道为 `system`，同时启用提示音；本轮未验证独立悬浮窗或桌宠。
- 配置中有 PermissionRequest、PreToolUse、PostToolUse、Stop、UserPromptSubmit 五条 AgentPulse Hook，均存在信任记录。本轮真实 A、B 记录到了 PostToolUse，没有记录到 PreToolUse。
- 日志按当前 `session_id` 与工具输入指纹关联，排除同机其他任务的提醒。原始本地证据位于 `/tmp/agentpulse-permission-20260916/`；提交用附件只保留本轮用例、时间、指纹和渠道结果，不复制用户配置或凭据。
- 界面证据来自用户观察：计算机工具禁止读取 Codex 界面，读取 AgentPulse 也超时。没有截图证据，不能把渠道成功回执当作屏幕显示证明。

安装版 Hook 位于 `/Applications/AgentPulse.app/Contents/MacOS/agentpulse-hook`。测试期间该文件被其他操作更新，分别记录实际取样的 SHA-256：

| 阶段 | Hook SHA-256 |
| --- | --- |
| A、B 之前取样；15:18:26 隔离用例启动时再次匹配 | `ae9d5e878d295aa364eedf28ad0f6115b9715a264af93a2b86cb28f63a72abfe` |
| C 请求前取样 | `5b70fd2b02f6a5e4d30da42985407c3b0a38f24bd08b6ce03447c19863cb5479` |

这是安装环境的实测记录，不宣称三个用例均来自同一构建。两个阶段都实测到了约 90 秒的观察窗口。

## 真实请求结果

| 用例 | 是否需要用户手动批准 | Codex 审批状态 | AgentPulse 投递 | 实际通知观察 | 判定 |
| --- | --- | --- | --- | --- | --- |
| A：自动批准、短进程检查 | 否 | 自动放行并执行完成 | 观察超过 100 秒，无对应权限提醒事件 | 没有该事件的投递记录；未单独要求用户确认 A 的可见性 | 抑制逻辑通过 |
| B：自动批准、运行 110 秒 | 否 | 先自动放行，再持续执行 | 请求后 90.009 秒投递一次权限提醒，桌面及声音渠道均成功 | 用户明确确认看到了 AP-PERM-B 通知，且没有手动批准 | **失败：误报** |
| C：人工审批、保持等待 | 是 | 批准按钮可见；最终用户拒绝，命令未执行 | 请求后 90.006 秒投递一次权限提醒，桌面及声音渠道均成功 | 用户最后明确选择“Codex 批准按钮可见，但没有 AgentPulse 通知” | 审批等待已确认；通知显示未通过确认 |

C 的观察过程中，用户先说“没有任何弹窗”，又说“刚刚弹出了窗口”，之后在明确区分两种窗口的选项中选择只看到了 Codex 批准按钮。前一句未指明窗口来源，不能据此认定 AgentPulse 系统通知显示成功。本记录以最后的明确反馈为准。

### A：自动批准短请求

测试动作：只读检查进程列表，标记 AP-PERM-A。

| 时间 | 证据 |
| --- | --- |
| 15:16:01.515710 | 捕获 PermissionRequest 暂存项；`permission_mode = default` |
| 15:16:08.401340 | 同输入指纹的 PostToolUse；距请求 6.886 秒 |
| 超过 100 秒后复查 | 没有 AP-PERM-A 的 `permission_request` 分发事件 |

此例确实进入了审批链路，不是拿无需审批的普通命令替代自动审批测试。

### B：自动批准长请求

动作：只读检查 PID 1，打印开始时间，等待 110 秒，再打印结束时间。无文件修改。

| 时间 | 证据 |
| --- | --- |
| 15:16:55.031925 | PermissionRequest；`permission_mode = default` |
| 15:17:00.671740 | 工具输出 `AP-PERM-B EXECUTION_STARTED`，说明已放行并开始运行 |
| 15:18:25.041284 | AgentPulse 生成 `permission_request`，ID `d969be69fa89`，标题正文包含 AP-PERM-B；桌面渠道 `ok: true` |
| 15:18:50.676795 | 工具输出 `AP-PERM-B EXECUTION_FINISHED` |
| 15:18:50.693120 | 匹配的 PostToolUse 到达，距请求 115.661 秒 |

提醒出现时工具已经运行约 84.370 秒，并在约 25.636 秒后正常完成。用户确认没有手动批准，但看到了通知。因此该提醒不代表正在等待用户授权。

### C：人工审批请求

动作：请求在沙箱外只读查询 PID 1 的进程名；要求审批保持等待至少约 100 秒。

| 时间/阶段 | 证据 |
| --- | --- |
| 15:20:06.157292 | PermissionRequest 暂存项；仍为 `permission_mode = default` |
| 等待期间 | 工具调用未返回，没有对应执行输出或 PostToolUse |
| 15:21:36.163464 | AgentPulse 生成 `permission_request`，ID `c6a7679126e6`；桌面和提示音均 `ok: true` |
| 用户操作后 | 工具返回 `Rejected("rejected by user")`，命令没有执行 |

人工审批与自动审批都发送了 `permission_mode = default`，实测不能用该值区分是否需要本人处理。

## 安装版 Hook 隔离检查

另用安装版 Hook 在临时 `AGENTPULSE_HOME` 下直接注入事件；关闭全部提醒渠道，不修改真实配置，不发通知。此组只验证 Hook/worker，不能替代上面的真实审批或屏幕观察。

| 注入顺序 | 观察 100 秒后的结果 | 说明 |
| --- | --- | --- |
| PermissionRequest，随后无活动 | 90.007 秒生成一次权限事件 | 安装版确实使用约 90 秒的窗口 |
| PermissionRequest → 匹配的 PostToolUse | 零权限事件 | 完成记录可以抑制提醒 |
| PreToolUse → PermissionRequest，随后无活动 | 90.007 秒生成一次权限事件 | 发生在请求之前的 PreToolUse 不会消解该请求 |

三个用例均无 Hook 错误，Hook 的 stdout 保持为空。合成 PreToolUse 能被记录，只能证明 AgentPulse 会处理它，不能证明真实 Codex 已经发出该事件。

## 已确认原因与未确认项

1. **观察窗口计时已经生效，误报来自判定依据。** [`codex_permission.rs`](../../src-tauri/src/core/codex_permission.rs) 在到期后检查活动；B 的完成 Hook 在 115.661 秒才到达，90 秒时没有匹配活动，于是仍按权限请求分发。继续加长时间只能移动误报阈值。
2. **不能把 PreToolUse 当作批准完成。** 它是可阻止或改写调用的拦截点；本轮真实路径还没有观察到它。需要分别验证 Hook 覆盖与审批状态，不能用合成事件补足真实证据。[OpenAI 官方 Hooks 文档](https://learn.chatgpt.com/docs/hooks)
3. **人工审批通知显示仍待定位。** [`macos.m`](../../src-tauri/src/notifications/macos.m) 在系统接受通知请求后返回成功，并安排 5 秒后清理；该回执不提供“用户看到了横幅”的证据。当前无法确定 C 未见通知是显示、清理、系统设置或观察时机导致，不能任选一个当根因。
4. 未覆盖用户快速批准、焦点切换后的清理、独立悬浮窗、桌宠以及新建 Codex 任务的 PreToolUse 覆盖。C 验证的是等待后拒绝，不是批准后执行。

以上 A/B/C 是修复前记录，当时尚未修改产品逻辑。后续修改与验证另列如下，不覆盖原始失败证据。


## 后续修复：改用真实待审批状态

用户确认立即修改后，移除了 90 秒观察分发逻辑以及 permission_mode 分支。PermissionRequest 仅唤醒只读观察器；分发依据改为 Codex 桌面会话中真实待处理的 requestApproval，请求移除后清理对应桌宠与远程队列。

本机曾通过只读 IPC 查询成功发现当前会话拥有者，并取得 v11 状态快照；该快照待处理请求数为 0。接口探测没有批准、拒绝或重启任务。协议取自本机随附 codex-cli 0.154.0-alpha.6.2 的桌面实现。新 Rust 观察器对真实人工审批的屏幕表现仍须另行验收。

### 已执行的修复后隔离回归

使用新编译的 debug Hook、临时 HOME / CODEX_HOME / AGENTPULSE_HOME、模拟 Codex IPC、空通知渠道，运行 `/tmp/agentpulse-approval-regression.py`。这是合成协议测试，不冒充真人审批和真实屏幕通知。

| 验证 | 结果 |
| --- | --- |
| 内部审核状态；真实等待 110.003 秒，无 PostToolUse | 零权限分发事件 |
| 同会话但错误 turn 的待审批请求 | 零事件 |
| 正确 turn 的真实 requestApproval 协议消息 | 约 0.028 秒内分发一次 |
| 重复快照 | 总事件数仍为 1 |
| requests 移除请求 | 暂存清空，观察器取消订阅退出 |
| Hook 输出与错误 | stdout 0 字节；无 hook-errors |
| 接口不可用：default、on-request、缺省 mode | 三种均 exit 0、stdout 为空、零权限事件，并记录观察不可用 |

本地原始结果位于 `/tmp/ap-approval-8ko4zb4b/result.json`。没有真实通知渠道，因而这组数据不能证明用户看到了系统横幅。

### 构建与自动化状态

- `pnpm build:renderer`：通过。
- 队列集成测试：44 项通过，包括审批清理的请求隔离。
- Rust 全量测试首次在沙箱内运行：110 项通过；15 个本地 socket 测试被沙箱的 `Operation not permitted` 阻止，集成测试未随之继续，故另行运行上面的队列测试。
- 两次申请沙箱外完整测试均被自动审批拒绝，原因是审批模型容量不足；不是这 15 项已经通过，也不是发现了 15 个产品缺陷。
- 真机的自动审批长任务回归、人工审批通知可见性，以及安装后的 Rust 观察器接入，尚待补验。

支持边界及复测步骤见新版[测试方案](codex-permission-approval-test-plan.md)。


### 本地 App 产物

已生成 `src-tauri/target/release/bundle/macos/AgentPulse.app`，主程序和 Hook 均已使用最终源码重新编译；重新进行本地 ad-hoc 签名，`codesign --verify --deep --strict` 通过。

- Hook SHA-256：`10ccb5665d3b83be9268b16b14cbc634032f928d653355c2ada2f522dc56811e`。
- 未覆盖安装到 `/Applications/AgentPulse.app`；当前安装版不应视为已应用修复。
- 默认打包命令在自动更新包签名环节因缺少 `TAURI_SIGNING_PRIVATE_KEY` 返回错误；本地 `.app` 已完成构建和签名校验，不提供本次自动更新包或 DMG。
- 本机安装、完整 Rust socket 测试及真实屏幕通知验收仍等待审批环境恢复。本轮没有提交 Git。


## 再次完整测试

用户要求重新完整测试后，重新执行了当前代码的全部 Rust 测试。本次完整测试获准在沙箱外运行，包含此前被沙箱阻止的本地 socket 用例，没有跳过这些用例。

| 项目 | 本轮结果 |
| --- | --- |
| Rust 单元测试 | 126 通过，0 失败，0 忽略 |
| 队列集成测试 | 44 通过，0 失败，0 忽略 |
| 合计 | **170 通过** |
| `pnpm build:renderer` | 通过 |
| `codesign --verify --deep --strict` | 通过 |
| `git diff --check` | 通过 |

覆盖了只读观察器握手与订阅、owner/thread 过滤、真实请求加入与移除、内部审核排除、断连与不兼容协议、半帧读取、去重、桌宠状态和队列隔离。编译仍有既有 macOS 图标接口弃用警告，不影响本轮测试结果。

### 仍未执行的真机部分

1. 已准备备份和覆盖安装脚本 `/tmp/agentpulse-install-for-retest.py`，但执行前被自动审批以“Selected model is at capacity”拒绝，未修改 `/Applications`。
2. 使用最终签名 Hook 的 110 秒隔离复测也被同一容量错误拒绝，未启动。不能把上轮 debug Hook 的 110 秒结果算作本轮发行版复测。
3. 当前安装版 SHA-256 仍是 `5b70fd2b02f6a5e4d30da42985407c3b0a38f24bd08b6ce03447c19863cb5479`；待安装新版为 `10ccb5665d3b83be9268b16b14cbc634032f928d653355c2ada2f522dc56811e`。
4. 实际人工审批按钮与 AgentPulse 系统通知的同步显示、批准/拒绝后的真机清理，需要安装成功后继续验证。

结论：**完整自动化测试通过；本机安装和真实屏幕通知验收未完成。** 已请求用户切换人工审批以继续，没有绕过拒绝，也未提交 Git。
