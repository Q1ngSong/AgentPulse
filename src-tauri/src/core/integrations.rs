//! 工具接入：改 Claude Code / Codex 的 Hook 配置文件。只动 AgentPulse 自己的条目，改前备份。
use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Map, Value};

use super::events::{agent_config_file, agent_name, ACTIVITY_HOOKS, HOOK_EVENTS};
use super::paths::{atomic_write_json, backup_private, Paths};

fn validate_agent(agent: &str) -> io::Result<()> {
    if super::config::TOOLS.contains(&agent) { return Ok(()); }
    Err(io::Error::new(io::ErrorKind::InvalidInput, format!("未知工具：{agent}")))
}

/// 面板与 CLI 共用的 shell 命令；路径中的单引号必须先结束引号、转义再重新打开。
pub fn hook_command(binary: &Path, agent: &str) -> io::Result<String> {
    validate_agent(agent)?;
    let path = binary.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Hook 路径不是 UTF-8"))?;
    Ok(format!("'{}' {agent}", path.replace('\'', "'\\''")))
}

/// 判断一条 hook 是不是 AgentPulse 装的（Python 版 hook.py 和 Rust 版 agentpulse-hook 都认）。
fn is_ours(hook: &Value) -> bool {
    let cmd = hook.get("command").and_then(Value::as_str).unwrap_or("").to_lowercase();
    cmd.contains("agentpulse") && (cmd.contains("hook.py") || cmd.contains("agentpulse-hook"))
}

/// 读配置文件；解析失败直接报错，绝不覆盖用户文件。
fn read_hook_file(path: &Path) -> io::Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} 顶层不是 JSON 对象", path.display()))),
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
    }
}

fn strip_ours(hooks: &mut Map<String, Value>) {
    let keys: Vec<String> = hooks.keys().cloned().collect();
    for ev in keys {
        let groups: Vec<Value> = hooks[&ev]
            .as_array()
            .map(|gs| {
                gs.iter()
                    .filter_map(|g| {
                        let inner: Vec<Value> = g.get("hooks").and_then(Value::as_array)
                            .map(|hs| hs.iter().filter(|h| !is_ours(h)).cloned().collect())
                            .unwrap_or_default();
                        if inner.is_empty() {
                            None
                        } else {
                            let mut g2 = g.as_object().cloned().unwrap_or_default();
                            g2.insert("hooks".into(), Value::Array(inner));
                            Some(Value::Object(g2))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if groups.is_empty() {
            hooks.remove(&ev);
        } else {
            hooks.insert(ev, Value::Array(groups));
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub agent: String,
    pub name: String,
    pub file: String,
    pub file_exists: bool,
    pub tool_dir_exists: bool,
    pub events: Map<String, Value>,
    pub installed: bool,
    pub error: Option<String>,
    pub backup: Option<String>,
}

/// 某个工具的接入状态：各 Hook 是否已装、配置文件是否可解析。
pub fn integration_status(home: &Path, agent: &str) -> Status {
    let path = agent_config_file(home, agent);
    let mut status = Status {
        agent: agent.into(), name: agent_name(agent).into(), file: path.to_string_lossy().into(),
        file_exists: path.exists(), tool_dir_exists: path.parent().map(Path::exists).unwrap_or(false),
        events: Map::new(), installed: false, error: None, backup: None,
    };
    let hooks = match read_hook_file(&path) {
        Ok(m) => m.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default(),
        Err(e) => {
            status.error = Some(format!("配置文件无法解析：{e}"));
            Map::new()
        }
    };
    for (ev, hook_ev) in HOOK_EVENTS.iter().chain(ACTIVITY_HOOKS.iter()) {
        let on = hooks.get(*hook_ev).and_then(Value::as_array)
            .map(|gs| gs.iter().any(|g| g.get("hooks").and_then(Value::as_array).map(|hs| hs.iter().any(is_ours)).unwrap_or(false)))
            .unwrap_or(false);
        status.events.insert(ev.to_string(), Value::Bool(on));
    }
    status.installed = status.events.values().all(|v| v.as_bool().unwrap_or(false));
    status
}

fn backup(paths: &Paths, path: &Path, agent: &str) -> io::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let name = path.file_name().unwrap().to_string_lossy();
    let dest = paths.backups().join(format!("{agent}-{name}.{}.bak", chrono::Local::now().format("%Y%m%d-%H%M%S-%f")));
    backup_private(path, &dest)?;
    Ok(Some(dest.to_string_lossy().into()))
}

/// 接入：备份后替换 AgentPulse 自己的 Hook 条目。hook_command 是写进配置的完整命令（含参数）。
pub fn install(paths: &Paths, home: &Path, agent: &str, hook_command: &str) -> io::Result<Status> {
    validate_agent(agent)?;
    let path = agent_config_file(home, agent);
    let mut data = read_hook_file(&path)?;
    let backup = backup(paths, &path, agent)?;
    let mut hooks = data.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default();
    strip_ours(&mut hooks);
    for (_, hook_ev) in HOOK_EVENTS.iter().chain(ACTIVITY_HOOKS.iter()) {
        // 所有事件都是异步 + 15 秒超时：Hook 从不通过 stdout 替用户做决定，Claude Code 不需要等它跑完。
        let entry = json!({"hooks": [{"type": "command", "command": hook_command, "async": true, "timeout": 15}]});
        match hooks.get_mut(*hook_ev).and_then(Value::as_array_mut) {
            Some(arr) => arr.push(entry),
            None => { hooks.insert(hook_ev.to_string(), Value::Array(vec![entry])); }
        }
    }
    data.insert("hooks".into(), Value::Object(hooks));
    atomic_write_json(&path, &Value::Object(data))?;
    let mut s = integration_status(home, agent);
    s.backup = backup;
    Ok(s)
}

/// 断开：备份后只删除 AgentPulse 自己的 Hook 条目。
pub fn uninstall(paths: &Paths, home: &Path, agent: &str) -> io::Result<Status> {
    validate_agent(agent)?;
    let path = agent_config_file(home, agent);
    if !path.exists() {
        return Ok(integration_status(home, agent));
    }
    let mut data = read_hook_file(&path)?;
    let backup = backup(paths, &path, agent)?;
    let mut hooks = data.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default();
    strip_ours(&mut hooks);
    data.insert("hooks".into(), Value::Object(hooks));
    atomic_write_json(&path, &Value::Object(data))?;
    let mut s = integration_status(home, agent);
    s.backup = backup;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::super::config;
    use super::super::paths::temp_paths;
    use super::*;

    const CMD: &str = "'/Applications/AgentPulse.app/Contents/MacOS/agentpulse-hook' claude";

    fn setup() -> (tempfile::TempDir, Paths, std::path::PathBuf) {
        let (d, p) = temp_paths();
        let home = d.path().join("home-user");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        (d, p, home)
    }

    /// 给某个工具写一张悬浮窗卡片；extra 里放 mode / enabled 之类的字段。
    fn save_desktop_card(p: &Paths, agent: &str, extra: Value) {
        let mut card = json!({"id": "desktop", "type": "desktop", "name": "macOS 弹窗", "enabled": true,
            "events": ["permission_request", "task_complete"], "mode": "overlay"});
        for (k, v) in extra.as_object().unwrap() {
            card[k] = v.clone();
        }
        let mut cfg = config::Config { templates: Map::new(), tools: Default::default(), pets: Vec::new() };
        cfg.set_tool_targets(agent, &[serde_json::from_value(card).unwrap()]);
        config::save_config(p, &cfg).unwrap();
    }

    fn our_permission_hook(home: &Path, agent: &str) -> Value {
        let data: Value = serde_json::from_str(&fs::read_to_string(agent_config_file(home, agent)).unwrap()).unwrap();
        data["hooks"]["PermissionRequest"].as_array().unwrap().iter()
            .flat_map(|g| g["hooks"].as_array().unwrap()).find(|h| is_ours(h)).cloned().unwrap()
    }

    #[test]
    fn install_keeps_other_hooks_and_is_idempotent() {
        let (_d, p, home) = setup();
        save_desktop_card(&p, "claude", json!({}));
        let f = agent_config_file(&home, "claude");
        fs::write(&f, json!({"model": "x", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "afplay Glass"}]}]}}).to_string()).unwrap();
        install(&p, &home, "claude", CMD).unwrap();
        let r = install(&p, &home, "claude", CMD).unwrap();
        let data: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(data["model"], "x");
        assert_eq!(data["hooks"]["Stop"][0]["hooks"][0]["command"], "afplay Glass");
        for ev in ["PermissionRequest", "Stop", "UserPromptSubmit", "PostToolUse"] {
            let ours: Vec<&Value> = data["hooks"][ev].as_array().unwrap().iter()
                .flat_map(|g| g["hooks"].as_array().unwrap()).filter(|h| is_ours(h)).collect();
            assert_eq!(ours.len(), 1, "{ev}");
            assert_eq!(ours[0]["async"], true, "{ev} async");
            assert_eq!(ours[0]["timeout"], 15, "{ev} timeout");
        }
        assert!(r.installed);
        assert!(r.backup.is_some());
    }

    #[test]
    fn uninstall_removes_only_ours() {
        let (_d, p, home) = setup();
        let f = agent_config_file(&home, "claude");
        fs::write(&f, json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "afplay Glass"}]}]}}).to_string()).unwrap();
        install(&p, &home, "claude", CMD).unwrap();
        uninstall(&p, &home, "claude").unwrap();
        let data: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(data["hooks"], json!({"Stop": [{"hooks": [{"type": "command", "command": "afplay Glass"}]}]}));
        assert!(!integration_status(&home, "claude").installed);
    }

    #[test]
    fn codex_file_created_when_missing_and_python_hooks_recognised() {
        let (_d, p, home) = setup();
        install(&p, &home, "codex", CMD).unwrap();
        assert!(integration_status(&home, "codex").installed);
        assert!(is_ours(&json!({"command": "'/opt/miniconda3/bin/python3' '/x/AgentPulse/cli/hook.py' claude"})));
        assert!(!is_ours(&json!({"command": "afplay Glass"})));
    }

    #[test]
    fn broken_json_is_never_overwritten() {
        let (_d, p, home) = setup();
        let f = agent_config_file(&home, "codex");
        fs::write(&f, "{bad").unwrap();
        assert!(install(&p, &home, "codex", CMD).is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), "{bad");
        assert!(integration_status(&home, "codex").error.is_some());
    }

    #[test]
    fn unknown_agent_never_changes_config() {
        let (_d, p, home) = setup();
        let f = agent_config_file(&home, "claude");
        let original = "{\"model\": \"keep\"}\n";
        fs::write(&f, original).unwrap();
        for agent in ["", "cluade", "claude; echo bad"] {
            assert_eq!(install(&p, &home, agent, CMD).unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert_eq!(uninstall(&p, &home, agent).unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert!(hook_command(Path::new("/tmp/agentpulse-hook"), agent).is_err());
        }
        assert_eq!(fs::read_to_string(f).unwrap(), original);
        assert!(!p.backups().exists());
    }

    #[test]
    fn failed_backup_leaves_original_config_intact() {
        let (_d, p, home) = setup();
        let f = agent_config_file(&home, "claude");
        fs::write(&f, "{\"model\": \"keep\"}").unwrap();
        fs::create_dir_all(&p.data_dir).unwrap();
        fs::write(p.backups(), "block backup directory").unwrap();
        assert!(install(&p, &home, "claude", CMD).is_err());
        assert!(uninstall(&p, &home, "claude").is_err());
        assert_eq!(fs::read_to_string(f).unwrap(), "{\"model\": \"keep\"}");
    }

    #[cfg(unix)]
    #[test]
    fn backup_keeps_original_bytes_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, p, home) = setup();
        let f = agent_config_file(&home, "claude");
        let original = b"{\n  \"secret\": \"private-value\"\n}\n";
        fs::write(&f, original).unwrap();
        fs::set_permissions(f.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
        let backup = install(&p, &home, "claude", CMD).unwrap().backup.unwrap();
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert_eq!(fs::metadata(backup).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn quoted_hook_path_executes_literally() {
        use std::os::unix::fs::PermissionsExt;
        let (d, _p) = temp_paths();
        let dir = d.path().join("Owner's $HOME `false` 中文");
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("agentpulse-hook");
        fs::write(&binary, "#!/bin/sh\nprintf '%s' \"$1\"\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        for agent in config::TOOLS {
            let output = std::process::Command::new("/bin/sh").arg("-c").arg(hook_command(&binary, agent).unwrap()).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(output.stdout, agent.as_bytes());
        }
    }

    #[test]
    fn permission_request_hook_is_always_async() {
        let (_d, p, home) = setup();
        save_desktop_card(&p, "claude", json!({}));
        let r = install(&p, &home, "claude", CMD).unwrap();
        assert!(r.installed);
        assert_eq!(our_permission_hook(&home, "claude")["async"], true);
        assert_eq!(our_permission_hook(&home, "claude")["timeout"], 15);
    }

    #[test]
    fn codex_keeps_every_hook_async() {
        let (_d, p, home) = setup();
        install(&p, &home, "codex", CMD).unwrap();
        let data: Value = serde_json::from_str(&fs::read_to_string(agent_config_file(&home, "codex")).unwrap()).unwrap();
        for ev in ["PermissionRequest", "Stop", "UserPromptSubmit", "PostToolUse"] {
            let h = &data["hooks"][ev][0]["hooks"][0];
            assert_eq!(h["async"], true, "{ev}");
            assert_eq!(h["timeout"], 15, "{ev}");
        }
    }
}
