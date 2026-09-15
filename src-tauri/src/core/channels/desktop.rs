//! 屏幕弹窗：通过本地 socket 交给运行中的 AgentPulse App 显示（悬浮窗 / 系统通知）。
//! App 没在运行时只启动后台通知模式，不打开主界面或桌宠；仍连不上则报错。
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{Channel, Message};
use crate::core::config::Target;
use crate::core::paths::{FileLock, Paths};
use crate::core::platform::agent_app_bundle;

pub const APP_BUNDLE_ID: &str = "com.agentpulse.desktop";
pub const BACKGROUND_ARG: &str = "--background-notifications";

pub struct Desktop {
    pub paths: Paths,
}

/// 连上 App 的本地 socket；连不上时（如果允许）先尝试拉起 App 再重试，超时后报错。
#[cfg(unix)]
fn connect(paths: &Paths, launch_if_needed: bool) -> Result<std::os::unix::net::UnixStream, String> {
    connect_with_launcher(paths, launch_if_needed, || {
        #[cfg(target_os = "macos")]
        {
            // Hook 的 stdout 留给来源工具；启动 App 不能混入任何输出。
            let mut command = std::process::Command::new("open");
            command.arg("-g");
            let bundle = std::env::current_exe().ok().and_then(|exe| {
                exe.ancestors().find(|p| p.extension().is_some_and(|ext| ext == "app")).map(std::path::Path::to_path_buf)
            });
            // 优先唤醒当前 Hook 所在的 App，避免 Launch Services 选中旧构建或备份。
            if let Some(bundle) = bundle { command.arg("-a").arg(bundle); }
            else { command.args(["-b", APP_BUNDLE_ID]); }
            command.args(["--args", BACKGROUND_ARG])
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()
                .map_err(|e| e.to_string())?
                .success().then_some(()).ok_or_else(|| "无法启动 AgentPulse 后台通知进程".to_string())?;
        }
        Ok(())
    })
}

/// 并发 Hook 共用启动锁，避免 App 初始化期间反复 open 导致 Reopen 打开主界面。
#[cfg(unix)]
fn connect_with_launcher(paths: &Paths, launch_if_needed: bool, launch: impl FnOnce() -> Result<(), String>) -> Result<std::os::unix::net::UnixStream, String> {
    use std::os::unix::net::UnixStream;
    let sock = paths.data_dir.join("app.sock");
    let mut launch = Some(launch);
    let mut launch_lock = None;
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        match UnixStream::connect(&sock) {
            Ok(stream) => return Ok(stream),
            Err(_) if launch_if_needed && Instant::now() < deadline => {
                if launch_lock.is_none() {
                    launch_lock = FileLock::try_acquire(&paths.data_dir.join("app-launch.lock")).map_err(|e| e.to_string())?;
                    if launch_lock.is_some() {
                        // 等待锁时另一个 Hook 可能已经启动了 App。
                        if let Ok(stream) = UnixStream::connect(&sock) { return Ok(stream); }
                        if let Some(start) = launch.take() { start()?; }
                    }
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            Err(e) => return Err(format!("RuntimeError: AgentPulse 没有运行，无法弹窗（{e}）")),
        }
    }
}
#[cfg(not(unix))]
fn connect(_paths: &Paths, _launch_if_needed: bool) -> Result<std::net::TcpStream, String> {
    Err("当前平台暂不支持".into())
}

/// 向 App 发一条请求，返回 App 的回复（JSON）。5 秒读超时。
pub fn send_to_app(paths: &Paths, req: &Value, launch_if_needed: bool) -> Result<Value, String> {
    let mut stream = connect(paths, launch_if_needed)?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    stream.write_all(req.to_string().as_bytes()).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(line.trim()).map_err(|e| format!("App 回复不是有效 JSON：{e}"))?;
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!("RuntimeError: {}", v.get("error").and_then(Value::as_str).unwrap_or("App 未确认发送成功")));
    }
    Ok(v)
}

impl Channel for Desktop {
    fn send(&self, target: &Target, message: &Message) -> Result<Option<String>, String> {
        let source_host = if !message.ctx.host_bundle.is_empty() {
            message.ctx.host_bundle.as_str()
        } else {
            agent_app_bundle(&message.agent)
        };
        // 点击动作与来源分开：只关闭的卡片也能在用户手动回到来源时一起清除。
        let host = if target.str("click") == "close" { "" } else { source_host };
        let mode = match target.str("mode") { "" => "system", m => m };
        let req = json!({"kind": "notify", "mode": mode, "title": message.title, "body": if message.body.is_empty() { " " } else { &message.body },
            "host": host, "source_host": source_host, "event": message.event, "agent": message.agent});
        send_to_app(&self.paths, &req, true)?;
        Ok(None)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::core::paths::temp_paths;
    use std::os::unix::net::UnixListener;

    fn accept_notify(paths: &Paths) -> std::thread::JoinHandle<Value> {
        let listener = UnixListener::bind(paths.data_dir.join("app.sock")).unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            conn.write_all(b"{\"ok\":true}\n").unwrap();
            serde_json::from_str(line.trim()).unwrap()
        })
    }

    #[test]
    fn concurrent_requests_only_launch_one_background_app() {
        use std::sync::{Arc, Barrier, Mutex, atomic::{AtomicUsize, Ordering}};
        let (_d, paths) = temp_paths();
        let count = Arc::new(AtomicUsize::new(0));
        let listener = Arc::new(Mutex::new(None));
        let barrier = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8).map(|_| {
            let (paths, count, listener, barrier) = (paths.clone(), count.clone(), listener.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                connect_with_launcher(&paths, true, || {
                    count.fetch_add(1, Ordering::SeqCst);
                    // 覆盖已请求启动、IPC 尚未就绪的窗口，不能对同一 App 再发送 open。
                    std::thread::sleep(Duration::from_millis(50));
                    *listener.lock().unwrap() = Some(UnixListener::bind(paths.data_dir.join("app.sock")).unwrap());
                    Ok(())
                }).unwrap()
            })
        }).collect();
        let _connections: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn existing_app_and_no_launch_requests_never_call_launcher() {
        let (_d, paths) = temp_paths();
        assert!(connect_with_launcher(&paths, false, || panic!("must not launch")).is_err());
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let _listener = UnixListener::bind(paths.data_dir.join("app.sock")).unwrap();
        assert!(connect_with_launcher(&paths, true, || panic!("already running")).is_ok());
    }

    #[test]
    fn ipc_requires_explicit_success_reply() {
        for reply in ["", "not-json\n", "{}\n", "{\"ok\":null}\n", "{\"ok\":\"true\"}\n", "{\"ok\":false,\"error\":\"failed\"}\n", "{\"ok\":true}\n"] {
            let (_d, paths) = temp_paths();
            std::fs::create_dir_all(&paths.data_dir).unwrap();
            let listener = UnixListener::bind(paths.data_dir.join("app.sock")).unwrap();
            let handle = std::thread::spawn(move || {
                let (mut conn, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&conn).read_line(&mut line).unwrap();
                conn.write_all(reply.as_bytes()).unwrap();
            });
            let result = send_to_app(&paths, &json!({"kind": "notify"}), false);
            handle.join().unwrap();
            assert_eq!(result.is_ok(), reply == "{\"ok\":true}\n", "reply: {reply:?}");
        }
    }

    #[test]
    fn send_defaults_to_system_mode_when_unset() {
        let (_d, paths) = temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let handle = accept_notify(&paths);
        let desktop = Desktop { paths };
        let target: Target = serde_json::from_value(json!({"id": "d", "type": "desktop"})).unwrap();
        let message = Message { event: "task_complete".into(), title: "T".into(), body: "B".into(), agent: "claude".into(), ..Default::default() };
        assert_eq!(desktop.send(&target, &message).unwrap(), None);
        let req = handle.join().unwrap();
        assert_eq!(req["kind"], "notify");
        assert_eq!(req["mode"], "system");
        assert_eq!(req["agent"], "claude", "悬浮窗要按这个字段找来源工具的自定义图标");
    }

    #[test]
    fn send_passes_explicit_mode_through() {
        let (_d, paths) = temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let handle = accept_notify(&paths);
        let desktop = Desktop { paths };
        let target: Target = serde_json::from_value(json!({"id": "d", "type": "desktop", "mode": "overlay"})).unwrap();
        let message = Message { event: "task_complete".into(), title: "T".into(), body: "B".into(), agent: "claude".into(), ..Default::default() };
        desktop.send(&target, &message).unwrap();
        assert_eq!(handle.join().unwrap()["mode"], "overlay");
    }

    #[test]
    fn send_clears_host_when_click_is_close() {
        let (_d, paths) = temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let handle = accept_notify(&paths);
        let desktop = Desktop { paths };
        let target: Target = serde_json::from_value(json!({"id": "d", "type": "desktop", "click": "close"})).unwrap();
        let mut message = Message { event: "task_complete".into(), title: "T".into(), body: "B".into(), agent: "claude".into(), ..Default::default() };
        message.ctx.host_bundle = "com.anthropic.claudefordesktop".into();
        desktop.send(&target, &message).unwrap();
        let req = handle.join().unwrap();
        assert_eq!(req["host"], "");
        assert_eq!(req["source_host"], "com.anthropic.claudefordesktop");
    }
}
