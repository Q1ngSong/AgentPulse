//! 每只编号桌宠有自己的窗口、素材和消息队列；来源选择可跨 Agent 和 App。
use std::{collections::BTreeMap, sync::{Mutex, OnceLock}};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use crate::core::{config, paths::Paths, pet::{PetConfig, Phase, State, Update}};

struct Companion { config: PetConfig, state: State, revision: u64 }
#[derive(Default)]
struct Companions { pets: BTreeMap<String, Companion> }
impl Companions {
    fn configure(&mut self, configs: &[PetConfig]) {
        self.pets.retain(|id, _| configs.iter().any(|c| c.id == *id && c.enabled));
        for config in configs.iter().filter(|c| c.enabled) {
            let pet = self.pets.entry(config.id.clone()).or_insert_with(|| Companion {
                config: config.clone(), state: State::default(), revision: 0,
            });
            if pet.config != *config {
                pet.state.retain_sources(&config.sources);
                pet.state.pending.retain(|n| config.events.iter().any(|e| Some(e.as_str()) == event(&n.phase)));
                pet.state.refresh_animations(&config.animations);
                pet.revision += 1;
                pet.config = config.clone();
            }
        }
    }
    fn receive(&mut self, update: Update) {
        for pet in self.pets.values_mut().filter(|p| p.config.matches(&update.agent, &update.host)) {
            let mut update = update.clone();
            update.animations = Some(pet.config.animations.clone());
            pet.state.receive(update);
        }
    }
    fn dismiss_source(&mut self, host: &str) {
        for pet in self.pets.values_mut() {
            pet.state.dismiss(&pet.state.source_ids(host));
        }
    }
}
fn event(phase: &Phase) -> Option<&'static str> {
    match phase { Phase::PermissionRequest => Some("permission_request"), Phase::TaskComplete => Some("task_complete"), _ => None }
}
static PETS: OnceLock<Mutex<Companions>> = OnceLock::new();
static CONFIGURATION: Mutex<()> = Mutex::new(());
/// 读配置、改配置和同步窗口属于同一次操作，防止旧提醒重新打开刚关闭的桌宠。
pub(crate) fn configuration_lock() -> std::sync::MutexGuard<'static, ()> { CONFIGURATION.lock().unwrap() }
fn companions() -> &'static Mutex<Companions> { PETS.get_or_init(|| Mutex::new(Companions::default())) }
fn window_label(id: &str) -> String { format!("pet-{id}") }
fn display_name(config: &PetConfig) -> String {
    if config.name.is_empty() { format!("桌宠 {}", config.number) }
    else { format!("桌宠 {} · {}", config.number, config.name) }
}

pub fn receive(update: Update) {
    if matches!(update.phase, Phase::Working | Phase::Sleeping) { companions().lock().unwrap().receive(update); }
}
pub fn dismiss_source(host: &str) { companions().lock().unwrap().dismiss_source(host); }

/// 分发与显示之间可能改过同步选项；以当前配置再次确认目的桌宠。
pub fn notify(app: &AppHandle, instance: &str, mut update: Update) -> Result<bool, String> {
    if !crate::interactive() { return Ok(false); }
    let _configuration = configuration_lock();
    let cfg = config::load_config(&Paths::from_env()).map_err(|e| e.to_string())?;
    sync_config(app, &cfg)?;
    let mut pets = companions().lock().unwrap();
    let pet = pets.pets.get_mut(instance).ok_or("桌宠已关闭或删除")?;
    if !pet.config.matches(&update.agent, &update.host)
        || !pet.config.events.iter().any(|e| Some(e.as_str()) == event(&update.phase)) {
        return Err("这只桌宠未同步此来源或事件".into());
    }
    update.animations = Some(pet.config.animations.clone());
    pet.state.receive(update);
    Ok(true)
}

/// 测试只在独立预览窗口显示，不覆盖已保存桌宠的队列或同步选项。
pub fn preview(app: &AppHandle, mut config: PetConfig, mut update: Update) -> Result<(), String> {
    let _configuration = configuration_lock();
    config.id = format!("preview-{}", uuid::Uuid::new_v4().simple());
    config.enabled = true;
    config.name = if config.name.is_empty() { "预览".into() } else { format!("{} · 预览", config.name) };
    update.animations = Some(config.animations.clone());
    let mut state = State::default();
    state.receive(update);
    companions().lock().unwrap().pets.insert(config.id.clone(), Companion { config: config.clone(), state, revision: 0 });
    show(app, &config, 0)
}

/// 保留仍订阅的消息；删除、关闭只销毁相应桌宠的窗口。
pub fn sync_config(app: &AppHandle, cfg: &config::Config) -> Result<(), String> {
    crate::core::pet::validate_configs(&cfg.pets)?;
    let (removed, enabled) = {
        let mut pets = companions().lock().unwrap();
        let previous = pets.pets.keys().cloned().collect::<Vec<_>>();
        pets.configure(&cfg.pets);
        let removed = previous.into_iter().filter(|id| !pets.pets.contains_key(id)).collect::<Vec<_>>();
        let mut enabled = pets.pets.values().map(|p| p.config.clone()).collect::<Vec<_>>();
        enabled.sort_by_key(|p| p.number);
        (removed, enabled)
    };
    for id in removed {
        if let Some(win) = app.get_webview_window(&window_label(&id)) { win.destroy().map_err(|e| e.to_string())?; }
    }
    for (index, config) in enabled.iter().enumerate() { show(app, config, index)?; }
    Ok(())
}

#[tauri::command]
pub fn get_pet_state(instance: String) -> Value {
    let pets = companions().lock().unwrap();
    let Some(pet) = pets.pets.get(&instance) else {
        return json!({"instance":instance,"enabled":false,"current":null,"pending":0});
    };
    json!({"instance":instance,"number":pet.config.number,"name":display_name(&pet.config),
        "enabled":true,"current":pet.state.current(),"pending":pet.state.visible_pending_len(),
        "animations":pet.config.animations,"revision":pet.revision})
}

/// 捕获当前消息来源与 ID 快照，跳转成功后清理该快照，保留随后到达的消息。
#[tauri::command]
pub async fn activate_pet_source(app: AppHandle, instance: String) -> Result<(), String> {
    let (host, ids) = {
        let pets = companions().lock().unwrap();
        let pet = pets.pets.get(&instance).ok_or("找不到这只桌宠")?;
        let first = pet.state.current().ok_or("没有可返回的来源")?;
        if first.host.is_empty() { return Err("当前消息的来源 App 无法识别".into()); }
        (first.host.clone(), pet.state.source_ids(&first.host))
    };
    tauri::async_runtime::spawn_blocking(move || {
        crate::overlay::activate(&app, &host)?;
        if let Some(pet) = companions().lock().unwrap().pets.get_mut(&instance) { pet.state.dismiss(&ids); }
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

fn show(app: &AppHandle, config: &PetConfig, index: usize) -> Result<(), String> {
    let app2 = app.clone();
    let id = config.id.clone();
    let title = display_name(config);
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let result = (|| {
            let label = window_label(&id);
            if let Some(win) = app2.get_webview_window(&label) {
                win.set_title(&title).map_err(|e| e.to_string())?;
                return win.show().map_err(|e| e.to_string());
            }
            let (x, y) = app2.primary_monitor().ok().flatten().map(|m| {
                let size = m.size().to_logical::<f64>(m.scale_factor());
                let pos = m.position().to_logical::<f64>(m.scale_factor());
                let columns = ((size.width - 40.0) / 280.0).floor().max(1.0) as usize;
                let rows = ((size.height - 60.0) / 300.0).floor().max(1.0) as usize;
                let slot = index % (columns * rows);
                (pos.x + (size.width - 340.0 - (slot % columns) as f64 * 280.0).max(0.0),
                 pos.y + (size.height - 420.0 - (slot / columns) as f64 * 300.0).max(30.0))
            }).unwrap_or((800.0 - index as f64 * 30.0, 400.0));
            let win = WebviewWindowBuilder::new(&app2, &label, WebviewUrl::App(format!("pet.html?instance={id}").into()))
                .title(&title).inner_size(320.0, 360.0).position(x, y)
                .decorations(false).transparent(true).shadow(false).always_on_top(true)
                .skip_taskbar(true).visible_on_all_workspaces(true).focused(false).resizable(false)
                .build().map_err(|e| e.to_string())?;
            if let Err(e) = crate::overlay::make_nonactivating_panel(&win) {
                let _ = win.destroy();
                return Err(e);
            }
            Ok(())
        })();
        let _ = tx.send(result);
    }).map_err(|e| e.to_string())?;
    rx.recv_timeout(std::time::Duration::from_secs(5)).map_err(|_| "创建桌宠窗口超时".to_string())?
}

#[tauri::command]
pub async fn close_pet(app: AppHandle, instance: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _configuration = configuration_lock();
        let paths = Paths::from_env();
        let mut cfg = config::load_config(&paths).map_err(|e| e.to_string())?;
        if let Some(pet) = cfg.pets.iter_mut().find(|p| p.id == instance) {
            pet.enabled = false;
            config::save_config(&paths, &cfg).map_err(|e| e.to_string())?;
            if let Err(e) = app.emit("pet-enabled-changed", json!({"id":instance,"enabled":false})) {
                crate::app_log(&format!("同步桌宠关闭状态失败：{e}"));
            }
        }
        companions().lock().unwrap().pets.remove(&instance);
        if let Some(win) = app.get_webview_window(&window_label(&instance)) { win.destroy().map_err(|e| e.to_string())?; }
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

pub fn restore(app: AppHandle) {
    std::thread::spawn(move || {
        let _configuration = configuration_lock();
        let result = config::load_config(&Paths::from_env()).map_err(|e| e.to_string()).and_then(|cfg| sync_config(&app, &cfg));
        if let Err(e) = result { crate::app_log(&format!("恢复桌宠失败：{e}")); }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pet::{PetSource, Phase};
    fn config(id: &str, number: u32, sources: &[(&str, &str)]) -> PetConfig {
        PetConfig { id: id.into(), number, name: String::new(), enabled: true,
            events: vec!["task_complete".into(), "permission_request".into()],
            animations: crate::core::pet_animation::defaults(),
            sources: sources.iter().map(|(agent, host)| PetSource { agent: (*agent).into(), host: (*host).into() }).collect() }
    }
    fn update(id: &str, agent: &str, host: &str, phase: Phase) -> Update {
        Update { id: id.into(), agent: agent.into(), host: host.into(), phase, session: id.into(), title: id.into(), detail: String::new(), animations: None,
            hook_event: String::new(), tool_use_id: String::new(), tool_input_key: String::new(), observed_at: 0.0 }
    }
    #[test]
    fn aggregate_sources_and_keep_other_pets_when_one_is_closed() {
        let one = config("one", 1, &[("codex", "terminal"), ("claude", "vscode")]);
        let mut two = config("two", 2, &[("codex", "")]);
        let mut pets = Companions::default();
        pets.configure(&[one.clone(), two.clone()]);
        pets.receive(update("a", "codex", "terminal", Phase::TaskComplete));
        pets.receive(update("b", "claude", "vscode", Phase::PermissionRequest));
        pets.receive(update("c", "claude", "terminal", Phase::TaskComplete));
        assert_eq!(pets.pets["one"].state.pending.len(), 2);
        assert_eq!(pets.pets["two"].state.pending.len(), 1);
        two.enabled = false;
        pets.configure(&[one, two]);
        assert!(!pets.pets.contains_key("two"));
        assert_eq!(pets.pets["one"].state.pending.len(), 2);
        pets.dismiss_source("terminal");
        assert_eq!(pets.pets["one"].state.current().unwrap().id, "b");
    }
    #[test]
    fn changing_sources_and_events_prunes_only_unsubscribed_messages() {
        let mut one = config("one", 1, &[("codex", "")]);
        let mut pets = Companions::default();
        pets.configure(&[one.clone()]);
        pets.receive(update("a", "codex", "terminal", Phase::TaskComplete));
        pets.receive(update("b", "codex", "vscode", Phase::PermissionRequest));
        pets.receive(update("c", "codex", "vscode", Phase::TaskComplete));
        pets.receive(update("w", "codex", "terminal", Phase::Working));
        one.sources[0].host = "vscode".into();
        one.events = vec!["permission_request".into()];
        pets.configure(&[one]);
        assert_eq!(pets.pets["one"].state.pending.len(), 1);
        pets.dismiss_source("vscode");
        assert!(pets.pets["one"].state.current().is_none());
    }
}
