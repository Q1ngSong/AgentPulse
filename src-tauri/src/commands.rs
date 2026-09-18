//! 给前端的 Tauri 命令：全部是对 core 的薄封装，参数/返回值用 JSON。
use std::path::PathBuf;

use serde_json::{json, Map, Value};
use tauri::AppHandle;

use crate::core::config::{self, Config, Target};
use crate::core::runtime::{hook_binary, Runtime};
use crate::core::pet::{self, PetConfig, Phase, Update};
use crate::core::{codex, dispatch, events, icons, integrations, log, paths::Paths, platform, queue, sounds};

type R<T> = Result<T, String>;

fn rt() -> Runtime {
    Runtime::system(Paths::from_env())
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn parse_targets(v: Vec<Value>) -> R<Vec<Target>> {
    v.into_iter().map(|t| serde_json::from_value(t).map_err(err)).collect()
}

#[tauri::command]
pub fn get_state() -> R<Value> {
    let paths = Paths::from_env();
    let home = rt().home;
    let cfg = config::load_config(&paths).map_err(err)?;
    let records = log::read_events(&paths, 1000);
    let today = chrono::Local::now().date_naive();
    let todays: Vec<&Value> = records.iter().filter(|e| {
        let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
        let src = e.get("source").and_then(Value::as_str).unwrap_or("");
        chrono::DateTime::from_timestamp(ts as i64, 0).map(|d| d.with_timezone(&chrono::Local).date_naive() == today).unwrap_or(false)
            && src != "test" && src != "delayed"
    }).collect();
    let queued = queue::queued_per_agent(&paths);
    let mut stats = Map::new();
    let mut integ = Map::new();
    let mut icon_map = Map::new();
    for (agent, _) in events::AGENTS {
        let mine: Vec<&&Value> = todays.iter().filter(|e| e.get("agent").and_then(Value::as_str) == Some(agent)).collect();
        stats.insert(agent.into(), json!({
            "today": mine.len(),
            "failed_today": mine.iter().filter(|e| e.get("results").and_then(Value::as_array).map(|rs| rs.iter().any(|r| r.get("ok") == Some(&Value::Bool(false)))).unwrap_or(false)).count(),
            "queued": queued.get(agent).cloned().unwrap_or(json!(0)),
        }));
        integ.insert(agent.into(), serde_json::to_value(integrations::integration_status(&home, agent)).map_err(err)?);
        icon_map.insert(agent.into(), json!(icons::custom_data_url(&paths, agent)));
    }
    Ok(json!({
        "config": config::redact(&cfg),
        "pet_sources": available_pet_sources(&cfg, &records, &log::read_activity(&paths, 400)),
        "integrations": integ,
        "stats": stats,
        "icons": icon_map,
        "hook_binary": hook_binary().to_string_lossy(),
        "hook_binary_exists": hook_binary().exists(),
        "meta": {"events": Map::from_iter(events::EVENTS.iter().map(|(k, v)| (k.to_string(), json!(v)))),
                 "agents": Map::from_iter(events::AGENTS.iter().map(|(k, v)| (k.to_string(), json!(v)))),
                 "data_dir": paths.data_dir.to_string_lossy()},
    }))
}

/// 已安装的常见来源、最近收到的 Hook 来源与已勾选来源取并集。
fn available_pet_sources(cfg: &Config, records: &[Value], activity: &[Value]) -> Vec<Value> {
    let mut found = std::collections::BTreeMap::new();
    let mut add = |agent: &str, host: &str, name: Option<String>| {
        if config::TOOLS.contains(&agent) && !host.is_empty() {
            found.insert((agent.to_string(), host.to_string()), name.unwrap_or_else(|| host.into()));
        }
    };
    for (_, host) in platform::TERM_PROGRAM_BUNDLES {
        if let Some(name) = platform::app_name(host) {
            for agent in config::TOOLS { add(agent, host, Some(name.clone())); }
        }
    }
    for (agent, host) in platform::AGENT_APP_BUNDLES {
        if let Some(name) = platform::app_name(host) { add(agent, host, Some(name)); }
    }
    for record in records.iter().filter(|r| r["source"] == "hook").chain(activity.iter()) {
        let agent = record["agent"].as_str().unwrap_or("");
        let host = record["host_bundle"].as_str().unwrap_or("");
        add(agent, host, platform::app_name(host));
    }
    for source in cfg.pets.iter().flat_map(|p| &p.sources) { add(&source.agent, &source.host, platform::app_name(&source.host)); }
    found.into_iter().map(|((agent, host), name)| json!({"agent":agent,"host":host,"name":name})).collect()
}

#[tauri::command]
pub async fn save_pets(app: AppHandle, pets: Vec<PetConfig>, expected: Vec<PetConfig>) -> R<()> {
    tauri::async_runtime::spawn_blocking(move || {
        pet::validate_configs(&pets)?;
        for pet in pets.iter().filter(|p| p.enabled) {
            if !expected.iter().any(|saved| saved.id == pet.id && saved.enabled && saved.animations == pet.animations) {
                crate::pet_assets::prepare(&pet.animations)?;
            }
        }
        let _configuration = config::configuration_lock();
        let paths = Paths::from_env();
        let mut cfg = config::load_config(&paths).map_err(err)?;
        if cfg.pets != expected { return Err("桌宠设置刚刚发生变化，请刷新后重试".into()); }
        cfg.pets = pets;
        config::save_config(&paths, &cfg).map_err(err)?;
        crate::pet::sync_config(&app, &cfg)
    }).await.map_err(err)?
}

#[tauri::command]
pub async fn test_pet(app: AppHandle, pet: PetConfig, event: String) -> R<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        pet::validate_configs(std::slice::from_ref(&pet))?;
        crate::pet_assets::prepare(&pet.animations)?;
        if !config::EVENT_KEYS.contains(&event.as_str()) { return Err("不支持的提醒事件".into()); }
        let agent = pet.sources.first().map(|s| s.agent.as_str()).unwrap_or("codex");
        let id = uuid::Uuid::new_v4().simple().to_string();
        let title = format!("桌宠 {} · 测试消息", pet.number);
        let body = "这是一条桌宠预览消息";
        let update = Update { id: id.clone(), agent: agent.into(), host: String::new(), session: id.clone(),
            hook_event: String::new(), tool_use_id: String::new(), tool_input_key: String::new(), observed_at: 0.0,
            phase: if event == "permission_request" { Phase::PermissionRequest } else { Phase::TaskComplete },
            title: title.clone(), detail: body.into(), animations: Some(pet.animations.clone()) };
        let result = crate::pet::preview(&app, pet.clone(), update);
        let record = json!({"id":id,"ts":log::now(),"source":"test","agent":agent,"event":event,
            "title":title,"body":body,"results":[{"target_id":pet.id,"name":pet.target().name,"type":"pet",
                "ok":result.is_ok(),"error":result.err(),"info":"已打开独立桌宠预览","ms":0}]});
        log::append_event(&Paths::from_env(), &record).map_err(err)?;
        Ok(record)
    }).await.map_err(err)?
}

/// 整体保存某个工具的提醒方式列表；MASK 字段用已保存值补回。
#[tauri::command]
pub async fn save_tool_targets(agent: String, targets: Vec<Value>) -> R<()> {
    tauri::async_runtime::spawn_blocking(move || {
    let _configuration = config::configuration_lock();
    let paths = Paths::from_env();
    let mut cfg = config::load_config(&paths).map_err(err)?;
    let saved = cfg.tool_targets(&agent);
    let restored: Vec<Target> = parse_targets(targets)?.iter().map(|t| config::restore_secrets(t, &saved)).collect();
    config::validate_targets(&restored)?;
    cfg.set_tool_targets(&agent, &restored);
    config::save_config(&paths, &cfg).map_err(err)
    }).await.map_err(err)?
}

/// 写进 Hook 配置的完整命令：与本程序同目录的 agentpulse-hook 加上工具名。
fn hook_command(agent: &str) -> R<String> {
    let bin = hook_binary();
    if !bin.exists() {
        return Err(format!("找不到 Hook 程序：{}", bin.display()));
    }
    integrations::hook_command(&bin, agent).map_err(err)
}

/// 把一张提醒方式卡片（含密钥）复制到另一个工具；弹窗/提示音会替换目标工具原有的那张。
#[tauri::command]
pub async fn copy_target(agent: String, id: String, to: String) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || {
    let _configuration = config::configuration_lock();
    if !config::TOOLS.contains(&to.as_str()) || to == agent {
        return Err("目标工具无效".into());
    }
    let paths = Paths::from_env();
    let mut cfg = config::load_config(&paths).map_err(err)?;
    let src = cfg.tool_targets(&agent).into_iter().find(|t| t.id == id).ok_or("找不到要复制的提醒方式")?;
    let mut new = src.clone();
    let mut dst = cfg.tool_targets(&to);
    if config::SINGLETON_TYPES.contains(&new.kind.as_str()) {
        new.id = new.kind.clone();
        dst.retain(|t| t.kind != new.kind);
    } else {
        new.id = format!("{}-{}", new.kind, &uuid::Uuid::new_v4().simple().to_string()[..6]);
    }
    dst.push(new.clone());
    cfg.set_tool_targets(&to, &dst);
    config::save_config(&paths, &cfg).map_err(err)?;
    Ok(new.id)
    }).await.map_err(err)?
}

#[tauri::command]
pub async fn save_templates(templates: Map<String, Value>) -> R<()> {
    tauri::async_runtime::spawn_blocking(move || {
    let _configuration = config::configuration_lock();
    let paths = Paths::from_env();
    let mut cfg: Config = config::load_config(&paths).map_err(err)?;
    cfg.templates = templates;
    config::save_config(&paths, &cfg).map_err(err)
    }).await.map_err(err)?
}

/// 用表单当前值测试发送（MASK 字段补回已保存值；忽略延迟）。
/// 发送会阻塞等待 socket / HTTP；放到阻塞线程池，让主线程能处理悬浮窗创建。
#[tauri::command]
pub async fn test_target(agent: String, target: Value, event: String) -> R<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let rt = rt();
        let saved = config::load_config(&rt.paths).map_err(err)?.tool_targets(&agent);
        let target: Target = serde_json::from_value(target).map_err(err)?;
        config::validate_targets(std::slice::from_ref(&target))?;
        let target = config::restore_secrets(&target, &saved);
        dispatch::dispatch(&rt, &agent, &event, &events::sample_context(&agent, &event), None, "test",
            Some(&json!({"note": "面板里的发送测试提醒"})), Some(vec![target])).map_err(err)
    }).await.map_err(err)?
}

/// 不用真跑工具，按当前工具的提醒方式真实发一次。
/// 与测试发送一样在阻塞线程池分发，避免主线程等自己创建悬浮窗。
#[tauri::command]
pub async fn simulate(agent: String, event: String) -> R<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let rt = rt();
        dispatch::dispatch(&rt, &agent, &event, &events::sample_context(&agent, &event), None, "simulate",
            Some(&json!({"note": "面板里的模拟触发"})), None).map_err(err)
    }).await.map_err(err)?
}

#[tauri::command]
pub fn list_sounds() -> R<Vec<sounds::Sound>> {
    sounds::list_sounds(&Paths::from_env()).map_err(err)
}

#[tauri::command]
pub fn upload_sound(name: String, data: Vec<u8>) -> R<String> {
    sounds::save_upload(&Paths::from_env(), &name, &data)
}

#[tauri::command]
pub fn rename_sound(r#ref: String, name: String) -> R<String> {
    sounds::rename(&Paths::from_env(), &r#ref, &name)
}

#[tauri::command]
pub fn delete_sound(r#ref: String) -> R<()> {
    sounds::delete(&Paths::from_env(), &r#ref)
}

#[tauri::command]
pub fn preview_sound(r#ref: String, volume: Option<u32>) -> R<()> {
    sounds::play(&Paths::from_env(), &r#ref, volume.unwrap_or(100))
}

#[tauri::command]
pub fn install_integration(agent: String) -> R<Value> {
    let paths = Paths::from_env();
    let home = rt().home;
    let cmd = hook_command(&agent)?;
    serde_json::to_value(integrations::install(&paths, &home, &agent, &cmd).map_err(err)?).map_err(err)
}

#[tauri::command]
pub async fn review_codex_hooks() -> R<codex::Review> {
    tauri::async_runtime::spawn_blocking(|| {
        codex::review(&platform::codex_binary()?, &rt().home, &hook_command("codex")?)
    }).await.map_err(err)?
}

#[tauri::command]
pub async fn trust_codex_hooks(hooks: Vec<codex::Hook>) -> R<codex::Review> {
    tauri::async_runtime::spawn_blocking(move || {
        codex::trust(&platform::codex_binary()?, &rt().home, &Paths::from_env(), &hook_command("codex")?, &hooks)
    }).await.map_err(err)?
}

#[tauri::command]
pub fn uninstall_integration(agent: String) -> R<Value> {
    let paths = Paths::from_env();
    let home = rt().home;
    serde_json::to_value(integrations::uninstall(&paths, &home, &agent).map_err(err)?).map_err(err)
}

#[tauri::command]
pub fn read_events(limit: Option<usize>) -> R<Vec<Value>> {
    Ok(log::read_events(&Paths::from_env(), limit.unwrap_or(300)))
}

/// 提醒记录页顶部的队列胶囊：与延迟队列 worker 读的是同一份 queue.json。
#[tauri::command]
pub fn read_queue() -> R<Vec<queue::Item>> {
    Ok(queue::load_queue(&Paths::from_env()))
}

/// 面板点「刷新」时顺手催一轮：不等 worker 下次醒来，立刻跑一遍撤销/发送判断。
/// 用一次性的 states，不影响后台 worker 自己的连续状态。
#[tauri::command]
pub async fn check_queue_now() -> R<Vec<queue::Item>> {
    tauri::async_runtime::spawn_blocking(|| {
        let rt = rt();
        queue::run_pass(&rt, &mut std::collections::HashMap::new()).map_err(err)?;
        Ok(queue::load_queue(&rt.paths))
    }).await.map_err(err)?
}

#[tauri::command]
pub fn clear_events() -> R<()> {
    log::clear_events(&Paths::from_env()).map_err(err)
}

/// 在 Finder 里显示：claude / codex 配置文件、data 目录、sounds、logs。
#[tauri::command]
pub fn reveal(key: String) -> R<()> {
    let paths = Paths::from_env();
    let home = rt().home;
    let target: PathBuf = match key.as_str() {
        "claude" | "codex" => events::agent_config_file(&home, &key),
        "sounds" => { let d = paths.sounds(); std::fs::create_dir_all(&d).map_err(err)?; d }
        "logs" => { let d = paths.logs(); std::fs::create_dir_all(&d).map_err(err)?; d }
        _ => paths.config(),
    };
    let target = if target.exists() { target } else { target.parent().map(|p| p.to_path_buf()).unwrap_or(target) };
    if !target.exists() {
        return Err(format!("{} 不存在", target.display()));
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = std::process::Command::new("open");
        if target.is_file() { c.arg("-R"); }
        c.arg(&target).status().map_err(err)?;
    }
    Ok(())
}

/// 开机自启（macOS 用 .app 路径登记登录项；Windows 注册表；Linux XDG autostart）。
fn auto_launcher() -> R<auto_launch::AutoLaunch> {
    let exe = std::env::current_exe().map_err(err)?;
    let path = exe.to_string_lossy().to_string();
    // macOS 要用 .app 路径，否则登录项会打开终端
    let app_path = match path.find(".app/Contents/MacOS/") { Some(i) if cfg!(target_os = "macos") => path[..i + 4].to_string(), _ => path };
    auto_launch::AutoLaunchBuilder::new().set_app_name("AgentPulse").set_app_path(&app_path).build().map_err(err)
}

#[tauri::command]
pub fn get_auto_launch() -> R<bool> {
    auto_launcher()?.is_enabled().map_err(err)
}

#[tauri::command]
pub fn set_auto_launch(enabled: bool) -> R<()> {
    let l = auto_launcher()?;
    if enabled { l.enable().map_err(err) } else { l.disable().map_err(err) }
}

#[tauri::command]
pub fn activate_app(app: AppHandle, bundle: String) -> R<()> {
    crate::overlay::activate(&app, &bundle)
}

/// 设置页：给某个工具上传自定义悬浮窗图标（覆盖旧的）。
#[tauri::command]
pub fn upload_tool_icon(agent: String, filename: String, data: Vec<u8>) -> R<()> {
    icons::save_upload(&Paths::from_env(), &agent, &filename, &data)
}

/// 设置页：删掉某个工具的自定义图标，改回自动读来源 App 的图标。
#[tauri::command]
pub fn delete_tool_icon(agent: String) -> R<()> {
    icons::delete(&Paths::from_env(), &agent)
}
