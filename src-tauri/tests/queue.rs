//! 分发与延迟队列：对应 Python tests/test_queue.py 的用例。
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentpulse_lib::core::channels::{Channel, Message, Registry};
use agentpulse_lib::core::config::{self, Config, Target};
use agentpulse_lib::core::events::{sample_context, Context};
use agentpulse_lib::core::platform::HostProbe;
use agentpulse_lib::core::runtime::Runtime;
use agentpulse_lib::core::{attention, dispatch, log, paths::Paths, queue};
use serde_json::{json, Value};

const CLAUDE: &str = "com.anthropic.claudefordesktop";
const CHROME: &str = "com.google.Chrome";
const TERMINAL: &str = "com.apple.Terminal";
const VSCODE: &str = "com.microsoft.VSCode";

#[derive(Clone, Default)]
struct Fake {
    front: Arc<Mutex<String>>,
    idle: Arc<Mutex<f64>>,
    locked: Arc<Mutex<bool>>,
}
impl HostProbe for Fake {
    fn frontmost_bundle(&self) -> String { self.front.lock().unwrap().clone() }
    fn idle_seconds(&self) -> f64 { *self.idle.lock().unwrap() }
    fn screen_locked(&self) -> bool { *self.locked.lock().unwrap() }
}

type Sent = Arc<Mutex<Vec<(String, String)>>>;
type Behaviour = Arc<Mutex<Box<dyn FnMut(&Target, &Message) -> Result<Option<String>, String> + Send>>>;

struct FakeChannel {
    sent: Sent,
    behaviour: Behaviour,
}
impl Channel for FakeChannel {
    fn send(&self, target: &Target, message: &Message) -> Result<Option<String>, String> {
        let r = (self.behaviour.lock().unwrap())(target, message);
        if r.is_ok() {
            self.sent.lock().unwrap().push((message.title.clone(), message.body.clone()));
        }
        r
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    rt: Runtime,
    fake: Fake,
    sent: Sent,
    behaviour: Behaviour,
    sleeps: Arc<Mutex<Vec<f64>>>,
    on_sleep: Arc<Mutex<Option<Box<dyn Fn(&Paths) + Send + Sync>>>>,
    spawned: Arc<Mutex<u32>>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("home"));
    let fake = Fake { front: Arc::new(Mutex::new(CHROME.into())), idle: Arc::new(Mutex::new(1.0)), locked: Arc::new(Mutex::new(false)) };
    let sent: Sent = Arc::default();
    let behaviour: Behaviour = Arc::new(Mutex::new(Box::new(|_, _| Ok(Some("ok".into())))));
    let mut channels = Registry::empty();
    // 同一个假渠道注册成三种 kind：desktop / sound 用来验证 dispatch 的发送顺序，wxtest 是其余用例用的
    for kind in ["wxtest", "desktop", "sound"] {
        channels.register(kind, Box::new(FakeChannel { sent: sent.clone(), behaviour: behaviour.clone() }));
    }
    let sleeps: Arc<Mutex<Vec<f64>>> = Arc::default();
    let on_sleep: Arc<Mutex<Option<Box<dyn Fn(&Paths) + Send + Sync>>>> = Arc::default();
    let spawned: Arc<Mutex<u32>> = Arc::default();
    let (s2, o2, p2, sp2) = (sleeps.clone(), on_sleep.clone(), paths.clone(), spawned.clone());
    let rt = Runtime {
        paths: paths.clone(), home: dir.path().join("user"), probe: Box::new(fake.clone()), channels,
        sleep: Box::new(move |d: Duration| { s2.lock().unwrap().push(d.as_secs_f64()); if let Some(f) = o2.lock().unwrap().as_ref() { f(&p2) } }),
        spawn_worker: Box::new(move || *sp2.lock().unwrap() += 1),
    };
    Harness { _dir: dir, rt, fake, sent, behaviour, sleeps, on_sleep, spawned }
}

fn target(delay: u64, batch: bool) -> Target {
    serde_json::from_value(json!({"id": "w", "type": "wxtest", "name": "微信", "enabled": true, "delay_minutes": delay, "batch": batch,
        "events": ["permission_request", "task_complete"]})).unwrap()
}

fn cfg_with(h: &Harness, targets: &[Target]) -> Config {
    let mut cfg = config::default_config();
    cfg.set_tool_targets("claude", targets);
    cfg.set_tool_targets("codex", targets);
    config::save_config(&h.rt.paths, &cfg).unwrap();
    cfg
}

/// 入队 n 条（各自独立会话），并把入队时间和记录时间都挪到 backdate 秒前。
fn fill(h: &Harness, n: usize, cfg: &Config, backdate: f64) {
    for k in 0..n {
        let event = if k % 2 == 1 { "task_complete" } else { "permission_request" };
        let mut ctx = sample_context("claude", event);
        ctx.session_id = Some(uuid::Uuid::new_v4().simple().to_string());
        ctx.session = format!("对话{k}");
        dispatch::dispatch(&h.rt, "claude", event, &ctx, Some(cfg), "hook", None, None).unwrap();
    }
    let mut items = queue::load_queue(&h.rt.paths);
    for it in &mut items { it.created -= backdate; }
    queue::save_queue(&h.rt.paths, &items).unwrap();
    let text = fs::read_to_string(h.rt.paths.events()).unwrap();
    let out: Vec<String> = text.lines().map(|l| { let mut v: Value = serde_json::from_str(l).unwrap(); v["ts"] = json!(v["ts"].as_f64().unwrap() - backdate); v.to_string() }).collect();
    fs::write(h.rt.paths.events(), format!("{}\n", out.join("\n"))).unwrap();
}

fn last_info(h: &Harness) -> String {
    log::read_events(&h.rt.paths, 1)[0]["results"][0]["info"].as_str().unwrap_or("").to_string()
}

fn set_front(h: &Harness, b: &str) { *h.fake.front.lock().unwrap() = b.into(); }
fn set_idle(h: &Harness, s: f64) { *h.fake.idle.lock().unwrap() = s; }

// ---------- dispatch ----------

#[test]
fn immediate_target_sent_and_logged() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(0, true)]);
    let e = dispatch::dispatch(&h.rt, "claude", "task_complete", &sample_context("claude", "task_complete"), Some(&cfg), "hook", None, None).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 1);
    assert_eq!(e["title"], "Claude Code: AgentPulse 测试对话");
    assert_eq!(e["results"][0]["ok"], true);
    assert_eq!(log::read_events(&h.rt.paths, 1)[0]["id"], e["id"]);
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn delayed_target_goes_to_queue_and_spawns_worker() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(5, true)]);
    let e = dispatch::dispatch(&h.rt, "claude", "task_complete", &sample_context("claude", "task_complete"), Some(&cfg), "hook", None, None).unwrap();
    assert!(h.sent.lock().unwrap().is_empty());
    assert_eq!(e["results"][0]["pending"], true);
    assert_eq!(queue::load_queue(&h.rt.paths).len(), 1);
    assert_eq!(*h.spawned.lock().unwrap(), 1);
}

#[test]
fn test_source_ignores_delay() {
    let h = harness();
    let e = dispatch::dispatch(&h.rt, "claude", "task_complete", &sample_context("claude", "task_complete"), None, "test", None, Some(vec![target(5, true)])).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 1);
    assert!(queue::load_queue(&h.rt.paths).is_empty());
    assert_eq!(e["results"][0]["ok"], true);
}

#[test]
fn disabled_or_unsubscribed_targets_skipped() {
    let h = harness();
    let mut off = target(0, true); off.enabled = false;
    let mut other = target(0, true); other.id = "o".into(); other.events = vec!["permission_request".into()];
    let cfg = cfg_with(&h, &[off, other]);
    let e = dispatch::dispatch(&h.rt, "claude", "task_complete", &sample_context("claude", "task_complete"), Some(&cfg), "hook", None, None).unwrap();
    assert_eq!(e["results"].as_array().unwrap().len(), 0);
}

#[test]
fn dispatch_uses_only_that_tools_targets() {
    let h = harness();
    let mut cfg = config::default_config();
    let a: Target = serde_json::from_value(json!({"id": "a", "type": "wxtest", "name": "A", "enabled": true, "events": ["task_complete"]})).unwrap();
    let b: Target = serde_json::from_value(json!({"id": "b", "type": "wxtest", "name": "B", "enabled": true, "events": ["task_complete"]})).unwrap();
    cfg.set_tool_targets("claude", &[a]);
    cfg.set_tool_targets("codex", &[b]);
    let e = dispatch::dispatch(&h.rt, "codex", "task_complete", &sample_context("codex", "task_complete"), Some(&cfg), "hook", None, None).unwrap();
    assert_eq!(e["results"][0]["name"], "B");
    assert_eq!(e["results"].as_array().unwrap().len(), 1);
}

#[test]
fn delayed_targets_are_enqueued_before_immediate_channels_are_sent() {
    let h = harness();
    let desktop: Target = serde_json::from_value(json!({"id": "d", "type": "desktop", "name": "浮层", "enabled": true,
        "mode": "overlay", "events": ["permission_request"]})).unwrap();
    let sound: Target = serde_json::from_value(json!({"id": "s", "type": "sound", "name": "声音", "enabled": true,
        "events": ["permission_request"]})).unwrap();
    let delayed: Target = serde_json::from_value(json!({"id": "w", "type": "wxtest", "name": "微信", "enabled": true,
        "delay_minutes": 5, "events": ["permission_request"]})).unwrap();
    let cfg = cfg_with(&h, &[desktop, sound, delayed]);
    let queued_when_first_sent: Arc<Mutex<Option<usize>>> = Arc::default();
    let (q2, p2) = (queued_when_first_sent.clone(), h.rt.paths.clone());
    let mut first = true;
    *h.behaviour.lock().unwrap() = Box::new(move |_: &Target, _: &Message| {
        if first {
            first = false;
            *q2.lock().unwrap() = Some(queue::load_queue(&p2).len());
        }
        Ok(None)
    });
    let e = dispatch::dispatch(&h.rt, "claude", "permission_request", &sample_context("claude", "permission_request"), Some(&cfg), "hook", None, None).unwrap();
    assert_eq!(*queued_when_first_sent.lock().unwrap(), Some(1), "延迟目标要在即时渠道发送开始之前就入队");
    assert_eq!(e["results"].as_array().unwrap().len(), 3, "三个提醒方式的结果都要写进记录");
}

// ---------- worker ----------

#[test]
fn merge_twelve_into_one_and_clear() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 12, &cfg, 61.0);
    queue::run_queue_worker(&h.rt).unwrap();
    let sent = h.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "AgentPulse：12 条提醒无人处理");
    assert_eq!(sent[0].1.lines().count(), 11);
    assert_eq!(sent[0].1.lines().last().unwrap(), "…另外 2 条");
    assert!(last_info(&h).contains("合并 12 条发送"));
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn one_by_one_when_batch_off() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, false)]);
    fill(&h, 12, &cfg, 61.0);
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 12);
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn new_prompt_cancels_same_apps_queue() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 12, &cfg, 61.0);
    log::record_activity(&h.rt.paths, "claude", CLAUDE, &json!({}), "prompt").unwrap();
    queue::run_queue_worker(&h.rt).unwrap();
    assert!(h.sent.lock().unwrap().is_empty());
    let r = &log::read_events(&h.rt.paths, 1)[0]["results"][0];
    assert_eq!(r["skipped"], true);
    assert!(r["info"].as_str().unwrap().contains("发了新消息"));
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn switch_back_to_host_cancels() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 3, &cfg, 0.0);
    set_front(&h, CLAUDE); set_idle(&h, 999.0);
    queue::run_queue_worker(&h.rt).unwrap();
    assert!(h.sent.lock().unwrap().is_empty());
    assert!(last_info(&h).contains("切回了 Claude Code"));
}

#[test]
fn focus_clears_only_that_apps_queue() {
    for front in [TERMINAL, VSCODE, CLAUDE] {
        let h = harness();
        let cfg = cfg_with(&h, &[target(5, true)]);
        fill(&h, 6, &cfg, 0.0);
        let mut items = queue::load_queue(&h.rt.paths);
        for (it, host) in items.iter_mut().zip([TERMINAL, TERMINAL, VSCODE, VSCODE, CLAUDE, CLAUDE]) {
            it.ctx.host_bundle = host.into();
        }
        queue::save_queue(&h.rt.paths, &items).unwrap();
        set_front(&h, front); set_idle(&h, 999.0);
        *h.on_sleep.lock().unwrap() = Some(Box::new(move |p| {
            let remaining = queue::load_queue(p);
            assert_eq!(remaining.len(), 4, "只清空当前 App 的两条提醒");
            assert!(remaining.iter().all(|it| it.ctx.host_bundle != front));
            queue::save_queue(p, &[]).unwrap();
        }));
        queue::run_queue_worker(&h.rt).unwrap();
        assert_eq!(h.sleeps.lock().unwrap().len(), 1);
        assert!(h.sent.lock().unwrap().is_empty());
        let records = log::read_events(&h.rt.paths, 20);
        let cancelled: Vec<_> = records.iter().filter(|e| e["source"] == "delayed").collect();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0]["host_bundle"], front);
        assert_eq!(cancelled[0]["payload"]["refs"].as_array().unwrap().len(), 2);
    }
}

#[test]
fn batches_do_not_mix_source_apps() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 6, &cfg, 61.0);
    let mut items = queue::load_queue(&h.rt.paths);
    for (it, host) in items.iter_mut().zip([TERMINAL, VSCODE, CLAUDE, TERMINAL, VSCODE, CLAUDE]) {
        it.ctx.host_bundle = host.into();
        it.title = host.into();
    }
    queue::save_queue(&h.rt.paths, &items).unwrap();
    *h.behaviour.lock().unwrap() = Box::new(|_, msg| {
        assert_eq!(msg.title, "AgentPulse：2 条提醒无人处理");
        assert_eq!(msg.body.lines().count(), 2);
        assert!(msg.body.lines().all(|line| line.ends_with(&msg.ctx.host_bundle)));
        Ok(None)
    });
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 3);
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn focus_clears_both_tools_in_that_app_only() {
    let h = harness();
    let mut cfg = cfg_with(&h, &[target(5, true)]);
    let mut codex_target = target(5, true); codex_target.id = "codex-w".into();
    cfg.set_tool_targets("codex", &[codex_target]);
    config::save_config(&h.rt.paths, &cfg).unwrap();
    fill(&h, 3, &cfg, 0.0);
    let mut items = queue::load_queue(&h.rt.paths);
    items[0].ctx.host_bundle = TERMINAL.into();
    items[1].ctx.host_bundle = TERMINAL.into();
    items[1].agent = "codex".into(); items[1].ctx.agent = "Codex".into();
    items[1].targets = vec!["codex-w".into()];
    items[2].ctx.host_bundle = VSCODE.into();
    queue::save_queue(&h.rt.paths, &items).unwrap();
    set_front(&h, TERMINAL); set_idle(&h, 999.0);
    *h.on_sleep.lock().unwrap() = Some(Box::new(|p| {
        let remaining = queue::load_queue(p);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].ctx.host_bundle, VSCODE);
        queue::save_queue(p, &[]).unwrap();
    }));
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sleeps.lock().unwrap().len(), 1);
    assert!(h.sent.lock().unwrap().is_empty());
}

#[test]
fn hook_activity_cancels_only_its_source_app() {
    for kind in ["prompt", "tool", "hook"] {
        let h = harness();
        let cfg = cfg_with(&h, &[target(1, true)]);
        fill(&h, 4, &cfg, 61.0);
        let mut items = queue::load_queue(&h.rt.paths);
        for (it, host) in items.iter_mut().zip([TERMINAL, TERMINAL, VSCODE, VSCODE]) {
            it.ctx.host_bundle = host.into();
            // 即使同一会话被搬到另一个 App，也只能清理产生活动的 App。
            it.ctx.session_id = Some("shared-session".into());
            it.ctx.tool_name = Some("Bash".into());
            it.title = host.into();
        }
        queue::save_queue(&h.rt.paths, &items).unwrap();
        if kind == "hook" {
            log::append_event(&h.rt.paths, &json!({"id": "later", "source": "hook", "ts": log::now(),
                "agent": "claude", "host_bundle": VSCODE, "session_id": "shared-session"})).unwrap();
        } else {
            log::record_activity(&h.rt.paths, "claude", VSCODE,
                &json!({"session_id": "shared-session", "tool_name": "Bash"}), kind).unwrap();
        }
        queue::run_queue_worker(&h.rt).unwrap();
        let sent = h.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "{kind} 只取消 VS Code；Terminal 正常发送");
        assert_eq!(sent[0].0, "AgentPulse：2 条提醒无人处理");
        assert!(sent[0].1.lines().all(|line| line.ends_with(TERMINAL)));
        let records = log::read_events(&h.rt.paths, 20);
        let cancelled: Vec<_> = records.iter().filter(|e| e["source"] == "delayed" && e["results"][0]["skipped"] == true).collect();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0]["host_bundle"], VSCODE);
    }
}

#[test]
fn unrelated_or_unknown_activity_does_not_cancel() {
    for (item_host, activity_agent, activity_host) in [(TERMINAL, "claude", ""), ("", "claude", ""), (TERMINAL, "codex", TERMINAL)] {
        let h = harness();
        let cfg = cfg_with(&h, &[target(1, true)]);
        fill(&h, 1, &cfg, 61.0);
        let mut items = queue::load_queue(&h.rt.paths);
        items[0].ctx.host_bundle = item_host.into();
        items[0].ctx.session_id = Some("shared-session".into());
        items[0].ctx.tool_name = Some("Bash".into());
        queue::save_queue(&h.rt.paths, &items).unwrap();
        for kind in ["prompt", "tool"] {
            log::record_activity(&h.rt.paths, activity_agent, activity_host,
                &json!({"session_id": "shared-session", "tool_name": "Bash"}), kind).unwrap();
        }
        log::append_event(&h.rt.paths, &json!({"id": "later", "source": "hook", "ts": log::now(),
            "agent": activity_agent, "host_bundle": activity_host, "session_id": "shared-session"})).unwrap();
        queue::run_queue_worker(&h.rt).unwrap();
        assert_eq!(h.sent.lock().unwrap().len(), 1);
        assert!(queue::load_queue(&h.rt.paths).is_empty());
    }
}

#[test]
fn attention_state_resets_for_one_app_while_another_stays_queued() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(5, true)]);
    fill(&h, 2, &cfg, 0.0);
    let mut items = queue::load_queue(&h.rt.paths);
    items[0].ctx.host_bundle = TERMINAL.into();
    items[1].ctx.host_bundle = VSCODE.into();
    let first = items[0].clone();
    queue::save_queue(&h.rt.paths, &items).unwrap();
    set_front(&h, TERMINAL); set_idle(&h, 999.0);
    let cycles: Arc<Mutex<u32>> = Arc::default();
    let observed = cycles.clone();
    *h.on_sleep.lock().unwrap() = Some(Box::new(move |p| {
        let mut n = observed.lock().unwrap(); *n += 1;
        let mut remaining = queue::load_queue(p);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].ctx.host_bundle, VSCODE);
        if *n == 1 {
            // Terminal 新提醒产生时在后台，检查前再次切回；旧 front=true 状态不能残留。
            let mut fresh = first.clone();
            fresh.r#ref = "terminal-new".into(); fresh.created = log::now(); fresh.front_at_event = false;
            remaining.push(fresh);
            queue::save_queue(p, &remaining).unwrap();
        } else {
            queue::save_queue(p, &[]).unwrap();
        }
    }));
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(*cycles.lock().unwrap(), 2);
    assert!(h.sent.lock().unwrap().is_empty());
    let records = log::read_events(&h.rt.paths, 20);
    assert_eq!(records.iter().filter(|e| e["source"] == "delayed" && e["host_bundle"] == TERMINAL).count(), 2);
}

#[test]
fn slacking_in_other_app_still_sends() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 3, &cfg, 61.0);
    set_front(&h, CHROME); set_idle(&h, 1.0);
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 1);
}

#[test]
fn host_front_but_nobody_or_locked_still_sends() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 2, &cfg, 61.0);
    let mut items = queue::load_queue(&h.rt.paths);
    for it in &mut items { it.front_at_event = true; }
    queue::save_queue(&h.rt.paths, &items).unwrap();
    set_front(&h, CLAUDE); set_idle(&h, 900.0);
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 1);
}

#[test]
fn waits_until_due() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 3, &cfg, 10.0);
    *h.on_sleep.lock().unwrap() = Some(Box::new(|p| {
        let mut items = queue::load_queue(p);
        for it in &mut items { it.created -= 60.0; }
        queue::save_queue(p, &items).unwrap();
    }));
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sleeps.lock().unwrap().len(), 1);
    assert_eq!(h.sent.lock().unwrap().len(), 1);
}

#[test]
fn second_worker_exits_when_lock_held() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 2, &cfg, 61.0);
    let held = agentpulse_lib::core::paths::FileLock::acquire(&h.rt.paths.worker_lock()).unwrap();
    queue::run_queue_worker(&h.rt).unwrap();
    drop(held);
    assert!(h.sent.lock().unwrap().is_empty());
    assert_eq!(queue::load_queue(&h.rt.paths).len(), 2);
}

// ---------- robustness ----------

#[test]
fn queue_stores_only_target_ids() {
    let h = harness();
    let mut t = target(1, true); t.set("secret", json!("top-secret"));
    let cfg = cfg_with(&h, &[t]);
    fill(&h, 1, &cfg, 61.0);
    assert_eq!(queue::load_queue(&h.rt.paths)[0].targets, vec!["w"]);
    assert!(!fs::read_to_string(h.rt.paths.queue()).unwrap().contains("top-secret"));
}

#[test]
fn disabled_after_enqueue_is_not_sent() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 2, &cfg, 61.0);
    let mut off = target(1, true); off.enabled = false;
    cfg_with(&h, &[off]);
    queue::run_queue_worker(&h.rt).unwrap();
    assert!(h.sent.lock().unwrap().is_empty());
    assert!(queue::load_queue(&h.rt.paths).is_empty());
    assert!(last_info(&h).contains("已删除或停用"));
}

#[test]
fn unsubscribed_events_are_filtered_per_channel_before_merging() {
    for batch in [true, false] {
        let h = harness();
        let mut other = target(1, batch); other.id = "z".into();
        let cfg = cfg_with(&h, &[target(1, batch), other.clone()]);
        fill(&h, 4, &cfg, 61.0);
        let items = queue::load_queue(&h.rt.paths);
        let mut edited = target(1, batch); edited.events = vec!["task_complete".into()];
        cfg_with(&h, &[edited, other]);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let recorded = sent.clone();
        *h.behaviour.lock().unwrap() = Box::new(move |t, m| {
            recorded.lock().unwrap().push((t.id.clone(), m.event.clone(), m.body.clone()));
            Ok(None)
        });
        queue::run_queue_worker(&h.rt).unwrap();
        let calls = sent.lock().unwrap();
        let filtered: Vec<_> = calls.iter().filter(|(id, _, _)| id == "w").collect();
        assert_eq!(filtered.len(), if batch { 1 } else { 2 });
        assert!(filtered.iter().all(|(_, event, body)| event == "task_complete" && !body.contains('🔐')));
        assert_eq!(calls.iter().filter(|(id, _, _)| id == "z").count(), if batch { 1 } else { 4 });
        let cancelled = log::read_events(&h.rt.paths, 20).into_iter()
            .find(|e| e["results"][0]["info"] == "未发送：已取消事件订阅").unwrap();
        assert_eq!(cancelled["results"][0]["target_id"], "w");
        assert_eq!(cancelled["results"][0]["skipped"], true);
        assert_eq!(cancelled["payload"]["refs"], json!([items[0].r#ref, items[2].r#ref]));
        assert!(queue::load_queue(&h.rt.paths).is_empty());
    }
}

#[test]
fn unsubscribed_event_is_cancelled_before_its_due_time() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(5, true)]);
    fill(&h, 1, &cfg, 0.0);
    let mut edited = target(5, true); edited.events.clear();
    cfg_with(&h, &[edited]);
    queue::run_queue_worker(&h.rt).unwrap();
    assert!(h.sent.lock().unwrap().is_empty());
    assert!(h.sleeps.lock().unwrap().is_empty());
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn unsubscribing_while_waiting_for_retry_prevents_another_attempt() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 1, &cfg, 61.0);
    let calls = Arc::new(Mutex::new(0));
    let recorded = calls.clone();
    *h.behaviour.lock().unwrap() = Box::new(move |_, _| {
        *recorded.lock().unwrap() += 1;
        Err("网络错误".into())
    });
    *h.on_sleep.lock().unwrap() = Some(Box::new(|p| {
        assert_eq!(queue::load_queue(p)[0].attempts["w"], 1);
        let mut cfg = config::load_config(p).unwrap();
        let mut edited = target(1, true); edited.events.clear();
        cfg.set_tool_targets("claude", &[edited]);
        config::save_config(p, &cfg).unwrap();
    }));
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(h.sleeps.lock().unwrap().len(), 1);
    assert!(last_info(&h).contains("已取消事件订阅"));
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn config_changes_during_send_apply_to_the_next_channel() {
    for action in ["unsubscribe", "disable", "delete"] {
        let h = harness();
        let mut other = target(1, true); other.id = "z".into();
        let cfg = cfg_with(&h, &[target(1, true), other.clone()]);
        fill(&h, 2, &cfg, 61.0);
        let paths = h.rt.paths.clone();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let recorded = sent.clone();
        *h.behaviour.lock().unwrap() = Box::new(move |t, m| {
            recorded.lock().unwrap().push((t.id.clone(), m.event.clone()));
            if t.id == "w" {
                let mut cfg = config::load_config(&paths).unwrap();
                let mut targets = cfg.tool_targets("claude");
                match action {
                    "unsubscribe" => targets[1].events = vec!["task_complete".into()],
                    "disable" => targets[1].enabled = false,
                    _ => { targets.pop(); }
                }
                cfg.set_tool_targets("claude", &targets);
                config::save_config(&paths, &cfg).unwrap();
            }
            Ok(None)
        });
        queue::run_queue_worker(&h.rt).unwrap();
        let calls = sent.lock().unwrap();
        assert_eq!(calls.len(), if action == "unsubscribe" { 2 } else { 1 });
        if action == "unsubscribe" { assert_eq!(calls[1], ("z".into(), "task_complete".into())); }
        assert!(queue::load_queue(&h.rt.paths).is_empty());
    }
}

#[test]
fn invalid_config_during_send_preserves_unsent_items_and_completed_progress() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, false)]);
    fill(&h, 2, &cfg, 61.0);
    let paths = h.rt.paths.clone();
    *h.behaviour.lock().unwrap() = Box::new(move |_, _| {
        fs::write(paths.config(), "broken").unwrap();
        Ok(None)
    });
    assert!(queue::run_queue_worker(&h.rt).is_err());
    assert_eq!(h.sent.lock().unwrap().len(), 1);
    let remaining = queue::load_queue(&h.rt.paths);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].event, "task_complete");
    assert!(remaining[0].attempts.is_empty());
}

#[test]
fn unsupported_channel_in_old_config_is_not_sent() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 1, &cfg, 61.0);
    let mut raw = serde_json::to_value(&cfg).unwrap();
    raw["tools"]["claude"]["targets"][0]["type"] = json!("retired");
    std::fs::write(h.rt.paths.config(), serde_json::to_vec(&raw).unwrap()).unwrap();
    queue::run_queue_worker(&h.rt).unwrap();
    assert!(h.sent.lock().unwrap().is_empty());
    assert!(queue::load_queue(&h.rt.paths).is_empty());
    assert!(last_info(&h).contains("已删除或停用"));
}

#[test]
fn send_happens_outside_queue_lock() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 1, &cfg, 61.0);
    let lock_path = h.rt.paths.queue_lock();
    let free: Arc<Mutex<Vec<bool>>> = Arc::default();
    let f2 = free.clone();
    *h.behaviour.lock().unwrap() = Box::new(move |_, _| {
        f2.lock().unwrap().push(agentpulse_lib::core::paths::FileLock::try_acquire(&lock_path).unwrap().is_some());
        Ok(None)
    });
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(*free.lock().unwrap(), vec![true]);
}

#[test]
fn failure_retries_then_gives_up() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 1, &cfg, 61.0);
    let calls: Arc<Mutex<u32>> = Arc::default();
    let c2 = calls.clone();
    *h.behaviour.lock().unwrap() = Box::new(move |_, _| { *c2.lock().unwrap() += 1; Err("RuntimeError: network down".into()) });
    *h.on_sleep.lock().unwrap() = Some(Box::new(|p| { // 每次等待都让重试时间到期
        let mut items = queue::load_queue(p);
        for it in &mut items { for v in it.next_try.values_mut() { *v = 0.0; } }
        queue::save_queue(p, &items).unwrap();
    }));
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(*calls.lock().unwrap(), queue::MAX_ATTEMPTS);
    assert!(queue::load_queue(&h.rt.paths).is_empty());
    let infos: Vec<String> = log::read_events(&h.rt.paths, 10).iter().filter(|e| e["source"] == "delayed").map(|e| e["results"][0]["info"].as_str().unwrap().to_string()).collect();
    assert!(infos[0].contains("不再重试"));
    assert!(infos.last().unwrap().contains("稍后重试"));
}

#[test]
fn merged_failure_preserves_each_items_retry_budget() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 2, &cfg, 61.0);
    let mut items = queue::load_queue(&h.rt.paths);
    items[0].attempts.insert("w".into(), queue::MAX_ATTEMPTS - 1);
    let new_ref = items[1].r#ref.clone();
    queue::save_queue(&h.rt.paths, &items).unwrap();
    *h.behaviour.lock().unwrap() = Box::new(|_, _| Err("network down".into()));
    let retries: Arc<Mutex<Vec<u32>>> = Arc::default();
    let observed = retries.clone();
    *h.on_sleep.lock().unwrap() = Some(Box::new(move |p| {
        let mut remaining = queue::load_queue(p);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].r#ref, new_ref);
        observed.lock().unwrap().push(remaining[0].attempts["w"]);
        remaining[0].next_try.clear();
        queue::save_queue(p, &remaining).unwrap();
    }));
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(*retries.lock().unwrap(), vec![1, 2]);
    assert!(queue::load_queue(&h.rt.paths).is_empty());
    let records = log::read_events(&h.rt.paths, 10);
    let delayed: Vec<_> = records.iter().filter(|e| e["source"] == "delayed").collect();
    assert_eq!(delayed.len(), queue::MAX_ATTEMPTS as usize);
    let first = delayed.last().unwrap()["results"][0]["info"].as_str().unwrap();
    assert!(first.contains("1 条稍后重试，1 条不再重试"), "{first}");
}

#[test]
fn failure_then_success() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 1, &cfg, 61.0);
    let n: Arc<Mutex<u32>> = Arc::default();
    let n2 = n.clone();
    *h.behaviour.lock().unwrap() = Box::new(move |_, _| { *n2.lock().unwrap() += 1; if *n2.lock().unwrap() == 1 { Err("once".into()) } else { Ok(None) } });
    *h.on_sleep.lock().unwrap() = Some(Box::new(|p| {
        let mut items = queue::load_queue(p);
        for it in &mut items { it.next_try.clear(); }
        queue::save_queue(p, &items).unwrap();
    }));
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(*n.lock().unwrap(), 2);
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn error_in_one_agent_drops_only_that_agent() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 1, &cfg, 61.0);
    let mut items = queue::load_queue(&h.rt.paths);
    let mut codex = items[0].clone();
    codex.r#ref = "codex1".into(); codex.agent = "codex".into(); codex.ctx.agent = "Codex".into();
    codex.ctx.host_bundle = "com.openai.codex".into();
    items.push(codex);
    queue::save_queue(&h.rt.paths, &items).unwrap();
    // codex 那条的发送抛 panic，模拟处理出错
    *h.behaviour.lock().unwrap() = Box::new(|_, m| if m.agent == "codex" { panic!("broken item") } else { Ok(None) });
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 1);
    assert!(queue::load_queue(&h.rt.paths).is_empty());
    assert!(log::read_events(&h.rt.paths, 10).iter().any(|e| e["results"][0]["info"].as_str().unwrap_or("").contains("处理出错")));
}

#[test]
fn malformed_items_are_ignored() {
    let h = harness();
    fs::create_dir_all(&h.rt.paths.data_dir).unwrap();
    fs::write(h.rt.paths.queue(), r#"[{"ref": "x"}, "junk"]"#).unwrap();
    assert!(queue::load_queue(&h.rt.paths).is_empty());
    fs::write(h.rt.paths.queue(), "{bad").unwrap();
    assert!(queue::load_queue(&h.rt.paths).is_empty());
}

#[test]
fn old_queue_format_with_full_targets_is_accepted() {
    let h = harness();
    fs::create_dir_all(&h.rt.paths.data_dir).unwrap();
    fs::write(h.rt.paths.queue(), json!([{"ref": "r", "created": 1.0, "agent": "claude", "event": "task_complete", "title": "t", "body": "b",
        "ctx": {"agent": "Claude Code", "project": "p"}, "targets": [{"id": "w", "type": "wxtest"}]}]).to_string()).unwrap();
    let q = queue::load_queue(&h.rt.paths);
    assert_eq!(q[0].targets, vec!["w"]);
    assert_eq!(q[0].ctx.project, "p");
}

#[test]
fn attention_state_reset_when_agent_leaves_queue() {
    // claude 的队列被取消后又来新提醒，新提醒拿到的是全新的注意力状态
    let h = harness();
    let cfg = cfg_with(&h, &[target(1, true)]);
    fill(&h, 1, &cfg, 61.0);
    let items = queue::load_queue(&h.rt.paths);
    let mut keepalive = items[0].clone(); // 让 worker 不退出
    keepalive.r#ref = "codex-long".into(); keepalive.agent = "codex".into(); keepalive.created = log::now() + 3600.0;
    queue::save_queue(&h.rt.paths, &[items[0].clone(), keepalive]).unwrap();
    log::record_activity(&h.rt.paths, "claude", CLAUDE, &json!({}), "prompt").unwrap(); // 第一轮：claude 整队取消
    let first_item = items[0].clone();
    let cycles: Arc<Mutex<u32>> = Arc::default();
    let c2 = cycles.clone();
    *h.on_sleep.lock().unwrap() = Some(Box::new(move |p| {
        let mut n = c2.lock().unwrap(); *n += 1;
        if *n == 1 { // 第二轮之前：清掉活动记录，claude 来了一条新提醒
            fs::remove_file(p.activity()).unwrap();
            let mut q = queue::load_queue(p);
            let mut it = first_item.clone(); it.r#ref = "claude-2".into(); it.created = log::now();
            q.push(it);
            queue::save_queue(p, &q).unwrap();
        } else if *n == 3 { queue::save_queue(p, &[]).unwrap(); }
    }));
    queue::run_queue_worker(&h.rt).unwrap();
    // 第二批 claude 条目：没有旧状态残留 → 不会被误判为“有操作”，也未到期 → 不发送
    assert!(h.sent.lock().unwrap().is_empty());
    let cancelled = log::read_events(&h.rt.paths, 20).iter().filter(|e| e["title"].as_str().unwrap_or("").contains("已取消")).count();
    assert_eq!(cancelled, 1, "只有第一批被取消");
}

// ---------- PostToolUse ----------

fn permission_item(h: &Harness, session: &str, tool: &str) {
    let cfg = cfg_with(h, &[target(1, true)]);
    let mut ctx = sample_context("claude", "permission_request");
    ctx.session_id = Some(session.into()); ctx.tool_name = Some(tool.into());
    dispatch::dispatch(&h.rt, "claude", "permission_request", &ctx, Some(&cfg), "hook", None, None).unwrap();
    let mut items = queue::load_queue(&h.rt.paths);
    for it in &mut items { it.created -= 61.0; }
    queue::save_queue(&h.rt.paths, &items).unwrap();
}

#[test]
fn tool_done_for_pending_permission_cancels() {
    let h = harness();
    permission_item(&h, "s1", "Bash");
    assert!(queue::has_pending_permission(&h.rt.paths, Some("s1")));
    log::record_activity(&h.rt.paths, "claude", CLAUDE, &json!({"session_id": "s1", "tool_name": "Bash"}), "tool").unwrap();
    queue::run_queue_worker(&h.rt).unwrap();
    assert!(h.sent.lock().unwrap().is_empty());
    assert!(last_info(&h).contains("被批准"));
}

#[test]
fn tool_done_in_other_session_or_other_tool_does_not_cancel() {
    let h = harness();
    permission_item(&h, "s1", "Bash");
    log::record_activity(&h.rt.paths, "claude", CLAUDE, &json!({"session_id": "s1", "tool_name": "Edit"}), "tool").unwrap();
    log::record_activity(&h.rt.paths, "claude", CLAUDE, &json!({"session_id": "s2", "tool_name": "Bash"}), "tool").unwrap();
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 1);
}

#[test]
fn no_pending_permission_means_no_record_needed() {
    let h = harness();
    assert!(!queue::has_pending_permission(&h.rt.paths, Some("s9")));
    assert!(!queue::has_pending_permission(&h.rt.paths, None));
}

#[test]
fn tool_done_does_not_count_as_new_prompt() {
    let h = harness();
    permission_item(&h, "s1", "Bash");
    log::record_activity(&h.rt.paths, "claude", CLAUDE, &json!({"session_id": "other", "tool_name": "Read"}), "tool").unwrap();
    assert!(!attention::new_prompt(&h.rt.paths, &queue::load_queue(&h.rt.paths)[0], 0.0));
    queue::run_queue_worker(&h.rt).unwrap();
    assert_eq!(h.sent.lock().unwrap().len(), 1);
}

#[test]
fn context_roundtrips_through_queue_file() {
    let h = harness();
    let cfg = cfg_with(&h, &[target(5, true)]);
    let mut ctx: Context = sample_context("claude", "task_complete");
    ctx.session_id = Some("sid".into());
    dispatch::dispatch(&h.rt, "claude", "task_complete", &ctx, Some(&cfg), "hook", None, None).unwrap();
    let q = queue::load_queue(&h.rt.paths);
    assert_eq!(q[0].ctx, ctx);
    assert_eq!(queue::queued_per_agent(&h.rt.paths)["claude"], 1);
}
