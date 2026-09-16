//! PermissionRequest 只唤醒观察器；只有 Codex 桌面的真实待审批请求才能发提醒。
//! 不用 permission_mode、工具运行时长或未收到 PostToolUse 推断人工审批。
use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{codex_approval::Observer, dispatch, log, pet, queue};
use super::events::Context;
use super::paths::{atomic_write_json, chmod_private, FileLock};
use super::runtime::{hook_binary, Runtime};

pub const CONFIRMED: &str = "CodexApprovalRequested";
pub const RESOLVED: &str = "CodexApprovalResolved";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    id: String,
    created: f64,
    agent: String,
    event: String,
    ctx: Context,
    payload: Value,
    #[serde(default)]
    notified: Vec<String>,
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
    if a["agent"] != p.agent || a["hook_event"] != "PostToolUse" { return false; }
    let Some(session) = p.ctx.session_id.as_deref().filter(|s| !s.is_empty()) else { return false };
    if a["session_id"] != session || a["ts"].as_f64().unwrap_or(0.0) + 0.001 < p.created { return false; }
    let id = a["tool_use_id"].as_str().unwrap_or("");
    if !p.ctx.tool_use_id.is_empty() && !id.is_empty() { return id == p.ctx.tool_use_id; }
    !p.ctx.tool_input_key.is_empty() && a["tool_input_key"] == p.ctx.tool_input_key
}

/// 超时和工具完成只能清理资源，绝不能触发提醒。
fn finished(p: &Pending, activity: &[Value], events: &[Value], now: f64) -> bool {
    now - p.created > 24.0 * 3600.0 || activity.iter().any(|a| same_call(a, p)) || events.iter().any(|e| {
        e["source"] == "hook" && e["agent"] == p.agent && e["event"] == "task_complete"
            && e["session_id"].as_str() == p.ctx.session_id.as_deref()
            && e["ts"].as_f64().unwrap_or(0.0) + 0.001 >= p.created
    })
}

pub fn schedule(rt: &Runtime, agent: &str, event: &str, ctx: &Context, payload: &Value) -> std::io::Result<()> {
    let Some(session) = ctx.session_id.as_deref().filter(|s| !s.is_empty()) else { return Ok(()) };
    if payload["turn_id"].as_str().filter(|s| !s.is_empty()).is_none() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Codex permission hook has no turn_id; notification suppressed"));
    }
    {
        let _lock = FileLock::acquire(&rt.paths.codex_permission_lock())?;
        let mut items = load(rt);
        items.push(Pending { id: uuid::Uuid::new_v4().simple().to_string(), created: log::now(),
            agent: agent.into(), event: event.into(), ctx: ctx.clone(), payload: payload.clone(), notified: Vec::new() });
        save(rt, &items)?;
    }
    let mut cmd = Command::new(hook_binary());
    cmd.args(["--codex-permission-worker", session])
        .env("AGENTPULSE_HOME", &rt.paths.data_dir)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()?;
    Ok(())
}

fn request_key(request: &Value) -> String {
    // JSON 保留数字 / 字符串 id 的区别，turnId 防止不同轮的 RPC id 复用。
    format!("codex-approval:{}:{}", request["params"]["turnId"], request["id"])
}

fn matches(p: &Pending, request: &Value) -> bool {
    p.ctx.session_id.as_deref().is_some_and(|s| request["params"]["threadId"] == s)
        && p.payload["turn_id"].as_str().is_some_and(|t| !t.is_empty() && request["params"]["turnId"] == t)
}

fn context(p: &Pending, request: &Value) -> Context {
    let mut ctx = p.ctx.clone();
    ctx.hook_event = CONFIRMED.into();
    ctx.tool_use_id = request_key(request);
    ctx.tool_input_key.clear();
    ctx.observed_at = log::now();
    // 以真实请求描述为准。同一轮可能有多个并行工具，不能用最后到达的 Hook 描述代替。
    ctx.detail = request["params"]["reason"].as_str().filter(|s| !s.is_empty())
        .or_else(|| request["params"]["command"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or(match request["method"].as_str() {
            Some("item/fileChange/requestApproval") => "Codex 正在等待你批准文件修改",
            Some("item/permissions/requestApproval") => "Codex 正在等待你授予权限",
            _ => "Codex 正在等待你批准命令执行",
        }).chars().take(500).collect();
    ctx
}

fn resolve(rt: &Runtime, ctx: &Context, known: bool) -> std::io::Result<()> {
    // 只取消这个请求，不清空同一 App 或同一轮里其他工具的提醒。
    queue::cancel_codex_approval(&rt.paths, ctx.session_id.as_deref().unwrap_or(""), &ctx.tool_use_id)?;
    pet::publish(&rt.paths, &pet::Update {
        id: uuid::Uuid::new_v4().to_string(), phase: if known { pet::Phase::Working } else { pet::Phase::Sleeping }, animations: None,
        agent: "codex".into(), host: ctx.host_bundle.clone(), session: ctx.session_id.clone().unwrap_or_default(),
        hook_event: RESOLVED.into(), tool_use_id: ctx.tool_use_id.clone(), tool_input_key: String::new(), observed_at: log::now(),
        title: format!("{} · {}", ctx.agent, ctx.session), detail: if known { "审批等待已结束" } else { "暂时无法读取审批状态" }.into(),
    });
    Ok(())
}

/// 对当前权威状态去重。持久化后才分发，worker 重启不会重复发送同一请求。
fn reconcile(rt: &Runtime, session: &str, requests: &[Value], active: &mut HashMap<String, Context>) -> std::io::Result<()> {
    let keys: HashSet<String> = requests.iter().map(request_key).collect();
    let removed: Vec<_> = active.keys().filter(|key| !keys.contains(*key)).cloned().collect();
    for key in removed {
        if let Some(ctx) = active.remove(&key) { resolve(rt, &ctx, true)?; }
    }
    let sends = {
        let _lock = FileLock::acquire(&rt.paths.codex_permission_lock())?;
        let mut items = load(rt);
        let mut sends = Vec::new();
        let mut changed = false;
        for request in requests {
            let key = request_key(request);
            if active.contains_key(&key) { continue; }
            let already_sent = items.iter().any(|p| p.ctx.session_id.as_deref() == Some(session) && p.notified.contains(&key));
            let Some(p) = items.iter_mut().rev().find(|p| matches(p, request)) else { continue };
            let ctx = context(p, request);
            active.insert(key.clone(), ctx.clone());
            if !already_sent {
                p.notified.push(key);
                changed = true;
                sends.push((ctx, request.clone()));
            }
        }
        if changed { save(rt, &items)?; }
        sends
    };
    for (ctx, request) in sends {
        let evidence = json!({"hook_event_name":CONFIRMED,"session_id":session,
            "turn_id":request["params"]["turnId"],"request_id":request["id"],"method":request["method"]});
        dispatch::dispatch(rt, "codex", "permission_request", &ctx, None, "hook", Some(&evidence), None)?;
    }
    Ok(())
}

pub fn run_worker(rt: &Runtime, session: Option<&str>) -> std::io::Result<()> {
    // 旧 worker 没有会话参数；不再处理旧的 90 秒推测队列。
    let Some(session) = session.filter(|s| !s.is_empty()) else { return Ok(()) };
    fs::create_dir_all(&rt.paths.data_dir)?;
    let lock_path = rt.paths.data_dir.join(format!("codex-approval-{:x}.lock", Sha256::digest(session.as_bytes())));
    let Some(worker_lock) = FileLock::try_acquire(&lock_path)? else { return Ok(()) };
    let mut worker_lock = Some(worker_lock);
    let mut active = HashMap::new();
    let result = observe(rt, session, &mut active, &mut worker_lock);
    // 断连不能继续声称还在等待批准。结束桌宠状态并取消尚未发送的远程提醒。
    for ctx in active.values() { let _ = resolve(rt, ctx, false); }
    // 与 schedule 共用锁：清理后先释放 worker 锁，再让新的 Hook 入队，避免丢失唤醒。
    if result.is_err() {
        let _lock = FileLock::acquire(&rt.paths.codex_permission_lock())?;
        let mut items = load(rt);
        items.retain(|p| p.ctx.session_id.as_deref() != Some(session));
        save(rt, &items)?;
        drop(worker_lock.take());
    }
    result.map_err(|e| std::io::Error::new(e.kind(), format!("Codex approval observation unavailable; no inferred notification: {e}")))
}

fn observe(rt: &Runtime, session: &str, active: &mut HashMap<String, Context>, worker_lock: &mut Option<FileLock>) -> std::io::Result<()> {
    let mut observer = Observer::connect(&rt.home, session)?;
    log::record_activity(&rt.paths, "codex", "", &json!({"session_id":session,
        "hook_event_name":"CodexApprovalObserverConnected"}), "approval_observer")?;
    let mut next_cleanup = Instant::now();
    loop {
        reconcile(rt, session, &observer.approvals(), active)?;
        if Instant::now() >= next_cleanup {
            let activity = log::read_activity(&rt.paths, 200);
            let events = log::read_events(&rt.paths, 200);
            let _lock = FileLock::acquire(&rt.paths.codex_permission_lock())?;
            let mut items = load(rt);
            let before = items.len();
            items.retain(|p| p.ctx.session_id.as_deref() != Some(session)
                || p.notified.iter().any(|key| active.contains_key(key))
                || (p.notified.is_empty() && !finished(p, &activity, &events, log::now())));
            if items.len() != before { save(rt, &items)?; }
            if !items.iter().any(|p| p.ctx.session_id.as_deref() == Some(session)) {
                drop(worker_lock.take());
                return Ok(());
            }
            next_cleanup = Instant::now() + Duration::from_secs(1);
        }
        observer.poll()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{channels::Registry, events::sample_context, platform::HostProbe, paths::temp_paths};
    struct Probe;
    impl HostProbe for Probe {
        fn frontmost_bundle(&self) -> String { String::new() }
        fn idle_seconds(&self) -> f64 { 99.0 }
        fn screen_locked(&self) -> bool { false }
    }
    fn runtime() -> (tempfile::TempDir, Runtime) {
        let (dir, paths) = temp_paths();
        let rt = Runtime { paths, home: dir.path().join("user"), probe: Box::new(Probe), channels: Registry::empty(),
            sleep: Box::new(|_| {}), spawn_worker: Box::new(|| {}) };
        (dir, rt)
    }
    fn pending() -> Pending {
        let payload = json!({"turn_id":"turn","tool_name":"Bash","tool_input":{"command":"sleep 110"}});
        let mut ctx = sample_context("codex", "permission_request");
        ctx.session_id = Some("session".into());
        ctx.tool_input_key = crate::core::events::tool_input_key(&payload);
        Pending { id:"p".into(), created:log::now()-120.0, agent:"codex".into(), event:"permission_request".into(), ctx, payload, notified:Vec::new() }
    }
    fn request() -> Value { json!({"id":7,"method":"item/commandExecution/requestApproval","params":{"threadId":"session","turnId":"turn","reason":"Approval test"}}) }
    #[test]
    fn long_auto_approved_tool_without_post_never_notifies() {
        let (_dir, rt) = runtime();
        save(&rt, &[pending()]).unwrap();
        let mut active = HashMap::new();
        for _ in 0..3 { reconcile(&rt, "session", &[], &mut active).unwrap(); }
        assert!(log::read_events(&rt.paths, 100).is_empty());
        assert!(active.is_empty());
        assert!(!finished(&pending(), &[], &[], log::now()));
    }
    #[test]
    fn actual_user_request_notifies_once_and_resolution_clears_it() {
        let (_dir, rt) = runtime();
        save(&rt, &[pending()]).unwrap();
        let mut active = HashMap::new();
        reconcile(&rt, "session", &[request()], &mut active).unwrap();
        assert_eq!(log::read_events(&rt.paths, 100).len(), 1);
        reconcile(&rt, "session", &[request()], &mut active).unwrap();
        // 模拟重读快照 / worker 的内存状态丢失：磁盘上的去重标记仍有效。
        active.clear();
        reconcile(&rt, "session", &[request()], &mut active).unwrap();
        assert_eq!(log::read_events(&rt.paths, 100).len(), 1);
        reconcile(&rt, "session", &[], &mut active).unwrap();
        assert!(active.is_empty());
    }
    #[test]
    fn other_turn_does_not_borrow_hook_context() {
        let (_dir, rt) = runtime();
        save(&rt, &[pending()]).unwrap();
        let mut r = request();
        r["params"]["turnId"] = json!("other");
        reconcile(&rt, "session", &[r], &mut HashMap::new()).unwrap();
        assert!(log::read_events(&rt.paths, 100).is_empty());
    }
    #[test]
    fn pre_tool_use_does_not_mean_approval_resolved() {
        let p = pending();
        let mut a = json!({"agent":"codex","session_id":"session","ts":log::now(),
            "hook_event":"PreToolUse","tool_input_key":p.ctx.tool_input_key});
        assert!(!same_call(&a, &p));
        a["hook_event"] = json!("PostToolUse");
        assert!(same_call(&a, &p));
    }
}
