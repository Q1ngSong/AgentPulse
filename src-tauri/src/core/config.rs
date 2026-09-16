//! 提醒配置：按工具保存提醒方式，提供默认值、文件读写与密钥脱敏。
use std::collections::BTreeMap;
use std::fs;
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::paths::{atomic_write_json, chmod_private, Paths};
use super::pet::{self, PetConfig};
static CONFIGURATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// 串行保存面板设置，避免并发操作覆盖其他配置。
pub(crate) fn configuration_lock() -> std::sync::MutexGuard<'static, ()> { CONFIGURATION.lock().unwrap() }

pub const EVENT_KEYS: [&str; 2] = ["permission_request", "task_complete"];
pub const TOOLS: [&str; 2] = ["claude", "codex"];
/// 每个工具最多一张
pub const SINGLETON_TYPES: [&str; 2] = ["desktop", "sound"];
pub const TARGET_TYPES: [&str; 4] = ["desktop", "sound", "feishu", "wxtest"];
pub const SECRET_FIELDS: [&str; 2] = ["secret", "webhook"];
pub const MASK: &str = "••••••";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Template {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
}

/// 一张提醒方式卡片。类型相关字段（mode/sounds/webhook/…）放在 extra 里，和 Python 版的 dict 一致。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Target {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_true() -> bool {
    true
}

impl Target {
    pub fn str(&self, key: &str) -> &str {
        self.extra.get(key).and_then(Value::as_str).unwrap_or("")
    }
    pub fn u64(&self, key: &str) -> u64 {
        match self.extra.get(key) {
            Some(Value::Number(n)) => n.as_f64().map(|f| f.max(0.0) as u64).unwrap_or(0),
            Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
            _ => 0,
        }
    }
    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.extra.get(key).and_then(Value::as_bool).unwrap_or(default)
    }
    pub fn set(&mut self, key: &str, v: Value) {
        self.extra.insert(key.into(), v);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Tool {
    #[serde(default)]
    pub targets: Vec<Target>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub templates: Map<String, Value>,
    pub tools: BTreeMap<String, Tool>,
    #[serde(default)]
    pub pets: Vec<PetConfig>,
}

impl Config {
    pub fn template(&self, event: &str) -> Template {
        self.templates
            .get(event)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_else(|| default_template(event))
    }

    /// 某个工具的提醒方式列表（按卡片顺序）；工具不存在时为空。
    pub fn tool_targets(&self, agent: &str) -> Vec<Target> {
        self.tools.get(agent).map(|t| t.targets.clone()).unwrap_or_default()
    }

    pub fn set_tool_targets(&mut self, agent: &str, targets: &[Target]) {
        self.tools.insert(agent.into(), Tool { targets: targets.to_vec() });
    }
}

fn default_template(event: &str) -> Template {
    match event {
        "permission_request" => Template { title: "{agent}: {session}".into(), body: "🔐 需要授权 · {detail}".into() },
        _ => Template { title: "{agent}: {session}".into(), body: "✅ 任务完成 · {detail}".into() },
    }
}

fn default_targets() -> Vec<Target> {
    vec![
        serde_json::from_value(json!({"id": "desktop", "type": "desktop", "name": "屏幕弹窗", "enabled": true,
            "events": EVENT_KEYS, "mode": "overlay", "click": "activate"})).unwrap(),
        serde_json::from_value(json!({"id": "sound", "type": "sound", "name": "提示音", "enabled": true,
            "events": EVENT_KEYS, "sounds": {"permission_request": "system:Ping", "task_complete": "system:Glass"},
            "volume": 100})).unwrap(),
    ]
}

pub fn default_config() -> Config {
    let mut cfg = Config { templates: Map::new(), tools: BTreeMap::new(), pets: Vec::new() };
    for e in EVENT_KEYS {
        cfg.templates.insert(e.into(), serde_json::to_value(default_template(e)).unwrap());
    }
    for t in TOOLS {
        cfg.set_tool_targets(t, &default_targets());
    }
    cfg
}

/// 返回去掉密钥的副本给面板显示：已填写的密钥字段替换为 MASK。
pub fn redact(cfg: &Config) -> Config {
    let mut out = cfg.clone();
    for t in TOOLS {
        let mut targets = out.tool_targets(t);
        for target in &mut targets {
            for k in SECRET_FIELDS {
                if !target.str(k).is_empty() {
                    target.set(k, Value::String(MASK.into()));
                }
            }
        }
        out.set_tool_targets(t, &targets);
    }
    out
}

/// 面板提交回来的 target 中仍为 MASK 的字段，用已保存的同 id 配置补回原值；找不到同 id 时置空。
pub fn restore_secrets(target: &Target, saved: &[Target]) -> Target {
    let old = saved.iter().find(|t| t.id == target.id);
    let mut out = target.clone();
    for k in SECRET_FIELDS {
        if out.str(k) == MASK {
            let v = old.map(|o| o.str(k).to_string()).unwrap_or_default();
            out.set(k, Value::String(v));
        }
    }
    out
}

/// 读取当前配置；文件不存在时使用默认值，解析失败时报错且不改写文件。
pub fn load_config(paths: &Paths) -> io::Result<Config> {
    let path = paths.config();
    if !path.exists() {
        return Ok(default_config());
    }
    let mut cfg: Config = serde_json::from_str(&fs::read_to_string(&path)?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    for tool in TOOLS {
        cfg.tools.entry(tool.into()).or_default().targets.retain(|t| TARGET_TYPES.contains(&t.kind.as_str()));
    }
    pet::validate_configs(&cfg.pets).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(cfg)
}

/// 校验渠道类型、id 非空且不重复，弹窗/提示音各最多一张。
pub fn validate_targets(targets: &[Target]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for t in targets {
        if !TARGET_TYPES.contains(&t.kind.as_str()) {
            return Err(format!("不支持的提醒方式：{}", t.kind));
        }
        if t.id.is_empty() || !seen.insert(&t.id) {
            return Err("提醒方式 id 缺失或重复".into());
        }
    }
    for kind in SINGLETON_TYPES {
        if targets.iter().filter(|t| t.kind == kind).count() > 1 {
            let name = match kind { "desktop" => "屏幕弹窗", _ => "提示音" };
            return Err(format!("每个工具最多只能有一个{name}"));
        }
    }
    Ok(())
}

/// 保存配置；某工具的 targets 不合法时报错且不写入。
pub fn save_config(paths: &Paths, cfg: &Config) -> io::Result<()> {
    for tool in cfg.tools.values() {
        validate_targets(&tool.targets).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    }
    pet::validate_configs(&cfg.pets).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    atomic_write_json(&paths.config(), cfg)?;
    chmod_private(&paths.config());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::paths::temp_paths;
    use super::*;

    #[test]
    fn default_when_missing_and_private_on_save() {
        let (_d, p) = temp_paths();
        let cfg = load_config(&p).unwrap();
        assert_eq!(cfg.tool_targets("claude").len(), 2);
        assert!(!p.config().exists(), "读取默认值不应创建文件");
        save_config(&p, &cfg).unwrap();
        let saved = fs::read(p.config()).unwrap();
        let raw: Value = serde_json::from_slice(&saved).unwrap();
        assert_eq!(raw.as_object().unwrap().len(), 3, "保存 templates、tools 和 pets");
        assert!(raw.get("version").is_none());
        assert_eq!(load_config(&p).unwrap(), cfg);
        assert_eq!(fs::read(p.config()).unwrap(), saved);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(p.config()).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn singleton_and_duplicate_ids_rejected() {
        let (_d, p) = temp_paths();
        let mut cfg = default_config();
        let mut list = cfg.tool_targets("claude");
        list.push(serde_json::from_value(json!({"id": "d2", "type": "desktop"})).unwrap());
        cfg.set_tool_targets("claude", &list);
        assert!(save_config(&p, &cfg).is_err());
        list.pop();
        list.push(serde_json::from_value(json!({"id": "desktop", "type": "wxtest"})).unwrap());
        cfg.set_tool_targets("claude", &list);
        assert!(save_config(&p, &cfg).is_err());
    }

    #[test]
    fn redact_and_restore_secrets() {
        let mut cfg = default_config();
        let mut list = cfg.tool_targets("claude");
        list.push(serde_json::from_value(json!({"id": "fs", "type": "feishu", "webhook": "https://x/hook", "secret": "s3cret"})).unwrap());
        cfg.set_tool_targets("claude", &list);
        let shown = redact(&cfg).tool_targets("claude");
        assert_eq!(shown[2].str("secret"), MASK);
        assert_eq!(shown[2].str("webhook"), MASK);
        assert_eq!(shown[0].str("secret"), "", "没填的字段不脱敏");
        let mut edited = shown[2].clone();
        edited.name = "改名".into();
        let restored = restore_secrets(&edited, &cfg.tool_targets("claude"));
        assert_eq!((restored.name.as_str(), restored.str("webhook"), restored.str("secret")), ("改名", "https://x/hook", "s3cret"));
        let orphan = restore_secrets(&Target { id: "new".into(), ..edited }, &cfg.tool_targets("claude"));
        assert_eq!(orphan.str("secret"), "", "找不到同 id 时 MASK 置空");
    }

    #[test]
    fn unsupported_targets_are_ignored_without_rewriting_file() {
        let (_d, p) = temp_paths();
        let mut raw = serde_json::to_value(default_config()).unwrap();
        for tool in TOOLS {
            raw["tools"][tool]["targets"].as_array_mut().unwrap()
                .push(json!({"id": "old", "type": "retired", "secret": "old-secret"}));
        }
        atomic_write_json(&p.config(), &raw).unwrap();
        let original = fs::read(&p.config()).unwrap();
        let cfg = load_config(&p).unwrap();
        assert_eq!(cfg, default_config());
        assert_eq!(fs::read(&p.config()).unwrap(), original);
        save_config(&p, &cfg).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(p.config()).unwrap()).unwrap();
        assert_eq!(saved, serde_json::to_value(default_config()).unwrap());
    }

    #[test]
    fn unsupported_target_cannot_be_saved() {
        let (_d, p) = temp_paths();
        let mut cfg = default_config();
        save_config(&p, &cfg).unwrap();
        let original = fs::read(p.config()).unwrap();
        cfg.set_tool_targets("claude", &[serde_json::from_value(json!({"id": "old", "type": "retired"})).unwrap()]);
        assert!(save_config(&p, &cfg).unwrap_err().to_string().contains("不支持的提醒方式"));
        assert_eq!(fs::read(p.config()).unwrap(), original);
    }

    #[test]
    fn malformed_config_never_becomes_an_empty_config() {
        let mut bad_card = serde_json::to_value(default_config()).unwrap();
        bad_card["tools"]["claude"]["targets"].as_array_mut().unwrap()
            .push(json!({"id": "bad", "type": "wxtest", "events": "task_complete"}));
        let mut bad_tool = serde_json::to_value(default_config()).unwrap();
        bad_tool["tools"]["claude"] = json!({"targets": "invalid"});
        for raw in [bad_card, bad_tool, json!({"tools": null}), json!({}),
            json!({"tools": {}, "templates": []}), json!({"targets": []}), json!(null)] {
            let (_d, p) = temp_paths();
            atomic_write_json(&p.config(), &raw).unwrap();
            let before = fs::read(p.config()).unwrap();
            assert_eq!(load_config(&p).unwrap_err().kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read(p.config()).unwrap(), before);
            assert!(!p.backups().exists(), "读取不应写入备份");
        }
    }

    #[test]
    fn target_accessors() {
        let t: Target = serde_json::from_value(json!({"id": "w", "type": "wxtest", "delay_minutes": "5", "batch": false})).unwrap();
        assert_eq!(t.u64("delay_minutes"), 5);
        assert!(!t.bool_or("batch", true));
        assert!(t.bool_or("missing", true));
        assert!(t.enabled, "enabled 缺省为 true");
    }

    #[test]
    fn pets_are_independent_and_invalid_pet_config_never_overwrites_saved_data() {
        let (_d, p) = temp_paths();
        let mut cfg = default_config();
        cfg.pets.push(serde_json::from_value(json!({"id":"pet-1","number":1,"sources":[],"events":[]})).unwrap());
        save_config(&p, &cfg).unwrap();
        assert_eq!(load_config(&p).unwrap(), cfg);
        let original = fs::read(p.config()).unwrap();
        cfg.pets[0].id = "../unsafe".into();
        assert_eq!(save_config(&p, &cfg).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read(p.config()).unwrap(), original);
        atomic_write_json(&p.config(), &cfg).unwrap();
        let invalid = fs::read(p.config()).unwrap();
        assert_eq!(load_config(&p).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(p.config()).unwrap(), invalid);
        let card: Target = serde_json::from_value(json!({"id":"pet-1","type":"pet"})).unwrap();
        assert!(validate_targets(&[card]).is_err(), "桌宠不再属于工具提醒卡片");
    }
}
