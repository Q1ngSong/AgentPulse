//! Hook → App 的本地 socket：<数据目录>/app.sock，一行 JSON 请求、一行 JSON 回复。
//! 请求 {"kind":"notify","mode":"overlay|system|both","title","body","host","agent"}。
//!
//! 路径只有一条，同时开着两份 AgentPulse 时最新启动的那份占着它（start 照旧 remove + bind）。
//! 退出时只删 inode 对得上自己的文件，免得先退出的那份把还在服务的那份的 socket 删掉；
//! accept 循环每秒回头看一眼路径：没了就重新 bind，被顶掉了就 ping 一下——对方活着就安静让位
//! （等它退出再接回来），没人应答就是崩溃留下的陈旧文件，照样 bind 回来。
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::core::paths::Paths;

/// 我们自己 bind 出来的 app.sock 的 inode；0 = 我们手上没有 socket（bind 失败，或者刚丢了路径还没重新 bind 上）。
/// 被更新的实例顶掉时这里仍然是我们那个旧 inode——正因如此 shutdown 会发现路径上的 inode 对不上、什么都不删。
/// 退出时只删 inode 对得上的文件：别的 AgentPulse 实例的 socket 不能碰。
#[cfg(unix)]
static OWN_INODE: AtomicU64 = AtomicU64::new(0);

/// 每隔这么久看一眼 app.sock 还在不在、还是不是我们的。
#[cfg(unix)]
const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// 路径上那个 socket 文件的 inode；路径不存在、或者上面的东西不是 socket，都算没有。
#[cfg(unix)]
fn inode_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let meta = std::fs::metadata(path).ok()?;
    if !meta.file_type().is_socket() {
        return None;
    }
    Some(meta.ino())
}

/// 删掉路径上原来的东西再 bind，顺带记下这次 bind 出来的 inode。
/// 数据目录每次都建一遍：目录一时不在（还没建、被人挪走）不该让监听永久失败。
#[cfg(unix)]
fn bind(sock: &Path) -> std::io::Result<(UnixListener, u64)> {
    if let Some(dir) = sock.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(sock);
    let listener = UnixListener::bind(sock)?;
    crate::core::paths::chmod_private(sock);
    let ino = inode_of(sock)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "刚 bind 出来的 socket 就不见了"))?;
    Ok((listener, ino))
}

/// 只删我们自己 bind 出来的那个文件。更新的实例把路径顶掉之后 inode 就对不上了，
/// 这时候还删就会把正在服务的那份 AgentPulse 的 socket 删掉——那正是这套机制要防的事。
#[cfg(unix)]
fn remove_if_ours(sock: &Path, own_ino: u64) {
    if own_ino != 0 && inode_of(sock) == Some(own_ino) {
        let _ = std::fs::remove_file(sock);
    }
}

/// App 退出时调用：收掉自己的 app.sock，别人的不动。
pub fn shutdown(paths: &Paths) {
    #[cfg(unix)]
    remove_if_ours(&paths.data_dir.join("app.sock"), OWN_INODE.load(Ordering::Relaxed));
    #[cfg(not(unix))]
    let _ = paths;
}

/// 路径上的 socket 还有人应答吗？发一条 ping，等一行带 "ok": true 的回复。
/// 连不上（对方已经死了、文件是陈旧的）、超时、回复不对，都算没人。
#[cfg(unix)]
fn someone_alive_at(sock: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(sock) else { return false };
    let t = Some(Duration::from_secs(1));
    if stream.set_read_timeout(t).is_err() || stream.set_write_timeout(t).is_err() {
        return false;
    }
    if stream.write_all(b"{\"kind\":\"ping\"}\n").is_err() {
        return false;
    }
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return false;
    }
    serde_json::from_str::<Value>(line.trim()).ok().and_then(|v| v.get("ok").and_then(Value::as_bool)) == Some(true)
}

/// 我们监听的这个路径，现在该不该重新 bind 抢回来。
#[cfg(unix)]
fn should_rebind(sock: &Path, own_ino: u64) -> bool {
    match inode_of(sock) {
        Some(ino) if ino == own_ino => false, // 还是我们那个，继续服务
        // 被更新的实例顶掉了：对方活着就让它服务（它退出时我们再接回来），没人应答说明是崩溃留下的陈旧文件
        Some(_) => !someone_alive_at(sock),
        None => true, // 文件没了，多半是另一份实例退出时删掉的
    }
}

/// 一边 accept 一边每隔 interval 巡查一次路径，该重新 bind 了就返回。
/// listener 必须已经设成非阻塞：用 poll 等连接、拿下一次巡查的时刻当超时，
/// 所以正常服务时巡查不会给连接添一点延迟（只是看一眼路径上的 inode）；
/// 被别的实例顶掉之后每次巡查会去 ping 对方、最多等两秒，那时路径已经不指向我们了、
/// 本来也没人连得上我们，卡一会儿没关系。
#[cfg(unix)]
fn accept_until_lost(
    listener: &UnixListener,
    sock: &Path,
    own_ino: u64,
    interval: Duration,
    on_conn: &dyn Fn(UnixStream),
) {
    use std::os::unix::io::AsRawFd;
    let mut next_check = Instant::now() + interval;
    loop {
        let wait = next_check.saturating_duration_since(Instant::now());
        let timeout = wait.as_millis().min(libc::c_int::MAX as u128) as libc::c_int;
        let mut pfd = libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: pfd 是一个有效的、我们自己拥有的 pollfd，数组长度就是 1
        let ready = unsafe { libc::poll(&mut pfd, 1, timeout) };
        if ready < 0 {
            // 被信号打断就直接再 poll；别的错误退避一下，免得空转
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::Interrupted {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        if ready > 0 {
            match listener.accept() {
                // macOS 上 accept 出来的连接会继承 listener 的 O_NONBLOCK，处理连接的代码要的是阻塞语义
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    on_conn(stream);
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(100)), // 别在持续出错时空转
            }
        }
        if Instant::now() >= next_check {
            if should_rebind(sock, own_ino) {
                return;
            }
            next_check = Instant::now() + interval;
        }
    }
}

/// 一直守着 sock 这个路径：bind 上去 accept，路径丢了就重新 bind；bind 失败也只是歇一会儿再来，不返回。
#[cfg(unix)]
pub(crate) fn serve(sock: &Path, interval: Duration, on_conn: &dyn Fn(UnixStream), log: &dyn Fn(&str)) {
    let mut bind_failures: u32 = 0;
    loop {
        let bound = bind(sock).and_then(|(l, ino)| {
            l.set_nonblocking(true)?;
            Ok((l, ino))
        });
        let (listener, ino) = match bound {
            Ok(v) => v,
            Err(e) => {
                // bind 失败多半是暂时的：数据目录一时不在，或者别的实例的 remove+bind 正好插在
                // 我们 remove 和 bind 中间（EADDRINUSE）。这时候放弃比重试糟得多——进程会一路活下去，
                // 既没有 socket 也没有这个巡查线程，Hook 从此找不到我们，只能重启 App 才好。
                bind_failures = bind_failures.saturating_add(1);
                if bind_failures == 1 || bind_failures % 60 == 0 {
                    log(&format!("socket 监听失败：{e}，稍后重试"));
                }
                OWN_INODE.store(0, Ordering::Relaxed);
                std::thread::sleep(interval);
                continue;
            }
        };
        bind_failures = 0;
        OWN_INODE.store(ino, Ordering::Relaxed);
        accept_until_lost(&listener, sock, ino, interval, on_conn);
        OWN_INODE.store(0, Ordering::Relaxed);
        log("app.sock 不再是我们的了（被删掉了，或是别的实例留下的陈旧文件），重新监听");
    }
}

/// 一条连接的完整处理：读一行 JSON 请求 → 回一行 JSON。
#[cfg(unix)]
fn handle_conn(app: AppHandle, stream: UnixStream) {
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() { return; }
    let req = match serde_json::from_str::<Value>(line.trim()) {
        Ok(req) => req,
        Err(e) => {
            let mut w = &stream;
            let _ = w.write_all(json!({"ok": false, "error": format!("请求不是 JSON：{e}")}).to_string().as_bytes());
            let _ = w.write_all(b"\n");
            return;
        }
    };
    let reply = handle(&app, &req);
    let mut w = &stream;
    let _ = w.write_all(reply.to_string().as_bytes());
    let _ = w.write_all(b"\n");
}

pub fn start(app: AppHandle, paths: &Paths) {
    #[cfg(unix)]
    {
        let sock = paths.data_dir.join("app.sock");
        std::thread::spawn(move || {
            serve(
                &sock,
                WATCH_INTERVAL,
                &|stream| {
                    let app = app.clone();
                    std::thread::spawn(move || handle_conn(app, stream));
                },
                &|m| crate::app_log(m),
            );
        });
    }
    #[cfg(not(unix))]
    {
        let _ = (app, paths);
    }
}

fn handle(app: &AppHandle, req: &Value) -> Value {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match s("kind").as_str() {
        "notify" => {
            let mode = s("mode");
            let (title, body, host, agent) = (s("title"), s("body"), s("host"), s("agent"));
            // 兼容旧 Hook：没有独立来源字段时，点击目标就是来源。
            let source_host = req.get("source_host").and_then(Value::as_str).unwrap_or(&host);
            let mut errors = Vec::new();
            if mode == "overlay" || mode == "both" {
                if let Err(e) = crate::overlay::show(app, &title, &body, &host, source_host, &agent) { errors.push(e); }
            }
            if mode == "system" || mode == "both" {
                if let Err(e) = crate::overlay::system_notify(app, &title, &body, &host, source_host) { errors.push(e); }
            }
            if errors.is_empty() { json!({"ok": true}) } else { json!({"ok": false, "error": errors.join("；")}) }
        }
        "ping" => json!({"ok": true, "pid": std::process::id()}),
        other => json!({"ok": false, "error": format!("未知请求 {other}")}),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    /// 往路径上发一条 ping，拿回解析好的回复；连不上、超时、回的不是 JSON 都算没人应答。
    fn ask(sock: &Path) -> Option<Value> {
        let mut s = UnixStream::connect(sock).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
        s.write_all(b"{\"kind\":\"ping\"}\n").ok()?;
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).ok()?;
        serde_json::from_str(line.trim()).ok()
    }

    /// 每 50ms 试一次 cond，直到它成立（true）或者等够了 limit（false）。
    fn wait_for(limit: Duration, cond: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if cond() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// 假的连接处理：读一行请求，回一行带 who 的成功回复，好认出现在是哪个实例在服务。
    fn reply_who(who: &'static str) -> impl Fn(UnixStream) {
        move |stream: UnixStream| {
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
            let mut w = &stream;
            let _ = w.write_all(json!({"ok": true, "who": who}).to_string().as_bytes());
            let _ = w.write_all(b"\n");
        }
    }

    /// 起一个跑 serve 的线程（不 join：serve 不会返回，线程活到测试进程结束），返回它写的日志。
    fn spawn_serve(sock: PathBuf, who: &'static str) -> Arc<Mutex<Vec<String>>> {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let sink = logs.clone();
        std::thread::spawn(move || {
            let log = move |m: &str| sink.lock().unwrap().push(m.to_string());
            serve(&sock, Duration::from_millis(100), &reply_who(who), &log);
        });
        logs
    }

    /// 准备一个空数据目录和里面的 app.sock 路径（TempDir 要一直拿着，drop 了目录就没了）。
    fn temp_sock() -> (tempfile::TempDir, PathBuf) {
        let (dir, paths) = crate::core::paths::temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let sock = paths.data_dir.join("app.sock");
        (dir, sock)
    }

    fn who_is(sock: &Path, expected: &str) -> bool {
        ask(sock).map(|v| v["who"] == expected).unwrap_or(false)
    }

    #[test]
    fn serve_rebinds_after_someone_deletes_the_socket_file() {
        let (_d, sock) = temp_sock();
        let logs = spawn_serve(sock.clone(), "A");
        assert!(wait_for(Duration::from_secs(2), || ask(&sock).is_some()), "serve 应该先把 socket 起起来");

        // 模拟另一份 AgentPulse 退出时把文件删了
        std::fs::remove_file(&sock).unwrap();

        assert!(
            wait_for(Duration::from_secs(2), || who_is(&sock, "A")),
            "socket 文件被删后应该在一个周期内重新监听"
        );
        assert!(logs.lock().unwrap().iter().any(|m| m.contains("重新监听")), "重新监听时该留一行日志");
    }

    #[test]
    fn serve_stays_passive_while_a_newer_live_instance_owns_the_path_then_takes_over_when_it_exits() {
        let (_d, sock) = temp_sock();
        let _logs = spawn_serve(sock.clone(), "A");
        assert!(wait_for(Duration::from_secs(2), || ask(&sock).is_some()), "serve 应该先把 socket 起起来");

        // 更新的一份实例 B 启动：remove + bind 把路径顶掉，自己开始应答
        let (b, b_ino) = bind(&sock).unwrap();
        b.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let b_thread = {
            let stop = stop.clone();
            std::thread::spawn(move || loop {
                match b.accept() {
                    Ok((s, _)) => {
                        let _ = s.set_nonblocking(false);
                        reply_who("B")(s);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            })
        };

        std::thread::sleep(Duration::from_millis(350)); // 够 A 巡查好几轮了
        assert_eq!(inode_of(&sock), Some(b_ino), "别的实例活着时不能把路径抢回来");
        assert!(who_is(&sock, "B"), "路径该由更新的那份实例服务");

        // B 退出：线程停掉、listener drop、把自己的 socket 收走
        stop.store(true, Ordering::Relaxed);
        b_thread.join().unwrap();
        remove_if_ours(&sock, b_ino);

        assert!(wait_for(Duration::from_secs(2), || who_is(&sock, "A")), "B 退出后 A 要接回来");
    }

    #[test]
    fn serve_takes_over_a_stale_socket_file_left_by_a_dead_instance() {
        let (_d, sock) = temp_sock();
        let _logs = spawn_serve(sock.clone(), "A");
        assert!(wait_for(Duration::from_secs(2), || ask(&sock).is_some()), "serve 应该先把 socket 起起来");

        // 一个 bind 完就死掉的实例：文件留在那儿（UnixListener 的 Drop 不删路径），但没人应答
        let (dead, _) = bind(&sock).unwrap();
        drop(dead);

        assert!(wait_for(Duration::from_secs(2), || who_is(&sock, "A")), "陈旧的 socket 文件该被顶掉重新监听");
    }

    #[test]
    fn serve_keeps_retrying_until_the_socket_can_be_bound() {
        let (_d, paths) = crate::core::paths::temp_paths();
        // 数据目录的位置上摆一个普通文件：create_dir_all 和 bind 都会失败（不是目录）
        std::fs::write(&paths.data_dir, b"not a dir").unwrap();
        let sock = paths.data_dir.join("app.sock");
        let logs = spawn_serve(sock.clone(), "A");

        assert!(
            wait_for(Duration::from_secs(2), || logs.lock().unwrap().iter().any(|m| m.contains("监听失败"))),
            "bind 不上时该留一行日志"
        );

        // 数据目录恢复正常
        std::fs::remove_file(&paths.data_dir).unwrap();
        std::fs::create_dir_all(&paths.data_dir).unwrap();

        assert!(
            wait_for(Duration::from_secs(2), || who_is(&sock, "A")),
            "监听失败后要一直重试，目录好了就该服务"
        );
    }

    #[test]
    fn remove_if_ours_leaves_another_instances_socket_alone() {
        let (_d, sock) = temp_sock();
        let (a, a_ino) = bind(&sock).unwrap();
        let (b, b_ino) = bind(&sock).unwrap(); // B 顶掉了 A 的文件
        assert_ne!(a_ino, b_ino, "两次 bind 该是两个不同的文件");

        remove_if_ours(&sock, a_ino);
        assert_eq!(inode_of(&sock), Some(b_ino), "A 退出时不能删掉 B 的 socket");
        remove_if_ours(&sock, b_ino);
        assert_eq!(inode_of(&sock), None, "B 退出时该把自己的 socket 收走");

        drop((a, b)); // 两个 listener 都活到最后，免得 inode 被回收再用
    }
}
