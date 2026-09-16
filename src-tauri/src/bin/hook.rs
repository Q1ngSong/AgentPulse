//! Claude Code / Codex 的 Hook 入口：agentpulse-hook <claude|codex>
//!
//! 从 stdin 读 Hook 数据，按配置发提醒，永远 exit 0，从不往 stdout 写任何东西（不会被工具当成
//! 任何形式的决定）。
//! `--queue-worker` 启动延迟队列的后台 worker；
//! `--install <工具>` / `--uninstall <工具>` 是命令行接入（子命令里只有这两个会往 stdout 打印结果）。
use std::io::{Read, Write};

use agentpulse_lib::core::{dispatch, events, log, paths::Paths, pet, platform, queue, runtime::Runtime};
use serde_json::Value;

fn run() -> Result<(), String> {
    let paths = Paths::from_env();
    let rt = Runtime::system(paths.clone());
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--queue-worker") {
        return queue::run_queue_worker(&rt).map_err(|e| e.to_string());
    }
    if args.first().map(String::as_str) == Some("--codex-permission-worker") {
        return agentpulse_lib::core::codex_permission::run_worker(&rt, args.get(1).map(String::as_str)).map_err(|e| e.to_string());
    }
    // --install / --uninstall <claude|codex>：命令行接入，写入本程序自身的路径（打包脚本和面板都能用）
    if let Some(action) = args.first().filter(|a| *a == "--install" || *a == "--uninstall") {
        let agent = args.get(1).cloned().unwrap_or_else(|| "claude".into());
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let status = if action == "--install" {
            let command = agentpulse_lib::core::integrations::hook_command(&me, &agent).map_err(|e| e.to_string())?;
            agentpulse_lib::core::integrations::install(&paths, &rt.home, &agent, &command)
        } else {
            agentpulse_lib::core::integrations::uninstall(&paths, &rt.home, &agent)
        }.map_err(|e| e.to_string())?;
        println!("{}", serde_json::to_string(&status).map_err(|e| e.to_string())?);
        return Ok(());
    }
    let agent = args.first().cloned().unwrap_or_else(|| "claude".into());
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).map_err(|e| e.to_string())?;
    let payload: Value = if input.trim().is_empty() { Value::Object(Default::default()) } else { serde_json::from_str(&input).map_err(|e| e.to_string())? };
    // 在标题查找和渠道发送前只取一次时间，后续异步投递仍使用同一事件顺序。
    let observed_at = log::now();
    let name = payload.get("hook_event_name").and_then(Value::as_str).unwrap_or("");
    if matches!(name, "UserPromptSubmit" | "PreToolUse" | "PostToolUse") {
        let project = payload.get("cwd").and_then(Value::as_str).and_then(|p| std::path::Path::new(p).file_name()).and_then(|s| s.to_str()).unwrap_or("");
        let tool = payload.get("tool_name").and_then(Value::as_str).unwrap_or("");
        pet::publish(&paths, &pet::Update {
            id: uuid::Uuid::new_v4().to_string(), phase: pet::Phase::Working,
            animations: None, agent: agent.clone(), host: platform::host_bundle_id(),
            session: payload.get("session_id").and_then(Value::as_str).unwrap_or("").into(),
            hook_event: name.into(), tool_use_id: events::tool_use_id(&payload), tool_input_key: events::tool_input_key(&payload), observed_at,
            title: format!("{} · {}", events::agent_name(&agent), project),
            detail: if name == "PreToolUse" { format!("正在使用 {tool}") } else if name == "PostToolUse" { format!("刚完成 {tool}，等待后续动作") } else { "已收到任务，工作中".into() },
        });
    }
    if name == "PreToolUse" {
        if agent == "codex" {
            log::record_activity(&paths, &agent, &platform::host_bundle_id(), &payload, "tool").map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if name == "PostToolUse" {
        if agent == "codex" {
            log::record_activity(&paths, &agent, &platform::host_bundle_id(), &payload, "tool").map_err(|e| e.to_string())?;
            return Ok(());
        }
        // 每次工具调用都会触发：只有这个对话有等待中的权限请求时才记录，其余直接退出
        if queue::has_pending_permission(&paths, payload.get("session_id").and_then(Value::as_str)) {
            log::record_activity(&paths, &agent, &platform::host_bundle_id(), &payload, "tool").map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if events::ACTIVITY_HOOKS.iter().any(|(_, h)| *h == name) {
        return log::record_activity(&paths, &agent, &platform::host_bundle_id(), &payload, "prompt").map_err(|e| e.to_string());
    }
    if let Some((event, mut ctx)) = events::normalize(&agent, &payload, &rt.home) {
        ctx.observed_at = observed_at;
        if name == "Stop" {
            pet::publish(&paths, &pet::Update {
                id: uuid::Uuid::new_v4().to_string(),
                phase: pet::Phase::Sleeping,
                animations: None, agent: agent.clone(), host: ctx.host_bundle.clone(), session: ctx.session_id.clone().unwrap_or_default(),
                hook_event: name.into(), tool_use_id: ctx.tool_use_id.clone(), tool_input_key: ctx.tool_input_key.clone(), observed_at,
                title: format!("{} · {}", ctx.agent, ctx.session), detail: ctx.detail.clone(),
            });
        }

        if agent == "codex" && event == "permission_request" {
            // Hook 只负责唤醒只读观察器。所有 permission_mode 都必须由真实待审批状态确认。
            return agentpulse_lib::core::codex_permission::schedule(&rt, &agent, &event, &ctx, &payload).map_err(|e| e.to_string());
        }
        dispatch::dispatch(&rt, &agent, &event, &ctx, None, "hook", Some(&payload), None).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        let paths = Paths::from_env();
        let _ = std::fs::create_dir_all(&paths.data_dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(paths.hook_errors()) {
            let _ = writeln!(f, "{} {e}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        }
    }
    std::process::exit(0);
}
