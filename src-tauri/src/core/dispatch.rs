//! 分发：收到提醒事件后，立即发送的马上发，设置了延迟的放进队列，并写提醒记录。
use serde_json::{json, Value};

use super::channels::{Message, SendResult};
use super::config::{self, Config, Target};
use super::events::{render, Context};
use super::log;
use super::queue;
use super::runtime::Runtime;

/// 按配置把事件发给匹配的提醒方式，返回写入的提醒记录。
/// source: "hook" / "simulate" / "test"；"test" 忽略延迟，永远立即发送。
/// targets 为 None 时取该工具的提醒方式，以及接收当前工具与来源 App 的桌宠。
/// 发送顺序：设了延迟的先入队，再发即时渠道。
pub fn dispatch(rt: &Runtime, agent: &str, event: &str, ctx: &Context, cfg: Option<&Config>, source: &str,
                payload: Option<&Value>, targets: Option<Vec<Target>>) -> std::io::Result<Value> {
    let loaded;
    let cfg = match cfg { Some(c) => c, None => { loaded = config::load_config(&rt.paths)?; &loaded } };
    let tpl = cfg.template(event);
    let message = Message { event: event.into(), title: render(&tpl.title, ctx), body: render(&tpl.body, ctx), agent: agent.into(), ctx: ctx.clone() };
    let targets = targets.unwrap_or_else(|| {
        cfg.tool_targets(agent).into_iter()
            .chain(cfg.pets.iter().filter(|pet| pet.matches(agent, &ctx.host_bundle)).map(|pet| pet.target()))
            .filter(|t| t.enabled && t.events.iter().any(|e| e == event)).collect()
    });
    let (now_targets, later): (Vec<Target>, Vec<Target>) = targets.into_iter().partition(|t| source == "test" || queue::delay_minutes(t) == 0);
    let mut entry = json!({
        "id": &uuid::Uuid::new_v4().simple().to_string()[..12], "ts": log::now(), "source": source,
        "agent": agent, "event": event, "project": ctx.project, "host_bundle": ctx.host_bundle, "session_id": ctx.session_id,
        "title": message.title, "body": message.body, "results": [],
        "payload": payload.map(|p| log::trim(p, 2000)).unwrap_or(Value::Null),
    });
    // 延迟队列先入队，再发即时渠道
    if !later.is_empty() {
        queue::enqueue(rt, &entry, &later, ctx)?;
    }
    let mut results: Vec<SendResult> = now_targets.iter().map(|t| rt.channels.send_one(t, &message)).collect();
    for t in &later {
        results.push(SendResult { target_id: Some(t.id.clone()), name: t.name.clone(), kind: Some(t.kind.clone()), ok: true, pending: true,
            info: Some(format!("等待 {} 分钟：期间有人在 {} 里工作就不发", queue::delay_minutes(t), ctx.agent)), ..Default::default() });
    }
    entry["results"] = json!(results);
    log::append_event(&rt.paths, &entry)?;
    Ok(entry)
}
