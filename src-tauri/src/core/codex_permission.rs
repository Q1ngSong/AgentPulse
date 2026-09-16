//! Codex 权限请求的短观察窗口。
//!
//! Codex 的 PermissionRequest Hook 不区分“内部自动批准”和“正在等用户批准”。
//! 这里先把请求暂存一段观察窗口：如果随后看到对应工具开始或执行完成，说明已经被处理，
//! 不打扰用户；如果长期没有后续执行，才按普通权限请求发提醒。
use std::fs;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::dispatch;
use super::events::Context;
use super::log;
use super::paths::{atomic_write_json, chmod_private, FileLock};
use super::runtime::{hook_binary, Runtime};

pub const GRACE_SECONDS: f64 = 90.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    id: String,
    created: f64,
    agent: String,
    event: String,
    ctx: Context,
    payload: Value,
}

fn load(rt: &Runtime) -> Vec<Pending> {
    let Ok(text) = fs::read_to_string(rt.paths.codex_permission_pending()) else { return Vec::new() };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save(rt: &Runtime, items: &[Pending]) -> std::io::Result<()> {
    atomic_write_json(&rt.paths.codex_permission_pending(), &items)?;
    chmod_private(&rt.paths.codex_permission_pending());
    Ok(())
}

fn same_call(a: &Value, p: &Pending) -> bool {
    if a.get("agent").and_then(Value::as_str) != Some(p.agent.as_str()) {
        return false;
    }
    let Some(session) = p.ctx.session_id.as_deref().filter(|s| !s.is_empty()) else { return false };
    if a.get("session_id").and_then(Value::as_str) != Some(session) {
        return false;
    }
    let ts = a.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
    if ts + 0.001 < p.created {
        return false;
    }
    let request_id = p.ctx.tool_use_id.as_str();
    let done_id = a.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
    if !request_id.is_empty() && !done_id.is_empty() {
        return request_id == done_id;
    }
    let request_key = p.ctx.tool_input_key.as_str();
    let done_key = a.get("tool_input_key").and_then(Value::as_str).unwrap_or("");
    if !request_key.is_empty() && !done_key.is_empty() {
        return request_key == done_key;
    }
    p.ctx.tool_name.as_deref().map(|tool| a.get("tool_name").and_then(Value::as_str) == Some(tool)).unwrap_or(true)
}

fn task_finished(rt: &Runtime, p: &Pending) -> bool {
    let Some(session) = p.ctx.session_id.as_deref().filter(|s| !s.is_empty()) else { return false };
    log::read_events(&rt.paths, 200).iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some("hook")
            && e.get("agent").and_then(Value::as_str) == Some(p.agent.as_str())
            && e.get("event").and_then(Value::as_str) == Some("task_complete")
            && e.get("session_id").and_then(Value::as_str) == Some(session)
            && e.get("ts").and_then(Value::as_f64).unwrap_or(0.0) + 0.001 >= p.created
    })
}

fn auto_resolved(rt: &Runtime, p: &Pending) -> bool {
    log::read_activity(&rt.paths, 200).iter().any(|a| a.get("kind").and_then(Value::as_str) == Some("tool") && same_call(a, p))
        || task_finished(rt, p)
}

pub fn schedule(rt: &Runtime, agent: &str, event: &str, ctx: &Context, payload: &Value) -> std::io::Result<()> {
    let pending = Pending {
        id: uuid::Uuid::new_v4().simple().to_string(),
        created: log::now(),
        agent: agent.to_string(),
        event: event.to_string(),
        ctx: ctx.clone(),
        payload: payload.clone(),
    };
    {
        let _lock = FileLock::acquire(&rt.paths.codex_permission_lock())?;
        let mut items = load(rt);
        items.push(pending);
        save(rt, &items)?;
    }
    let mut cmd = Command::new(hook_binary());
    cmd.arg("--codex-permission-worker")
        .env("AGENTPULSE_HOME", &rt.paths.data_dir)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let _ = cmd.spawn();
    Ok(())
}

pub fn run_worker(rt: &Runtime) -> std::io::Result<()> {
    fs::create_dir_all(&rt.paths.data_dir)?;
    let Some(worker_lock) = FileLock::try_acquire(&rt.paths.codex_permission_worker_lock())? else { return Ok(()) };
    let result = run_loop(rt);
    drop(worker_lock);
    result
}

fn run_loop(rt: &Runtime) -> std::io::Result<()> {
    loop {
        let (due, wait) = {
            let _lock = FileLock::acquire(&rt.paths.codex_permission_lock())?;
            let items = load(rt);
            if items.is_empty() {
                return Ok(());
            }
            let now = log::now();
            let wait = items.iter().map(|p| p.created + GRACE_SECONDS - now).fold(f64::INFINITY, f64::min);
            let (due, keep): (Vec<_>, Vec<_>) = items.into_iter().partition(|p| now >= p.created + GRACE_SECONDS);
            save(rt, &keep)?;
            (due, wait)
        };
        if due.is_empty() {
            (rt.sleep)(Duration::from_secs_f64(wait.clamp(0.2, 1.0)));
            continue;
        }
        for p in due {
            if !auto_resolved(rt, &p) {
                dispatch::dispatch(rt, &p.agent, &p.event, &p.ctx, None, "hook", Some(&p.payload), None)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::channels::Registry;
    use crate::core::events::sample_context;
    use crate::core::platform::HostProbe;
    use crate::core::paths::temp_paths;

    #[derive(Clone)]
    struct Probe;
    impl HostProbe for Probe {
        fn frontmost_bundle(&self) -> String { String::new() }
        fn idle_seconds(&self) -> f64 { 99.0 }
        fn screen_locked(&self) -> bool { false }
    }

    fn runtime() -> (tempfile::TempDir, Runtime) {
        let (dir, paths) = temp_paths();
        let rt = Runtime {
            paths: paths.clone(),
            home: dir.path().join("user"),
            probe: Box::new(Probe),
            channels: Registry::empty(),
            sleep: Box::new(|_| {}),
            spawn_worker: Box::new(|| {}),
        };
        (dir, rt)
    }

    fn pending() -> Pending {
        let payload = serde_json::json!({
            "turn_id": "t1", "tool_name": "Bash", "tool_input": {"command": "true", "description": "run true"}
        });
        let mut ctx = sample_context("codex", "permission_request");
        ctx.session_id = Some("s1".into());
        ctx.tool_name = Some("Bash".into());
        ctx.tool_input_key = crate::core::events::tool_input_key(&payload);
        Pending {
            id: "p1".into(),
            created: log::now(),
            agent: "codex".into(),
            event: "permission_request".into(),
            ctx,
            payload,
        }
    }

    #[test]
    fn tool_activity_resolves_pending_codex_permission() {
        let (_dir, rt) = runtime();
        let p = pending();
        log::record_activity(&rt.paths, "codex", "", &serde_json::json!({
            "session_id": "s1", "turn_id": "t1", "tool_name": "Bash", "tool_input": {"command": "true"}, "hook_event_name": "PostToolUse"
        }), "tool").unwrap();
        assert!(auto_resolved(&rt, &p));
    }

    #[test]
    fn tool_start_activity_resolves_pending_codex_permission() {
        let (_dir, rt) = runtime();
        let p = pending();
        log::record_activity(&rt.paths, "codex", "", &serde_json::json!({
            "session_id": "s1", "turn_id": "t1", "tool_name": "Bash", "tool_input": {"command": "true"}, "hook_event_name": "PreToolUse"
        }), "tool").unwrap();
        assert!(auto_resolved(&rt, &p));
    }

    #[test]
    fn other_session_does_not_resolve_pending_codex_permission() {
        let (_dir, rt) = runtime();
        let p = pending();
        log::record_activity(&rt.paths, "codex", "", &serde_json::json!({
            "session_id": "s2", "tool_name": "Bash", "hook_event_name": "PostToolUse"
        }), "tool").unwrap();
        assert!(!auto_resolved(&rt, &p));
    }
}
