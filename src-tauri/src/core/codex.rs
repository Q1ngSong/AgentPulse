//! Codex 接入信任：展示 Codex 自己计算的 Hook 指纹，确认后只保存这些指纹。
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(not(windows))]
use std::process::ChildStdout;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::events::{ACTIVITY_HOOKS, HOOK_EVENTS};
use super::paths::{backup_private, Paths};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Hook {
    pub key: String,
    pub hash: String,
    pub event: String,
    pub command: String,
    pub trusted: bool,
}

#[derive(Debug, Serialize)]
pub struct Review {
    pub hooks: Vec<Hook>,
}

/// 单次接入操作拥有一个临时 app-server；读写共用 deadline，退出时清理整个进程组。
struct Rpc {
    child: Child,
    input: Option<ChildStdin>,
    #[cfg(not(windows))]
    output: Option<ChildStdout>,
    #[cfg(windows)]
    output_rx: Option<std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>>,
    buffer: Vec<u8>,
    next_id: u64,
    deadline: Instant,
}

#[cfg(unix)]
fn nonblocking<T: std::os::fd::AsRawFd>(pipe: &T) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    // 只调整自己创建的管道，确保 Codex 无响应时写入和读取都能遵守总超时。
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(format!("无法设置 Codex 通信超时：{}", std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(windows)]
fn nonblocking<T>(_: &T) -> Result<(), String> {
    // Windows anonymous pipes do not support the Unix fcntl mode used above.
    // The reader thread below provides the same deadline without blocking Rpc.
    Ok(())
}

#[cfg(all(not(unix), not(windows)))]
fn nonblocking<T>(_: &T) -> Result<(), String> {
    Err("当前平台暂不支持 Codex Hook 信任接入".into())
}

impl Rpc {
    fn start(binary: &Path, home: &Path, timeout: Duration) -> Result<Self, String> {
        let deadline = Instant::now() + timeout;
        let mut command = Command::new(binary);
        command.args(["app-server", "--stdio"])
            .env("CODEX_HOME", home.join(".codex"))
            .current_dir(home)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // npm 的 codex 包装器会派生原生程序；单独一组可一起清理，避免留下 app-server。
            command.process_group(0);
        }
        let child = command.spawn().map_err(|e| format!("无法启动 Codex：{e}"))?;
        // 先交给 Drop 管理，再进行任何可能失败的初始化。
        let mut rpc = Self {
            child,
            input: None,
            #[cfg(not(windows))]
            output: None,
            #[cfg(windows)]
            output_rx: None,
            buffer: Vec::new(), next_id: 0, deadline,
        };
        rpc.input = rpc.child.stdin.take();
        #[cfg(not(windows))]
        { rpc.output = rpc.child.stdout.take(); }
        #[cfg(windows)]
        {
            let mut output = rpc.child.stdout.take().ok_or("Codex 输出管道不可用")?;
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                loop {
                    let mut chunk = [0; 8192];
                    match output.read(&mut chunk) {
                        Ok(0) => { let _ = tx.send(Ok(Vec::new())); break; }
                        Ok(n) => { if tx.send(Ok(chunk[..n].to_vec())).is_err() { break; } }
                        Err(e) => { let _ = tx.send(Err(e)); break; }
                    }
                }
            });
            rpc.output_rx = Some(rx);
        }
        nonblocking(rpc.input.as_ref().ok_or("Codex 输入管道不可用")?)?;
        #[cfg(not(windows))]
        nonblocking(rpc.output.as_ref().ok_or("Codex 输出管道不可用")?)?;
        rpc.call("initialize", json!({
            "clientInfo": {"name": "agentpulse", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }))?;
        rpc.send(&json!({"method": "initialized"}))?;
        Ok(rpc)
    }

    fn check_deadline(&self) -> Result<(), String> {
        if Instant::now() >= self.deadline {
            return Err("Codex 响应超时，请稍后重新检查接入状态".into());
        }
        Ok(())
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let mut written = 0;
        while written < bytes.len() {
            self.check_deadline()?;
            match self.input.as_mut().ok_or("Codex 输入已关闭")?.write(&bytes[written..]) {
                Ok(0) => return Err("Codex 输入已关闭".into()),
                Ok(n) => written += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {},
                Err(e) => return Err(format!("无法向 Codex 发送请求：{e}")),
            }
        }
        Ok(())
    }

    fn receive(&mut self) -> Result<Value, String> {
        loop {
            self.check_deadline()?;
            if let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<_> = self.buffer.drain(..=end).collect();
                if line.iter().all(u8::is_ascii_whitespace) { continue; }
                return serde_json::from_slice(&line).map_err(|_| "Codex 返回了无法解析的响应".into());
            }
            #[cfg(not(windows))]
            {
                let mut chunk = [0; 8192];
                match self.output.as_mut().ok_or("Codex 输出已关闭")?.read(&mut chunk) {
                    Ok(0) => return Err("Codex 已退出，无法读取 Hook 信任状态；请确认版本支持 Hook 接入".into()),
                    Ok(n) => {
                        self.buffer.extend_from_slice(&chunk[..n]);
                        if self.buffer.len() > 8 * 1024 * 1024 { return Err("Codex 响应过大，已停止读取".into()); }
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {},
                    Err(e) => return Err(format!("无法读取 Codex 响应：{e}")),
                }
            }
            #[cfg(windows)]
            {
                // 后台线程持有阻塞式的 ChildStdout；用剩余超时时间做阻塞 recv，避免忙轮询。
                let remaining = self.deadline.saturating_duration_since(Instant::now());
                match self.output_rx.as_ref().ok_or("Codex 输出已关闭")?.recv_timeout(remaining) {
                    Ok(Ok(chunk)) if chunk.is_empty() =>
                        return Err("Codex 已退出，无法读取 Hook 信任状态；请确认版本支持 Hook 接入".into()),
                    Ok(Ok(chunk)) => {
                        self.buffer.extend_from_slice(&chunk);
                        if self.buffer.len() > 8 * 1024 * 1024 { return Err("Codex 响应过大，已停止读取".into()); }
                    },
                    Ok(Err(e)) => return Err(format!("无法读取 Codex 响应：{e}")),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {},
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) =>
                        return Err("Codex 已退出，无法读取 Hook 信任状态；请确认版本支持 Hook 接入".into()),
                }
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"id": id, "method": method, "params": params}))?;
        loop {
            let response = self.receive()?;
            if response.get("id").and_then(Value::as_u64) != Some(id) { continue; }
            if let Some(error) = response.get("error") {
                return Err(format!("Codex {method} 失败：{}", error.get("message").and_then(Value::as_str).unwrap_or("未知错误")));
            }
            return response.get("result").cloned().ok_or_else(|| format!("Codex {method} 响应缺少结果"));
        }
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            // process_group(0) 将组 ID 设置为本次 Child 的 PID，绝不使用调用者所在的组。
            libc::killpg(self.child.id() as libc::pid_t, libc::SIGKILL);
        }
        let _ = self.child.kill();
        self.input.take();
        #[cfg(not(windows))]
        self.output.take();
        let _ = self.child.wait();
    }
}

fn same_path(actual: &str, expected: &Path) -> bool {
    let actual = Path::new(actual);
    actual == expected || matches!((actual.canonicalize(), expected.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

fn nonempty<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value.get(field).and_then(Value::as_str).filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Codex Hook 信息缺少 {field}，无法确认信任范围"))
}

/// 只接受用户目录里本次安装的精确命令，不信任项目、插件、托管或其他命令。
fn parse_hooks(result: Value, home: &Path, command: &str) -> Result<Review, String> {
    let data = result.get("data").and_then(Value::as_array).ok_or("Codex 未返回 Hook 列表")?;
    let source = home.join(".codex/hooks.json");
    let mut hooks = Vec::new();
    for entry in data {
        if entry.get("errors").and_then(Value::as_array).is_some_and(|e| !e.is_empty()) {
            return Err("Codex 读取 Hook 配置失败，请先修复 hooks.json 后重新接入".into());
        }
        for hook in entry.get("hooks").and_then(Value::as_array).ok_or("Codex Hook 列表格式不受支持")? {
            let event = hook.get("eventName").and_then(Value::as_str).unwrap_or("");
            if hook.get("command").and_then(Value::as_str) != Some(command)
                || hook.get("source").and_then(Value::as_str) != Some("user")
                || hook.get("handlerType").and_then(Value::as_str) != Some("command")
                || hook.get("isManaged").and_then(Value::as_bool) != Some(false)
                || !hook.get("sourcePath").and_then(Value::as_str).is_some_and(|p| same_path(p, &source))
                || !HOOK_EVENTS.iter().chain(ACTIVITY_HOOKS.iter()).any(|(_, name)| {
                    // hooks/list 的 eventName 使用 lowerCamel，hooks.json 使用 PascalCase。
                    let mut api_name = name.to_string();
                    api_name[..1].make_ascii_lowercase();
                    api_name == event
                }) {
                continue;
            }
            if hook.get("enabled").and_then(Value::as_bool) != Some(true) {
                return Err(format!("AgentPulse 的 {event} Hook 已被禁用，请在 Codex 中启用后重新接入"));
            }
            let trusted = match hook.get("trustStatus").and_then(Value::as_str) {
                Some("trusted") => true,
                Some("untrusted" | "modified") => false,
                _ => return Err("Codex Hook 信任状态不受支持，请更新 Codex 后重试".into()),
            };
            hooks.push(Hook { key: nonempty(hook, "key")?.into(), hash: nonempty(hook, "currentHash")?.into(),
                event: event.into(), command: command.into(), trusted });
        }
    }
    if hooks.is_empty() { return Err("Codex 未找到当前 AgentPulse 的 Hook，请先接入后重新检查".into()); }
    hooks.sort_by(|a, b| a.key.cmp(&b.key));
    if hooks.windows(2).any(|pair| pair[0].key == pair[1].key) {
        return Err("Codex 返回了重复的 Hook 标识，请重新检查接入配置".into());
    }
    Ok(Review { hooks })
}

fn list(rpc: &mut impl FnMut(&str, Value) -> Result<Value, String>, home: &Path, command: &str) -> Result<Review, String> {
    parse_hooks(rpc("hooks/list", json!({"cwds": [home]}))?, home, command)
}

/// 信任状态变化不扩大授权；命令、事件、标识、内容指纹或集合变化必须重新展示。
fn same_approval(current: &[Hook], approved: &[Hook]) -> bool {
    let identity = |hooks: &[Hook]| {
        let mut hooks = hooks.to_vec();
        for hook in &mut hooks { hook.trusted = false; }
        hooks.sort_by(|a, b| a.key.cmp(&b.key));
        hooks
    };
    identity(current) == identity(approved)
}

pub fn review(binary: &Path, home: &Path, command: &str) -> Result<Review, String> {
    let mut rpc = Rpc::start(binary, home, Duration::from_secs(15))?;
    list(&mut |method, params| rpc.call(method, params), home, command)
}

pub fn trust(binary: &Path, home: &Path, paths: &Paths, command: &str, approved: &[Hook]) -> Result<Review, String> {
    let mut rpc = Rpc::start(binary, home, Duration::from_secs(15))?;
    trust_with(&mut |method, params| rpc.call(method, params), home, paths, command, approved)
}

fn trust_with(rpc: &mut impl FnMut(&str, Value) -> Result<Value, String>, home: &Path, paths: &Paths,
    command: &str, approved: &[Hook]) -> Result<Review, String> {
    let current = list(rpc, home, command)?;
    if !same_approval(&current.hooks, approved) {
        return Err("AgentPulse Hook 已在确认期间发生变化，请重新检查并确认信任".into());
    }
    // 弹窗只展示待信任条目，已受信任项若期间被撤销，不能借旧确认重新授信。
    if current.hooks.iter().any(|hook| !hook.trusted && approved.iter().any(|old| old.key == hook.key && old.trusted)) {
        return Err("AgentPulse Hook 的信任已在确认期间被撤销，请重新检查并确认信任".into());
    }
    let pending: Vec<_> = current.hooks.iter().filter(|hook| !hook.trusted).collect();
    if pending.is_empty() { return Ok(current); }
    let config = rpc("config/read", json!({"includeLayers": true, "cwd": home}))?;
    let user_file = home.join(".codex/config.toml");
    let layers = config.get("layers").and_then(Value::as_array).ok_or("Codex 未返回用户配置版本，无法安全保存信任")?;
    let layer = layers.iter().find(|layer| layer["name"]["type"] == "user"
        && layer["name"]["file"].as_str().is_some_and(|file| same_path(file, &user_file)))
        .ok_or("Codex 用户配置位置不匹配，未保存信任")?;
    let version = nonempty(layer, "version")?;
    // Codex 负责保留其他 TOML 内容及并发版本检查；我们保留写前原始字节以供恢复。
    match fs::metadata(&user_file) {
        Ok(_) => {
            let dest = paths.backups().join(format!("codex-config.toml.{}.bak", uuid::Uuid::new_v4()));
            backup_private(&user_file, &dest).map_err(|e| format!("备份 Codex 配置失败，未保存信任：{e}"))?;
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => return Err(format!("读取 Codex 配置失败，未保存信任：{e}")),
    }
    let edits: Vec<_> = pending.iter().map(|hook| json!({
        "keyPath": format!("hooks.state.{}.trusted_hash", serde_json::to_string(&hook.key).expect("string serialization")),
        "value": hook.hash,
        "mergeStrategy": "replace"
    })).collect();
    rpc("config/batchWrite", json!({"filePath": user_file, "expectedVersion": version,
        "edits": edits, "reloadUserConfig": true}))?;
    let verified = list(rpc, home, command)?;
    if !same_approval(&verified.hooks, approved) || verified.hooks.iter().any(|hook| !hook.trusted) {
        return Err("Codex 未确认全部 AgentPulse Hook 已受信任，请重新检查接入状态".into());
    }
    Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const COMMAND: &str = "'/Applications/AgentPulse.app/Contents/MacOS/agentpulse-hook' codex";
    const ORIGINAL: &str = "# keep my configuration\nmodel = \"test\"\n";

    fn setup() -> (tempfile::TempDir, PathBuf, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(home.join(".codex/config.toml"), ORIGINAL).unwrap();
        fs::write(home.join(".codex/hooks.json"), "{}").unwrap();
        let paths = Paths::new(dir.path().join("agentpulse"));
        (dir, home, paths)
    }

    fn hook(home: &Path, key: &str, event: &str, trusted: bool) -> Value {
        json!({"key": key, "eventName": event, "command": COMMAND, "handlerType": "command",
            "source": "user", "sourcePath": home.join(".codex/hooks.json"), "enabled": true,
            "isManaged": false, "currentHash": "authoritative-hash", "trustStatus": if trusted { "trusted" } else { "untrusted" }})
    }

    fn listing(hooks: Vec<Value>) -> Value {
        json!({"data": [{"hooks": hooks, "errors": [], "warnings": ["untrusted hooks do not run"]}]})
    }

    fn config(home: &Path) -> Value {
        json!({"layers": [{"name": {"type": "user", "file": home.join(".codex/config.toml")}, "version": "sha256:config-version"}]})
    }

    #[test]
    fn review_accepts_all_current_codex_api_event_names() {
        let (_dir, home, _paths) = setup();
        let events = ["permissionRequest", "stop", "userPromptSubmit", "preToolUse", "postToolUse"];
        let hooks = events.iter().map(|event| hook(&home, event, event, false)).collect();
        let review = parse_hooks(listing(hooks), &home, COMMAND).unwrap();
        assert_eq!(review.hooks.len(), events.len());
        assert!(review.hooks.iter().all(|hook| events.contains(&hook.event.as_str())));
    }

    #[test]
    fn review_excludes_other_commands_sources_managed_and_unsupported_events() {
        let (_dir, home, _paths) = setup();
        let ours = hook(&home, "ours", "stop", false);
        let mut hooks = vec![ours.clone()];
        for (field, value) in [("command", json!(format!("{COMMAND} && echo extra"))), ("source", json!("project")),
            ("sourcePath", json!(home.join("project/hooks.json"))), ("handlerType", json!("prompt")),
            ("isManaged", json!(true)), ("eventName", json!("sessionStart"))] {
            let mut other = ours.clone();
            other[field] = value;
            hooks.push(other);
        }
        let review = parse_hooks(listing(hooks), &home, COMMAND).unwrap();
        assert_eq!(review.hooks.len(), 1);
        assert_eq!(review.hooks[0].key, "ours");
        assert_eq!(review.hooks[0].hash, "authoritative-hash");
    }

    #[test]
    fn review_rejects_errors_missing_disabled_or_duplicate_hooks() {
        let (_dir, home, _paths) = setup();
        assert!(parse_hooks(listing(vec![]), &home, COMMAND).is_err());
        let original = hook(&home, "ours", "stop", false);
        for (field, value) in [("enabled", json!(false)), ("currentHash", Value::Null), ("key", json!("")), ("trustStatus", json!("unknown"))] {
            let mut item = original.clone();
            item[field] = value;
            assert!(parse_hooks(listing(vec![item]), &home, COMMAND).is_err(), "{field}");
        }
        assert!(parse_hooks(listing(vec![original.clone(), original.clone()]), &home, COMMAND).is_err());
        let mut broken = listing(vec![original]);
        broken["data"][0]["errors"] = json!(["invalid JSON"]);
        assert!(parse_hooks(broken, &home, COMMAND).is_err());
    }

    #[test]
    fn changed_hash_command_event_key_or_set_never_writes() {
        let (_dir, home, paths) = setup();
        let original = hook(&home, "ours", "stop", false);
        let approved = parse_hooks(listing(vec![original.clone()]), &home, COMMAND).unwrap().hooks;
        let mut variants = Vec::new();
        for (field, value) in [("currentHash", "new-hash"), ("command", "different command"), ("eventName", "permissionRequest"), ("key", "new-key")] {
            let mut changed = original.clone();
            changed[field] = json!(value);
            variants.push(vec![changed]);
        }
        variants.push(vec![original, hook(&home, "new-hook", "permissionRequest", false)]);
        variants.push(vec![]);
        for hooks in variants {
            let result = trust_with(&mut |method, _| {
                assert_eq!(method, "hooks/list", "must reject before reading or writing config");
                Ok(listing(hooks.clone()))
            }, &home, &paths, COMMAND, &approved);
            assert!(result.is_err());
        }
        assert_eq!(fs::read_to_string(home.join(".codex/config.toml")).unwrap(), ORIGINAL);
        assert!(!paths.backups().exists());
    }

    #[test]
    #[cfg(unix)]
    fn writes_only_approved_pending_hash_after_private_backup_then_verifies() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, home, paths) = setup();
        let pending_key = "user:some.\"quoted\\key";
        let pending = hook(&home, pending_key, "stop", false);
        let already = hook(&home, "already", "permissionRequest", true);
        let mut other = hook(&home, "other", "stop", false);
        other["command"] = json!("another notification app");
        let approved = parse_hooks(listing(vec![pending.clone(), already.clone(), other.clone()]), &home, COMMAND).unwrap().hooks;
        let mut wrote = false;
        let mut calls = Vec::new();
        let result = trust_with(&mut |method, params| {
            calls.push(method.to_string());
            match method {
                "hooks/list" => {
                    let mut item = pending.clone();
                    if wrote { item["trustStatus"] = json!("trusted"); }
                    Ok(listing(vec![item, already.clone(), other.clone()]))
                },
                "config/read" => Ok(config(&home)),
                "config/batchWrite" => {
                    let backups: Vec<_> = fs::read_dir(paths.backups()).unwrap().map(Result::unwrap).collect();
                    assert_eq!(backups.len(), 1);
                    assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), ORIGINAL);
                    assert_eq!(fs::metadata(backups[0].path()).unwrap().permissions().mode() & 0o777, 0o600);
                    assert_eq!(params, json!({"filePath": home.join(".codex/config.toml"), "expectedVersion": "sha256:config-version",
                        "edits": [{"keyPath": format!("hooks.state.{}.trusted_hash", serde_json::to_string(pending_key).unwrap()),
                            "value": "authoritative-hash", "mergeStrategy": "replace"}], "reloadUserConfig": true}));
                    wrote = true;
                    Ok(json!({}))
                },
                _ => panic!("unexpected request: {method}"),
            }
        }, &home, &paths, COMMAND, &approved).unwrap();
        assert!(result.hooks.iter().all(|h| h.trusted));
        assert_eq!(calls, ["hooks/list", "config/read", "config/batchWrite", "hooks/list"]);
    }

    #[test]
    fn version_or_backup_failure_never_writes() {
        let (_dir, home, paths) = setup();
        let hooks = listing(vec![hook(&home, "ours", "stop", false)]);
        let approved = parse_hooks(hooks.clone(), &home, COMMAND).unwrap().hooks;
        for wrong_config in [json!({}), json!({"layers": []}), json!({"layers": [{"name": {"type": "user", "file": "/different/config.toml"}, "version": "x"}]})] {
            assert!(trust_with(&mut |method, _| match method {
                "hooks/list" => Ok(hooks.clone()), "config/read" => Ok(wrong_config.clone()), _ => panic!("must not write"),
            }, &home, &paths, COMMAND, &approved).is_err());
        }
        fs::create_dir_all(&paths.data_dir).unwrap();
        fs::write(paths.backups(), "not a directory").unwrap();
        assert!(trust_with(&mut |method, _| match method {
            "hooks/list" => Ok(hooks.clone()), "config/read" => Ok(config(&home)), _ => panic!("must not write"),
        }, &home, &paths, COMMAND, &approved).unwrap_err().contains("备份"));
        assert_eq!(fs::read_to_string(home.join(".codex/config.toml")).unwrap(), ORIGINAL);
    }

    #[test]
    fn already_trusted_is_read_only_and_unverified_write_is_error() {
        let (_dir, home, paths) = setup();
        let trusted = listing(vec![hook(&home, "ours", "stop", true)]);
        let approved = parse_hooks(trusted.clone(), &home, COMMAND).unwrap().hooks;
        trust_with(&mut |method, _| { assert_eq!(method, "hooks/list"); Ok(trusted.clone()) }, &home, &paths, COMMAND, &approved).unwrap();
        assert!(!paths.backups().exists());
        let untrusted = listing(vec![hook(&home, "ours", "stop", false)]);
        let approved = parse_hooks(untrusted.clone(), &home, COMMAND).unwrap().hooks;
        let result = trust_with(&mut |method, _| match method {
            "hooks/list" => Ok(untrusted.clone()), "config/read" => Ok(config(&home)), "config/batchWrite" => Ok(json!({})), _ => unreachable!(),
        }, &home, &paths, COMMAND, &approved);
        assert!(result.unwrap_err().contains("未确认"));
    }

    #[test]
    fn revoked_trust_requires_review_but_newly_trusted_items_need_no_write() {
        let (_dir, home, paths) = setup();
        for (was_trusted, now_trusted) in [(true, false), (false, true)] {
            let approved = parse_hooks(listing(vec![hook(&home, "ours", "stop", was_trusted)]), &home, COMMAND).unwrap().hooks;
            let result = trust_with(&mut |method, _| {
                assert_eq!(method, "hooks/list", "must not read or write config");
                Ok(listing(vec![hook(&home, "ours", "stop", now_trusted)]))
            }, &home, &paths, COMMAND, &approved);
            if was_trusted {
                assert!(result.unwrap_err().contains("撤销"));
            } else {
                assert!(result.unwrap().hooks[0].trusted);
            }
        }
        assert!(!paths.backups().exists());
        assert_eq!(fs::read_to_string(home.join(".codex/config.toml")).unwrap(), ORIGINAL);
    }

    #[cfg(unix)]
    fn script(home: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = home.join("fake-codex");
        fs::write(&path, format!("#!/bin/sh\necho $$ > child.pid\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    fn assert_reaped(home: &Path) {
        let pid = fs::read_to_string(home.join("child.pid")).unwrap().trim().parse::<i32>().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "temporary Codex process is still running");
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[test]
    #[cfg(unix)]
    fn rpc_timeout_kills_and_reaps_its_child() {
        let (_dir, home, _paths) = setup();
        let binary = script(&home, "read -r initialize\necho '{\"id\":1,\"result\":{}}'\nexec /bin/sleep 30");
        let started = Instant::now();
        let mut rpc = Rpc::start(&binary, &home, Duration::from_secs(2)).unwrap();
        let error = rpc.call("test", json!({})).unwrap_err();
        drop(rpc);
        assert!(error.contains("超时"));
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_reaped(&home);
    }

    #[test]
    #[cfg(unix)]
    fn rpc_skips_notifications_and_cleans_up_after_success() {
        let (_dir, home, _paths) = setup();
        let binary = script(&home, r#"
read -r initialize
echo '{"method":"status","params":{}}'
echo '{"id":1,"result":{}}'
read -r initialized
read -r request
echo '{"id":2,"result":{"ok":true}}'
exec /bin/sleep 30
"#);
        let mut rpc = Rpc::start(&binary, &home, Duration::from_secs(2)).unwrap();
        assert_eq!(rpc.call("test", json!({})).unwrap(), json!({"ok": true}));
        drop(rpc);
        assert_reaped(&home);
    }

    #[test]
    #[cfg(unix)]
    fn rpc_timeout_also_stops_a_wrappers_child_process() {
        let (_dir, home, _paths) = setup();
        let binary = script(&home, r#"
/bin/sleep 30 &
echo $! > descendant.pid
read -r initialize
echo '{"id":1,"result":{}}'
wait
"#);
        let mut rpc = Rpc::start(&binary, &home, Duration::from_secs(2)).unwrap();
        let descendant = fs::read_to_string(home.join("descendant.pid")).unwrap().trim().parse::<i32>().unwrap();
        assert_eq!(unsafe { libc::getpgid(descendant) }, rpc.child.id() as i32);
        assert!(rpc.call("test", json!({})).unwrap_err().contains("超时"));
        drop(rpc);
        assert_reaped(&home);
        // 包装器一同退出后，由系统回收其子进程，留出短暂调度时间。
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(descendant, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let stopped = unsafe { libc::kill(descendant, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if !stopped {
            // 即使回归导致断言失败，也不遗留本测试创建的 sleep。
            unsafe { libc::kill(descendant, libc::SIGKILL); }
        }
        assert!(stopped, "wrapper descendant survived RPC timeout");
    }
}
