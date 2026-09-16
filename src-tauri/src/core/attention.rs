//! 注意力判定：同一来源 App 内某工具的延迟提醒，期间是否有人回来工作。
//! 有人值守仅当：从其他窗口切回宿主 App；宿主 App 在前台且上次检查后有键鼠操作；
//! 在该工具里发了新消息；等待中的权限请求对应的工具已执行（说明被批准了）；
//! 或同一对话有新的 Hook 事件。活动必须属于同一来源 App；来源不明时不据此取消。
use serde_json::Value;

use super::log;
use super::paths::Paths;
use super::platform::HostProbe;
use super::queue::Item;

/// 上一次检查时的前台状态与时间。
#[derive(Clone, Debug, Default)]
pub struct State {
    pub front: Option<bool>,
    pub last_check: Option<f64>,
}

/// 活动和提醒是否属于同一个工具、同一个已知来源 App。
fn same_source(record: &Value, item: &Item) -> bool {
    !item.ctx.host_bundle.is_empty()
        && record.get("host_bundle").and_then(Value::as_str) == Some(item.ctx.host_bundle.as_str())
        && record.get("agent").and_then(Value::as_str) == Some(item.agent.as_str())
}

/// since 之后同一来源 App、同一工具里是否有 UserPromptSubmit。
pub fn new_prompt(paths: &Paths, item: &Item, since: f64) -> bool {
    log::read_activity(paths, 50).iter().any(|a| {
        same_source(a, item)
            && a.get("kind").and_then(Value::as_str).unwrap_or("prompt") == "prompt"
            && a.get("ts").and_then(Value::as_f64).unwrap_or(0.0) > since
    })
}

/// 队列里的权限请求，是否在请求之后执行了同一对话、同一工具（PostToolUse），即已被批准。
pub fn permission_granted(paths: &Paths, items: &[Item]) -> bool {
    let done: Vec<Value> = log::read_activity(paths, 200).into_iter().filter(|a| a.get("kind").and_then(Value::as_str) == Some("tool")).collect();
    items.iter().any(|it| {
        let Some(sid) = it.ctx.session_id.as_deref().filter(|s| !s.is_empty()) else { return false };
        it.event == "permission_request" && it.ctx.hook_event != super::codex_permission::CONFIRMED
            && done.iter().any(|a| {
                same_source(a, it)
                    && a.get("session_id").and_then(Value::as_str) == Some(sid)
                    && a.get("ts").and_then(Value::as_f64).unwrap_or(0.0) > it.created
                    && it.ctx.tool_name.as_deref().map(|t| a.get("tool_name").and_then(Value::as_str) == Some(t)).unwrap_or(true)
            })
    })
}

/// 同一来源 App、同一对话在提醒之后又有新的 Hook 事件。
pub fn newer_activity(paths: &Paths, item: &Item) -> bool {
    if item.ctx.hook_event == super::codex_permission::CONFIRMED { return false; }
    let Some(sid) = item.ctx.session_id.as_deref().filter(|s| !s.is_empty()) else { return false };
    log::read_events(paths, 100).iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some("hook")
            && same_source(e, item)
            && e.get("session_id").and_then(Value::as_str) == Some(sid)
            && e.get("ts").and_then(Value::as_f64).unwrap_or(0.0) > item.created + 1.0
            && e.get("id").and_then(Value::as_str) != Some(item.r#ref.as_str())
    })
}

/// 同一来源 App 内这个工具是否有人在工作；items 必须来自同一 App、同一工具。
pub fn handled_reason(paths: &Paths, probe: &dyn HostProbe, agent: &str, items: &[Item], state: &mut State) -> Option<String> {
    let ctx = &items[0].ctx;
    let host = ctx.host_bundle.as_str();
    let name = if ctx.agent.is_empty() { agent } else { ctx.agent.as_str() };
    let since = items.iter().map(|i| i.created).fold(f64::INFINITY, f64::min);
    let last_check = state.last_check.unwrap_or(since);
    let front = !host.is_empty() && !probe.screen_locked() && probe.frontmost_bundle() == host;
    let prev_front = state.front.unwrap_or(items.last().map(|i| i.front_at_event).unwrap_or(false));
    let now = log::now();
    state.front = Some(front);
    state.last_check = Some(now);
    if front && !prev_front {
        return Some(format!("你从其他窗口切回了 {name}"));
    }
    if front && probe.idle_seconds() < now - last_check {
        return Some(format!("你在 {name} 里有操作"));
    }
    if permission_granted(paths, items) {
        return Some(format!("权限请求已在 {name} 里被批准（工具已执行）"));
    }
    if new_prompt(paths, &items[0], since) {
        return Some(format!("你在 {name} 里发了新消息"));
    }
    if items.iter().any(|it| newer_activity(paths, it)) {
        return Some("同一对话有了新动作".into());
    }
    None
}
