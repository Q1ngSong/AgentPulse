//! 延迟队列：外部提醒先入队，由唯一的后台 worker 每分钟检查，无人值守到期后合并发送。
//! 按来源 App 隔离注意力状态、合并和清理；同一 App 内仍按工具使用各自的提醒配置。
//! 队列文件只存提醒方式的 id，发送时按当前配置取提醒方式；停用、删除或取消事件订阅后不再发送。
//! 锁只保护读写 queue.json，判断注意力和网络发送都在锁外进行，避免阻塞 Hook 入队。
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::attention::{self, State};
use super::channels::{Message, SendResult};
use super::config::{self, Target};
use super::events::Context;
use super::log;
use super::paths::{atomic_write_json, chmod_private, FileLock};
use super::runtime::Runtime;

pub const QUEUE_POLL_SECONDS: f64 = 60.0; // 队列非空时每分钟检查一次“有没有人在工具里工作”
pub const MAX_ATTEMPTS: u32 = 3;          // 同一条提醒对同一个提醒方式最多发送 3 次
pub const RETRY_SECONDS: f64 = 60.0;      // 第 n 次失败后等 n 分钟再试

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub r#ref: String,
    pub created: f64,
    pub agent: String,
    pub event: String,
    #[serde(default)]
    pub front_at_event: bool,
    pub title: String,
    pub body: String,
    pub ctx: Context,
    #[serde(deserialize_with = "target_ids")]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attempts: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub next_try: BTreeMap<String, f64>,
}

/// 旧格式的 targets 是完整配置（dict），只取 id。
fn target_ids<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let raw: Vec<Value> = Vec::deserialize(d)?;
    Ok(raw.into_iter().filter_map(|v| match v {
        Value::String(s) => Some(s),
        Value::Object(m) => m.get("id").and_then(Value::as_str).map(String::from),
        _ => None,
    }).collect())
}

/// 提醒方式配置的延迟分钟数；非法值按 0（立即发送）处理。
pub fn delay_minutes(target: &Target) -> u64 {
    target.u64("delay_minutes")
}

/// 读取队列；文件不存在或损坏时返回空列表，缺字段的条目丢弃。
pub fn load_queue(paths: &super::paths::Paths) -> Vec<Item> {
    let Ok(text) = fs::read_to_string(paths.queue()) else { return Vec::new() };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    items.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect()
}

pub fn save_queue(paths: &super::paths::Paths, items: &[Item]) -> std::io::Result<()> {
    atomic_write_json(&paths.queue(), &items)?;
    chmod_private(&paths.queue());
    Ok(())
}

/// 队列里是否有这个对话等待中的权限请求；PostToolUse 用它决定要不要记录。
pub fn has_pending_permission(paths: &super::paths::Paths, session_id: Option<&str>) -> bool {
    let Some(sid) = session_id.filter(|s| !s.is_empty()) else { return false };
    load_queue(paths).iter().any(|it| it.event == "permission_request" && it.ctx.session_id.as_deref() == Some(sid))
}

/// 把待发的外部提醒放进队列，并尝试启动后台 worker（已有 worker 时新进程立即退出）。
pub fn enqueue(rt: &Runtime, entry: &Value, targets: &[Target], ctx: &Context) -> std::io::Result<()> {
    let host = ctx.host_bundle.as_str();
    let item = Item {
        r#ref: entry["id"].as_str().unwrap_or("").into(),
        created: entry["ts"].as_f64().unwrap_or_else(log::now),
        agent: entry["agent"].as_str().unwrap_or("").into(),
        event: entry["event"].as_str().unwrap_or("").into(),
        front_at_event: !host.is_empty() && !rt.probe.screen_locked() && rt.probe.frontmost_bundle() == host,
        title: entry["title"].as_str().unwrap_or("").into(),
        body: entry["body"].as_str().unwrap_or("").into(),
        ctx: ctx.clone(),
        targets: targets.iter().map(|t| t.id.clone()).collect(),
        attempts: BTreeMap::new(),
        next_try: BTreeMap::new(),
    };
    {
        let _lock = FileLock::acquire(&rt.paths.queue_lock())?;
        let mut items = load_queue(&rt.paths);
        items.push(item);
        save_queue(&rt.paths, &items)?;
    }
    (rt.spawn_worker)();
    Ok(())
}

/// 多条提醒合并成一条 (event, title, body)；有请求权限时按请求权限的样式。
pub fn merged_message(items: &[Item]) -> (String, String, String) {
    if items.len() == 1 {
        return (items[0].event.clone(), items[0].title.clone(), items[0].body.clone());
    }
    let mut lines: Vec<String> = items.iter().take(10).map(|it| {
        let icon = match it.event.as_str() { "permission_request" => "🔐", "task_complete" => "✅", _ => "•" };
        format!("{icon} {}", it.title)
    }).collect();
    if items.len() > 10 {
        lines.push(format!("…另外 {} 条", items.len() - 10));
    }
    let urgent = items.iter().any(|it| it.event == "permission_request");
    ((if urgent { "permission_request" } else { "task_complete" }).into(), format!("AgentPulse：{} 条提醒无人处理", items.len()), lines.join("\n"))
}

fn log_delayed(paths: &super::paths::Paths, items: &[Item], results: &[SendResult], title: &str, body: &str) {
    let it = &items[0];
    let _ = log::append_event(paths, &json!({
        "id": &uuid::Uuid::new_v4().simple().to_string()[..12], "ts": log::now(), "source": "delayed",
        "agent": it.agent, "event": it.event, "project": it.ctx.project, "host_bundle": it.ctx.host_bundle,
        "session_id": it.ctx.session_id, "title": title, "body": body, "results": results,
        "payload": {"note": "延迟队列", "refs": items.iter().map(|x| x.r#ref.clone()).collect::<Vec<_>>()},
    }));
}

fn skip(target_id: Option<&str>, name: &str, info: String) -> SendResult {
    SendResult { target_id: target_id.map(String::from), name: name.into(), kind: None, ok: true, skipped: true, info: Some(info), ..Default::default() }
}

/// 一个来源 App 内某工具本轮的决定：移出队列的 ref，待发送的批次，要写的记录。
struct Plan {
    drop_refs: BTreeSet<String>,
    sends: Vec<(Option<Target>, String, Vec<Item>)>,
    logs: Vec<(Vec<Item>, Vec<SendResult>, String, String)>,
}

fn plan_group(rt: &Runtime, agent: &str, group: &[Item], targets: &HashMap<String, Target>, now: f64, state: &mut State) -> Plan {
    let mut plan = Plan { drop_refs: BTreeSet::new(), sends: Vec::new(), logs: Vec::new() };
    if let Some(reason) = attention::handled_reason(&rt.paths, rt.probe.as_ref(), agent, group, state) {
        let tids: BTreeSet<&String> = group.iter().flat_map(|it| it.targets.iter()).collect();
        let results: Vec<SendResult> = tids.iter().map(|tid| {
            let name = targets.get(*tid).map(|t| t.name.as_str()).unwrap_or(tid);
            skip(Some(tid), name, format!("未发送（{} 条）：{reason}", group.len()))
        }).collect();
        plan.drop_refs = group.iter().map(|it| it.r#ref.clone()).collect();
        plan.logs.push((group.to_vec(), results, format!("{} 条提醒已取消", group.len()), reason));
        return plan;
    }
    let tids: BTreeSet<String> = group.iter().flat_map(|it| it.targets.iter().cloned()).collect();
    for tid in tids {
        let waiting: Vec<Item> = group.iter().filter(|it| it.targets.contains(&tid)).cloned().collect();
        let Some(target) = targets.get(&tid).filter(|t| t.enabled) else {
            plan.logs.push((waiting.clone(), vec![skip(Some(&tid), &tid, "未发送：提醒方式已删除或停用".into())], waiting[0].title.clone(), waiting[0].body.clone()));
            plan.sends.push((None, tid, waiting)); // 只从队列里移除
            continue;
        };
        let (waiting, cancelled): (Vec<Item>, Vec<Item>) = waiting.into_iter().partition(|it| target.events.contains(&it.event));
        if !cancelled.is_empty() {
            plan.logs.push((cancelled.clone(), vec![skip(Some(&tid), &target.name, "未发送：已取消事件订阅".into())], cancelled[0].title.clone(), cancelled[0].body.clone()));
            plan.sends.push((None, tid.clone(), cancelled));
        }
        let due: Vec<Item> = waiting.into_iter().filter(|it| {
            now >= (it.created + delay_minutes(target) as f64 * 60.0).max(it.next_try.get(&tid).copied().unwrap_or(0.0))
        }).collect();
        if due.is_empty() {
            continue;
        }
        if target.bool_or("batch", true) {
            plan.sends.push((Some(target.clone()), tid, due));
        } else {
            for it in due {
                plan.sends.push((Some(target.clone()), tid.clone(), vec![it]));
            }
        }
    }
    plan
}

/// 后台 worker 入口：拿不到 worker 锁说明已有进程在处理，直接返回。
pub fn run_queue_worker(rt: &Runtime) -> std::io::Result<()> {
    fs::create_dir_all(&rt.paths.data_dir)?;
    let Some(worker_lock) = FileLock::try_acquire(&rt.paths.worker_lock())? else { return Ok(()) };
    let result = queue_loop(rt);
    drop(worker_lock);
    result
}

fn queue_loop(rt: &Runtime) -> std::io::Result<()> {
    let mut states: HashMap<(String, String), State> = HashMap::new();
    loop {
        let items = {
            let _lock = FileLock::acquire(&rt.paths.queue_lock())?;
            let items = load_queue(&rt.paths);
            if items.is_empty() {
                return Ok(());
            }
            items
        };
        let now = log::now();
        let cfg = config::load_config(&rt.paths)?;
        let agents: BTreeSet<String> = items.iter().map(|it| it.agent.clone()).collect();
        let targets_of: HashMap<String, HashMap<String, Target>> = agents.iter()
            .map(|a| (a.clone(), cfg.tool_targets(a).into_iter().map(|t| (t.id.clone(), t)).collect())).collect();
        let groups: BTreeSet<(String, String)> = items.iter().map(|it| (it.agent.clone(), it.ctx.host_bundle.clone())).collect();

        let mut drop_refs: BTreeSet<String> = BTreeSet::new();
        let mut outcomes: Vec<(String, Vec<Item>, bool)> = Vec::new(); // (tid, batch, finished)
        for (agent, host) in &groups {
            let group: Vec<Item> = items.iter().filter(|it| &it.agent == agent && &it.ctx.host_bundle == host).cloned().collect();
            let empty = HashMap::new();
            let targets = targets_of.get(agent).unwrap_or(&empty);
            let state = states.entry((agent.clone(), host.clone())).or_default();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let plan = plan_group(rt, agent, &group, targets, now, state);
                let mut local = Vec::new();
                for (items_, results, title, body) in &plan.logs {
                    log_delayed(&rt.paths, items_, results, title, body);
                }
                for (target, tid, batch) in plan.sends {
                    let Some(target) = target else { local.push((tid, batch, true)); continue };
                    // 前一批发送期间配置可能已变化，每批发送前重新核对。
                    let current = match config::load_config(&rt.paths) {
                        Ok(cfg) => cfg.tool_targets(agent).into_iter().find(|t| t.id == tid && t.enabled),
                        Err(e) => {
                            let r = SendResult { target_id: Some(tid), name: target.name, error: Some(format!("无法读取配置，暂缓发送：{e}")), ..Default::default() };
                            log_delayed(&rt.paths, &batch, &[r], &batch[0].title, &batch[0].body);
                            continue;
                        }
                    };
                    let (batch, cancelled): (Vec<Item>, Vec<Item>) = batch.into_iter()
                        .partition(|it| current.as_ref().is_some_and(|t| t.events.contains(&it.event)));
                    if !cancelled.is_empty() {
                        let reason = if current.is_some() { "已取消事件订阅" } else { "提醒方式已删除或停用" };
                        log_delayed(&rt.paths, &cancelled, &[skip(Some(&tid), &target.name, format!("未发送：{reason}"))], &cancelled[0].title, &cancelled[0].body);
                        local.push((tid.clone(), cancelled, true));
                    }
                    if batch.is_empty() { continue; }
                    let target = current.unwrap();
                    let (event, title, body) = merged_message(&batch);
                    let msg = Message { event, title: title.clone(), body: body.clone(), agent: agent.clone(), ctx: batch.last().unwrap().ctx.clone() };
                    let mut r = rt.channels.send_one(&target, &msg);
                    let mut head = format!("{} 分钟内无人处理，{}", delay_minutes(&target), if batch.len() > 1 { format!("合并 {} 条发送", batch.len()) } else { "已发送".into() });
                    if !r.ok {
                        let exhausted = batch.iter().filter(|it| it.attempts.get(&tid).copied().unwrap_or(0) + 1 >= MAX_ATTEMPTS).count();
                        head += "，发送失败";
                        if exhausted < batch.len() { head += &format!("，{} 条稍后重试", batch.len() - exhausted); }
                        if exhausted > 0 { head += &format!("，{exhausted} 条不再重试"); }
                    }
                    r.info = Some(match r.info.take() { Some(i) => format!("{head}（{i}）"), None => head });
                    log_delayed(&rt.paths, &batch, std::slice::from_ref(&r), &title, &body);
                    local.push((tid, batch, r.ok));
                }
                (plan.drop_refs, local)
            }));
            match outcome {
                Ok((refs, local)) => { drop_refs.extend(refs); outcomes.extend(local); }
                Err(e) => {
                    let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    drop_refs.extend(group.iter().map(|it| it.r#ref.clone()));
                    log_delayed(&rt.paths, &group, &[skip(None, "延迟队列", format!("处理出错，已丢弃 {} 条：{msg}", group.len()))], &group[0].title, &group[0].body);
                }
            }
        }

        let next_due = {
            let _lock = FileLock::acquire(&rt.paths.queue_lock())?;
            let mut fresh = load_queue(&rt.paths); // 发送期间可能有新提醒入队
            for (tid, batch, done) in &outcomes {
                for sent in batch {
                    let Some(it) = fresh.iter_mut().find(|it| it.r#ref == sent.r#ref) else { continue };
                    if !it.targets.contains(tid) {
                        continue;
                    }
                    let n = it.attempts.get(tid).copied().unwrap_or(0) + 1;
                    if *done || n >= MAX_ATTEMPTS {
                        it.targets.retain(|t| t != tid);
                    } else {
                        it.attempts.insert(tid.clone(), n);
                        it.next_try.insert(tid.clone(), log::now() + RETRY_SECONDS * n as f64);
                    }
                }
            }
            let seen_refs: BTreeSet<&String> = items.iter().map(|it| &it.r#ref).collect();
            let keep: Vec<Item> = fresh.into_iter().filter(|it| !drop_refs.contains(&it.r#ref) && !it.targets.is_empty()).collect();
            save_queue(&rt.paths, &keep)?;
            let still: BTreeSet<(&str, &str)> = keep.iter().filter(|it| seen_refs.contains(&it.r#ref))
                .map(|it| (it.agent.as_str(), it.ctx.host_bundle.as_str())).collect();
            // 本轮已处理完的 App / 工具组合清掉状态；同一工具在其他 App 的状态继续保留。
            states.retain(|(agent, host), _| still.contains(&(agent.as_str(), host.as_str())));
            if keep.is_empty() {
                return Ok(());
            }
            let targets_ref = &targets_of;
            keep.iter().flat_map(|it| it.targets.iter().map(move |tid| {
                let d = targets_ref.get(&it.agent).and_then(|m| m.get(tid)).map(delay_minutes).unwrap_or(0) as f64;
                (it.created + d * 60.0).max(it.next_try.get(tid).copied().unwrap_or(0.0))
            })).fold(f64::INFINITY, f64::min)
        };
        let wait = (next_due - log::now()).min(QUEUE_POLL_SECONDS).max(1.0);
        (rt.sleep)(Duration::from_secs_f64(wait));
    }
}

/// 面板统计用：每个工具待发条数。
pub fn queued_per_agent(paths: &super::paths::Paths) -> Map<String, Value> {
    let mut m = Map::new();
    for it in load_queue(paths) {
        let e = m.entry(it.agent).or_insert(json!(0));
        *e = json!(e.as_u64().unwrap_or(0) + 1);
    }
    m
}
