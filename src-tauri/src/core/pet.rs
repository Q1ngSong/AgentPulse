//! 桌宠的活动状态与待处理提醒。按到达顺序返回第一条，只清理指定来源。
use std::{collections::{HashMap, HashSet}, time::{Duration, Instant}};
use serde::{Deserialize, Serialize};
use super::{config::{Target, EVENT_KEYS, TOOLS}, pet_animation::{self, Animations}};

/// 一个工具在某个来源 App 中的消息；host 留空表示该工具的全部来源。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct PetSource {
    pub agent: String,
    #[serde(default)]
    pub host: String,
}

impl PetSource {
    pub fn matches(&self, agent: &str, host: &str) -> bool {
        self.agent == agent && (self.host.is_empty() || self.host == host)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PetConfig {
    pub id: String,
    pub number: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default = "pet_animation::defaults")]
    pub animations: Animations,
    #[serde(default)]
    pub sources: Vec<PetSource>,
}

fn default_enabled() -> bool { true }

impl PetConfig {
    /// 同一来源即使同时匹配通配项和精确项，也只给这个桌宠送一次。
    pub fn matches(&self, agent: &str, host: &str) -> bool {
        self.sources.iter().any(|source| source.matches(agent, host))
    }

    /// 桌宠独立配置，在发送边界复用统一渠道；不写入工具的提醒卡片列表。
    pub fn target(&self) -> Target {
        Target {
            id: self.id.clone(), kind: "pet".into(),
            name: if self.name.trim().is_empty() { format!("桌宠 {}", self.number) } else { self.name.clone() },
            enabled: self.enabled, events: self.events.clone(),
            extra: serde_json::Map::from_iter([("animations".into(), serde_json::json!(self.animations))]),
        }
    }
}

/// 编号和窗口标识均独立且稳定；来源重复或无法安全用于匹配时拒绝整个配置。
pub fn validate_configs(pets: &[PetConfig]) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut numbers = HashSet::new();
    for pet in pets {
        if !(1..=64).contains(&pet.id.len()) || !pet.id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || !ids.insert(&pet.id) {
            return Err("桌宠 id 需为 1–64 位字母、数字、下划线或连字符，且不能重复".into());
        }
        if pet.number == 0 || !numbers.insert(pet.number) {
            return Err("桌宠编号需为正整数，且不能重复".into());
        }
        let mut events = HashSet::new();
        if pet.events.iter().any(|event| !EVENT_KEYS.contains(&event.as_str()) || !events.insert(event)) {
            return Err(format!("桌宠 {} 的提醒事件不受支持或重复", pet.number));
        }
        let mut sources = HashSet::new();
        for source in &pet.sources {
            if !TOOLS.contains(&source.agent.as_str()) {
                return Err(format!("桌宠 {} 的来源工具不受支持", pet.number));
            }
            if source.host.len() > 255 || !source.host.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-')) {
                return Err(format!("桌宠 {} 的来源 App 标识无效", pet.number));
            }
            if !sources.insert(source) {
                return Err(format!("桌宠 {} 的消息来源重复", pet.number));
            }
        }
        pet_animation::validate(&pet.animations).map_err(|e| format!("桌宠 {}：{e}", pet.number))?;
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase { Working, Sleeping, PermissionRequest, TaskComplete }

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Update {
    pub id: String,
    pub phase: Phase,
    pub agent: String,
    pub host: String,
    pub session: String,
    #[serde(default)]
    pub hook_event: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_input_key: String,
    #[serde(default)]
    pub observed_at: f64,
    pub title: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animations: Option<super::pet_animation::Animations>,
}

impl Update {
    fn same_session(&self, other: &Self) -> bool {
        self.agent == other.agent && self.host == other.host && self.session == other.session
    }

    fn correlated(&self, other: &Self) -> bool {
        !self.session.is_empty() && self.same_session(other)
    }

    fn no_newer_than(&self, other: &Self) -> bool {
        self.observed_at <= 0.0 || other.observed_at <= 0.0 || self.observed_at <= other.observed_at
    }

    fn resolved_by(&self, completed: &Self) -> bool {
        if !self.tool_use_id.is_empty() && !completed.tool_use_id.is_empty() {
            return self.tool_use_id == completed.tool_use_id;
        }
        !self.tool_input_key.is_empty() && self.tool_input_key == completed.tool_input_key
            && self.observed_at > 0.0 && completed.observed_at > 0.0 && self.observed_at <= completed.observed_at
    }
}

/// 已完成调用保留一个小的去重窗口，防止异步权限 Hook 晚到后重新显示。
struct Resolution {
    update: Update,
    entire_session: bool,
}

#[derive(Default)]
pub struct State {
    pub pending: Vec<Update>,
    working: Vec<Update>,
    visible_after: HashMap<String, Instant>,
    resolved: Vec<Resolution>,
}

impl State {
    pub fn receive(&mut self, update: Update) {
        self.receive_at(update, Instant::now());
    }

    pub fn receive_at(&mut self, update: Update, now: Instant) {
        if update.phase == Phase::Sleeping && update.hook_event == "PermissionRequest" { return; }
        if self.pending.iter().any(|n| n.id == update.id) { return; }
        if update.phase == Phase::PermissionRequest && self.resolved.iter().any(|r| {
            r.update.correlated(&update) && if r.entire_session {
                update.observed_at > 0.0 && r.update.observed_at > 0.0 && update.observed_at <= r.update.observed_at
            } else {
                update.resolved_by(&r.update)
            }
        }) { return; }

        let entire_session = matches!(update.hook_event.as_str(), "UserPromptSubmit" | "Stop") || update.phase == Phase::TaskComplete;
        let completed_call = matches!(update.hook_event.as_str(), "PostToolUse" | super::codex_permission::RESOLVED) && (!update.tool_use_id.is_empty()
            || (!update.tool_input_key.is_empty() && update.observed_at > 0.0));
        if entire_session || completed_call {
            self.pending.retain(|n| {
                n.phase != Phase::PermissionRequest || !n.correlated(&update)
                    || if entire_session { !n.no_newer_than(&update) }
                    else { !n.resolved_by(&update) }
            });
            if !update.session.is_empty() {
                self.remember_resolution(update.clone(), entire_session);
            }
        }

        // 上一轮的工具 Hook 迟到时，保留已经收到的新任务或结束状态。
        if update.phase == Phase::Working && update.hook_event != "UserPromptSubmit" && update.observed_at > 0.0
            && self.resolved.iter().any(|r| r.entire_session && r.update.correlated(&update)
                && r.update.observed_at > 0.0 && update.observed_at <= r.update.observed_at) {
            self.prune_deadlines();
            return;
        }

        let delayed = update.agent == "codex" && update.phase == Phase::PermissionRequest && update.hook_event == "PermissionRequest";
        if !delayed { self.working.retain(|n| !n.same_session(&update) || !n.no_newer_than(&update)); }
        match update.phase {
            Phase::Working => {
                if !update.host.is_empty() && !update.session.is_empty() {
                    // 新活动可收起已完成提醒；只有旧版无事件名的更新沿用旧权限清理规则。
                    self.pending.retain(|n| !n.same_session(&update) || !n.no_newer_than(&update)
                        || (n.phase == Phase::PermissionRequest && !update.hook_event.is_empty()));
                }
                self.working.push(update);
            }
            Phase::Sleeping => {}
            Phase::PermissionRequest | Phase::TaskComplete => {
                if delayed { self.visible_after.insert(update.id.clone(), now + Duration::from_secs(2)); }
                self.pending.push(update);
            }
        }
        self.prune_deadlines();
    }

    fn remember_resolution(&mut self, update: Update, entire_session: bool) {
        if let Some(existing) = self.resolved.iter_mut().find(|r| r.entire_session == entire_session
            && r.update.correlated(&update) && (entire_session || (r.update.tool_use_id == update.tool_use_id
                && r.update.tool_input_key == update.tool_input_key))) {
            if update.observed_at > existing.update.observed_at { existing.update = update; }
            return;
        }
        self.resolved.push(Resolution { update, entire_session });
        if self.resolved.len() > 128 { self.resolved.remove(0); }
    }

    fn prune_deadlines(&mut self) {
        self.visible_after.retain(|id, _| self.pending.iter().any(|n| n.id == *id));
    }

    pub fn retain_sources(&mut self, sources: &[PetSource]) {
        self.pending.retain(|n| sources.iter().any(|source| source.matches(&n.agent, &n.host)));
        self.working.retain(|n| sources.iter().any(|source| source.matches(&n.agent, &n.host)));
        self.resolved.retain(|r| sources.iter().any(|source| source.matches(&r.update.agent, &r.update.host)));
        self.prune_deadlines();
    }

    pub fn refresh_animations(&mut self, animations: &Animations) {
        for n in self.pending.iter_mut().chain(self.working.iter_mut()) {
            n.animations = Some(animations.clone());
        }
    }
    pub fn current(&self) -> Option<&Update> {
        self.current_at(Instant::now())
    }

    pub fn current_at(&self, now: Instant) -> Option<&Update> {
        self.pending.iter().find(|n| self.visible_at(n, now))
            .or_else(|| self.working.iter().rev().find(|work| {
                // 提示收起不代表已批准；没有后续活动时，不把请求前的工作状态重新当成当前状态。
                !self.pending.iter().any(|n| self.visible_after.get(&n.id)
                    .map(|deadline| now >= *deadline + Duration::from_secs(5)).unwrap_or(false)
                    && n.correlated(work) && work.no_newer_than(n))
            }))
    }

    fn visible_at(&self, update: &Update, now: Instant) -> bool {
        self.visible_after.get(&update.id)
            .map(|deadline| now >= *deadline && now < *deadline + Duration::from_secs(5)).unwrap_or(true)
    }

    pub fn visible_pending_len(&self) -> usize {
        self.visible_pending_len_at(Instant::now())
    }

    pub fn visible_pending_len_at(&self, now: Instant) -> usize {
        self.pending.iter().filter(|n| self.visible_at(n, now)).count()
    }

    pub fn source_ids(&self, host: &str) -> Vec<String> {
        if host.is_empty() { return Vec::new(); }
        self.pending.iter().filter(|n| n.host == host).map(|n| n.id.clone()).collect()
    }

    /// 用点击当时的 ID 快照清理，跳转期间到达的新提醒继续排队。
    pub fn dismiss(&mut self, ids: &[String]) {
        self.working.retain(|work| !self.pending.iter().any(|n| ids.contains(&n.id)
            && n.phase == Phase::PermissionRequest && n.correlated(work) && work.no_newer_than(n)));
        self.pending.retain(|n| !ids.contains(&n.id));
        self.prune_deadlines();
    }
}

/// 活动同步不拉起 App、不阻塞正常提醒；失败由下一个 Hook 更新恢复。
pub fn publish(paths: &super::paths::Paths, update: &Update) {
    super::channels::desktop::send_activity(paths, &serde_json::json!({"kind":"pet_update","update":update}));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> PetConfig {
        serde_json::from_value(serde_json::json!({"id":"pet-1","number":1,"events":["task_complete"],
            "sources":[{"agent":"claude","host":"com.apple.Terminal"}]})).unwrap()
    }

    #[test]
    fn pet_config_rejects_ambiguous_ids_numbers_sources_and_invalid_values() {
        let valid = config();
        assert!(validate_configs(&[valid.clone()]).is_ok());
        for id in ["", "../pet", "has space", "宠物", &"x".repeat(65)] {
            let mut bad = valid.clone(); bad.id = id.into();
            assert!(validate_configs(&[bad]).is_err(), "id {id}");
        }
        let mut other = valid.clone(); other.number = 2;
        assert!(validate_configs(&[valid.clone(), other]).is_err(), "重复 id");
        let mut other = valid.clone(); other.id = "pet-2".into();
        assert!(validate_configs(&[valid.clone(), other]).is_err(), "重复编号");
        let mut bad = valid.clone(); bad.number = 0;
        assert!(validate_configs(&[bad]).is_err());
        for events in [vec!["unsupported".into()], vec!["task_complete".into(), "task_complete".into()]] {
            let mut bad = valid.clone(); bad.events = events;
            assert!(validate_configs(&[bad]).is_err());
        }
        let mut bad = valid.clone(); bad.sources[0].agent = "other-agent".into();
        assert!(validate_configs(&[bad]).is_err());
        for host in ["has space", "some/app", "line\nbreak", &"x".repeat(256)] {
            let mut bad = valid.clone(); bad.sources[0].host = host.into();
            assert!(validate_configs(&[bad]).is_err(), "host {host}");
        }
        let mut bad = valid.clone(); bad.sources.push(bad.sources[0].clone());
        assert!(validate_configs(&[bad]).is_err(), "重复来源");
        let mut bad = valid.clone(); bad.animations.get_mut("working").unwrap().enter = [0, 5];
        assert!(validate_configs(&[bad]).is_err(), "无效动画");
    }

    #[test]
    fn sources_match_tool_and_app_without_requiring_selection_for_every_app() {
        let mut pet = config();
        assert!(pet.matches("claude", "com.apple.Terminal"));
        assert!(!pet.matches("codex", "com.apple.Terminal"));
        assert!(!pet.matches("claude", "com.microsoft.VSCode"));
        assert!(!pet.matches("claude", ""));
        pet.sources.push(PetSource { agent: "codex".into(), host: String::new() });
        assert!(pet.matches("codex", "com.microsoft.VSCode"));
        assert!(pet.matches("codex", ""));
        pet.sources.clear();
        assert!(!pet.matches("claude", "com.apple.Terminal"));
        assert!(!pet.matches("codex", ""));
    }

    fn notice(id: &str, host: &str, phase: Phase) -> Update {
        Update { id: id.into(), host: host.into(), phase, agent: "claude".into(), session: id.into(), hook_event: String::new(), tool_use_id: String::new(), tool_input_key: String::new(), observed_at: 0.0, title: id.into(), detail: String::new(), animations: None }
    }
    #[test]
    fn first_message_controls_pose_and_click_even_when_later_is_urgent() {
        let mut state = State::default();
        state.receive(notice("first", "terminal", Phase::TaskComplete));
        state.receive(notice("second", "vscode", Phase::PermissionRequest));
        assert_eq!(state.current().unwrap().id, "first");
        assert_eq!(state.current().unwrap().phase, Phase::TaskComplete);
    }
    #[test]
    fn source_snapshot_keeps_other_apps_and_new_arrivals() {
        let mut state = State::default();
        for (id, host) in [("1", "a"), ("2", "b"), ("3", "a"), ("4", "")] {
            state.receive(notice(id, host, Phase::TaskComplete));
        }
        let ids = state.source_ids("a");
        state.receive(notice("5", "a", Phase::PermissionRequest));
        state.dismiss(&ids);
        assert_eq!(state.pending.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["2", "4", "5"]);
        assert!(state.source_ids("").is_empty());
    }
    #[test]
    fn unknown_source_activity_does_not_erase_a_pending_notice() {
        let mut state = State::default();
        state.receive(notice("first", "", Phase::TaskComplete));
        let mut work = notice("work", "", Phase::Working);
        work.session = "first".into();
        state.receive(work);
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.current().unwrap().id, "first");
    }
    #[test]
    fn working_and_finished_sessions_do_not_hide_other_pending_messages() {
        let mut state = State::default();
        state.receive(notice("a", "terminal", Phase::PermissionRequest));
        let mut work = notice("working", "terminal", Phase::Working);
        work.session = "a".into();
        state.receive(work);
        assert!(state.pending.is_empty());
        assert_eq!(state.current().unwrap().phase, Phase::Working);
        state.receive(notice("other", "vscode", Phase::TaskComplete));
        assert_eq!(state.current().unwrap().host, "vscode");
        let mut finish = notice("done", "terminal", Phase::TaskComplete);
        finish.session = "a".into();
        state.receive(finish.clone());
        state.receive(finish);
        assert_eq!(state.pending.len(), 2);
        state.dismiss(&["other".into(), "done".into()]);
        assert!(state.current().is_none());
    }

    #[test]
    fn removing_sources_clears_only_unselected_messages_and_updates_pet_animations() {
        let mut state = State::default();
        state.receive(notice("selected", "terminal", Phase::TaskComplete));
        state.receive(notice("unselected-app", "vscode", Phase::TaskComplete));
        let mut other_agent = notice("unselected-agent", "terminal", Phase::PermissionRequest);
        other_agent.agent = "codex".into();
        state.receive(other_agent);
        state.receive(notice("working", "terminal", Phase::Working));
        state.retain_sources(&[PetSource { agent: "claude".into(), host: "terminal".into() }]);
        assert_eq!(state.pending.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["selected"]);
        assert_eq!(state.working.len(), 1);
        let mut animations = pet_animation::defaults();
        animations.get_mut("working").unwrap().frame_ms = 100;
        state.refresh_animations(&animations);
        assert!(state.pending.iter().chain(state.working.iter()).all(|n| n.animations.as_ref() == Some(&animations)));
        state.retain_sources(&[]);
        assert!(state.pending.is_empty());
        assert!(state.current().is_none());
    }

    fn codex_hook(id: &str, hook: &str, at: f64, call: &str) -> Update {
        let phase = match hook {
            "PermissionRequest" => Phase::PermissionRequest,
            "Stop" => Phase::Sleeping,
            _ => Phase::Working,
        };
        Update { agent: "codex".into(), session: "session".into(), hook_event: hook.into(),
            observed_at: at, tool_use_id: call.into(), ..notice(id, "", phase) }
    }

    #[test]
    fn confirmed_approval_is_immediate_and_resolution_only_clears_that_request() {
        let now = Instant::now();
        let mut state = State::default();
        for call in ["a", "b"] {
            let mut request = codex_hook(call, super::super::codex_permission::CONFIRMED, 10.0, call);
            request.phase = Phase::PermissionRequest;
            state.receive_at(request, now);
        }
        assert_eq!(state.visible_pending_len_at(now), 2);
        state.receive_at(codex_hook("resolved", super::super::codex_permission::RESOLVED, 11.0, "a"), now);
        assert_eq!(state.pending.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn codex_permission_waits_two_seconds_then_shows_for_five_without_assuming_approval() {
        let now = Instant::now();
        let mut state = State::default();
        state.receive_at(codex_hook("work", "PreToolUse", 10.0, "a"), now);
        let request = codex_hook("permission", "PermissionRequest", 11.0, "a");
        let mut sleep = request.clone(); sleep.id = "sleep".into(); sleep.phase = Phase::Sleeping;
        state.receive_at(sleep, now);
        state.receive_at(request.clone(), now);
        state.receive_at(request, now + Duration::from_secs(1));
        assert_eq!(state.current_at(now + Duration::from_millis(1999)).unwrap().id, "work");
        assert_eq!(state.visible_pending_len_at(now + Duration::from_millis(1999)), 0);
        assert_eq!(state.current_at(now + Duration::from_secs(2)).unwrap().id, "permission");
        assert_eq!(state.visible_pending_len_at(now + Duration::from_secs(2)), 1);
        assert_eq!(state.current_at(now + Duration::from_millis(6999)).unwrap().phase, Phase::PermissionRequest);
        assert_eq!(state.visible_pending_len_at(now + Duration::from_secs(7)), 0);
        assert!(state.current_at(now + Duration::from_secs(7)).is_none());
        assert_eq!(state.pending.len(), 1, "收起提示不代表请求已获批准");
        state.receive_at(codex_hook("done", "PostToolUse", 12.0, "a"), now + Duration::from_secs(10));
        assert_eq!(state.current_at(now + Duration::from_secs(10)).unwrap().phase, Phase::Working);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn fast_automatic_approval_never_becomes_visible_with_unknown_host() {
        let now = Instant::now();
        let mut state = State::default();
        let mut request = codex_hook("permission", "PermissionRequest", 10.0, "");
        request.tool_input_key = "turn-1-exec-command-a".into();
        state.receive_at(request.clone(), now);
        let mut completed = codex_hook("completed", "PostToolUse", 10.2, "");
        completed.tool_input_key = request.tool_input_key;
        state.receive_at(completed, now + Duration::from_millis(200));
        assert!(state.pending.is_empty());
        assert_eq!(state.visible_pending_len_at(now + Duration::from_secs(3)), 0);
        assert_eq!(state.current_at(now + Duration::from_secs(3)).unwrap().phase, Phase::Working);
    }

    #[test]
    fn hiding_codex_permission_keeps_other_agents_and_completion_messages_visible() {
        let now = Instant::now();
        let mut state = State::default();
        state.receive_at(codex_hook("codex", "PermissionRequest", 10.0, "a"), now);
        let mut claude = notice("claude", "terminal", Phase::PermissionRequest);
        claude.hook_event = "PermissionRequest".into();
        state.receive_at(claude, now);
        state.receive_at(notice("finished", "vscode", Phase::TaskComplete), now);
        assert_eq!(state.current_at(now + Duration::from_secs(60)).unwrap().id, "claude");
        assert_eq!(state.visible_pending_len_at(now + Duration::from_secs(60)), 2);
        state.dismiss(&["claude".into()]);
        assert_eq!(state.current_at(now + Duration::from_secs(60)).unwrap().id, "finished");
    }

    #[test]
    fn completed_call_rejects_late_permission_even_if_hook_capture_time_is_reversed() {
        let now = Instant::now();
        let mut state = State::default();
        state.receive_at(codex_hook("completed", "PostToolUse", 10.0, "call-a"), now);
        state.receive_at(codex_hook("late", "PermissionRequest", 11.0, "call-a"), now);
        assert!(state.pending.is_empty());
        assert_eq!(state.current_at(now + Duration::from_secs(3)).unwrap().id, "completed");
    }

    #[test]
    fn input_fallback_rejects_late_delivery_but_preserves_new_identical_requests() {
        let now = Instant::now();
        let mut state = State::default();
        let mut completed = codex_hook("completed", "PostToolUse", 20.0, "");
        completed.tool_input_key = "same-turn-and-command".into();
        state.receive_at(completed.clone(), now);
        let mut older = codex_hook("older", "PermissionRequest", 10.0, "");
        older.tool_input_key = completed.tool_input_key.clone();
        state.receive_at(older, now);
        assert!(state.pending.is_empty());
        let mut newer = codex_hook("newer", "PermissionRequest", 30.0, "");
        newer.tool_input_key = completed.tool_input_key;
        state.receive_at(newer, now);
        state.receive_at(codex_hook("pre", "PreToolUse", 31.0, ""), now);
        assert_eq!(state.current_at(now + Duration::from_secs(2)).unwrap().id, "newer");
        let mut next_turn = codex_hook("next-turn", "PostToolUse", 32.0, "");
        next_turn.tool_input_key = "next-turn-same-command".into();
        state.receive_at(next_turn, now);
        assert_eq!(state.pending.len(), 1);
    }

    #[test]
    fn parallel_tool_activity_only_resolves_the_matching_call() {
        let now = Instant::now();
        let mut state = State::default();
        for call in ["a", "b"] {
            let mut request = codex_hook(call, "PermissionRequest", 10.0, call);
            request.host = "terminal".into();
            request.tool_input_key = "same-input".into();
            state.receive_at(request, now);
        }
        let mut pre = codex_hook("pre", "PreToolUse", 11.0, "b"); pre.host = "terminal".into();
        state.receive_at(pre, now);
        assert_eq!(state.pending.len(), 2, "工具开始不等于权限批准");
        let mut completed = codex_hook("completed", "PostToolUse", 12.0, "b");
        completed.host = "terminal".into(); completed.tool_input_key = "same-input".into();
        state.receive_at(completed, now);
        assert_eq!(state.pending.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn permission_resolution_never_crosses_agent_app_or_session() {
        for difference in ["agent", "host", "session"] {
            let now = Instant::now();
            let mut state = State::default();
            state.receive_at(codex_hook("waiting", "PermissionRequest", 10.0, "same-call"), now);
            let mut other = codex_hook("other", "PostToolUse", 11.0, "same-call");
            match difference {
                "agent" => other.agent = "claude".into(),
                "host" => other.host = "terminal".into(),
                _ => other.session = "other-session".into(),
            }
            state.receive_at(other, now);
            assert_eq!(state.pending.len(), 1, "{difference}");
        }
    }

    #[test]
    fn stop_and_new_prompt_clear_old_permissions_without_erasing_later_requests() {
        for hook in ["Stop", "UserPromptSubmit"] {
            let now = Instant::now();
            let mut state = State::default();
            state.receive_at(codex_hook("old", "PermissionRequest", 10.0, "a"), now);
            state.receive_at(codex_hook("newer", "PermissionRequest", 30.0, "b"), now);
            state.receive_at(codex_hook("boundary", hook, 20.0, ""), now);
            state.receive_at(codex_hook("late-old", "PermissionRequest", 15.0, "c"), now);
            assert_eq!(state.pending.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["newer"], "{hook}");
        }
        let now = Instant::now();
        let mut state = State::default();
        state.receive_at(codex_hook("waiting", "PermissionRequest", 10.0, "a"), now);
        let mut done = codex_hook("done", "Stop", 20.0, ""); done.phase = Phase::TaskComplete;
        state.receive_at(done, now);
        state.receive_at(codex_hook("late-pre", "PreToolUse", 15.0, "b"), now);
        assert_eq!(state.current_at(now).unwrap().id, "done");
        assert_eq!(state.pending.len(), 1);
    }

    #[test]
    fn incomplete_tool_identity_does_not_guess_that_permission_was_granted() {
        for missing in ["key", "request-time", "completion-time", "session"] {
            let now = Instant::now();
            let mut state = State::default();
            let mut request = codex_hook("waiting", "PermissionRequest", 10.0, "");
            let mut completed = codex_hook("completed", "PostToolUse", 20.0, "");
            request.tool_input_key = "key".into(); completed.tool_input_key = "key".into();
            match missing {
                "key" => completed.tool_input_key.clear(),
                "request-time" => request.observed_at = 0.0,
                "completion-time" => completed.observed_at = 0.0,
                _ => { request.session.clear(); completed.session.clear(); }
            }
            state.receive_at(request, now);
            state.receive_at(completed, now);
            assert_eq!(state.pending.len(), 1, "{missing}");
        }
    }

    #[test]
    fn source_changes_and_manual_dismissal_remove_deferred_state() {
        let now = Instant::now();
        let mut state = State::default();
        let mut work = codex_hook("work", "PreToolUse", 10.0, "a"); work.host = "terminal".into();
        let mut request = codex_hook("waiting", "PermissionRequest", 11.0, "a"); request.host = "terminal".into();
        state.receive_at(work, now); state.receive_at(request, now);
        state.dismiss(&["waiting".into()]);
        assert!(state.current_at(now).is_none());
        assert!(state.visible_after.is_empty());
        state.receive_at(codex_hook("completed", "PostToolUse", 20.0, "a"), now);
        state.retain_sources(&[]);
        assert!(state.resolved.is_empty());
        state.receive_at(codex_hook("new", "PermissionRequest", 10.0, "a"), now);
        assert_eq!(state.pending.len(), 1);
    }

    #[test]
    fn completed_call_history_is_bounded() {
        let now = Instant::now();
        let mut state = State::default();
        for i in 0..150 {
            state.receive_at(codex_hook(&format!("done-{i}"), "PostToolUse", i as f64 + 1.0, &format!("call-{i}")), now);
        }
        assert_eq!(state.resolved.len(), 128);
        state.receive_at(codex_hook("late-recent", "PermissionRequest", 200.0, "call-149"), now);
        assert!(state.pending.is_empty());
    }
}
