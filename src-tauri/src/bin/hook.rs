//! Claude Code / Codex 的 Hook 入口：agentpulse-hook <claude|codex>
//!
//! 从 stdin 读 Hook 数据，按配置发提醒，永远 exit 0，从不往 stdout 写任何东西（不会被工具当成
//! 任何形式的决定）。
//! `--queue-worker` 启动延迟队列的后台 worker；
//! `--install <工具>` / `--uninstall <工具>` 是命令行接入（子命令里只有这两个会往 stdout 打印结果）。
use std::io::{Read, Write};

use agentpulse_lib::core::{dispatch, events, log, paths::Paths, platform, queue, runtime::Runtime};
use serde_json::Value;

fn run() -> Result<(), String> {
    let paths = Paths::from_env();
    let rt = Runtime::system(paths.clone());
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--queue-worker") {
        return queue::run_queue_worker(&rt).map_err(|e| e.to_string());
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
    if name == "PreToolUse" { return Ok(()); }
    if name == "PostToolUse" {
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
