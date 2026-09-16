//! 悬浮提醒窗与系统通知。
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg(target_os = "macos")]
mod macos;

#[derive(Default)]
struct Registry {
    // 创建顺序也是剩余卡片的排列顺序。来源独立于点击动作，click=close 也保留来源。
    entries: Vec<(String, String)>,
}

impl Registry {
    fn labels_for_source(&self, source: &str) -> Vec<String> {
        if source.is_empty() { return Vec::new(); }
        self.entries.iter().filter(|(_, host)| host == source).map(|(label, _)| label.clone()).collect()
    }

    fn remove(&mut self, label: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(id, _)| id != label);
        self.entries.len() != before
    }
}

static OVERLAYS: Mutex<Registry> = Mutex::new(Registry { entries: Vec::new() });
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 96.0;
const GAP: f64 = 10.0;

pub fn init(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    macos::observe_activations(app.clone());
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

fn origin(app: &AppHandle) -> (f64, f64) {
    match app.primary_monitor().ok().flatten() {
        Some(m) => {
            let scale = m.scale_factor();
            let size = m.size().to_logical::<f64>(scale);
            let pos = m.position().to_logical::<f64>(scale);
            (pos.x + size.width - WIDTH - 16.0, pos.y + 40.0)
        }
        None => (800.0, 40.0),
    }
}

/// 在主线程重排仍存在的卡片，关闭中间一张或一组后不留下空洞。
fn reflow(app: &AppHandle) {
    let labels: Vec<String> = OVERLAYS.lock().unwrap().entries.iter().map(|(id, _)| id.clone()).collect();
    let (x, mut y) = origin(app);
    for label in labels {
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.set_position(tauri::LogicalPosition::new(x, y));
            y += HEIGHT + GAP;
        }
    }
}

pub fn on_destroyed(app: &AppHandle, label: &str) {
    let removed = OVERLAYS.lock().unwrap().remove(label);
    if removed { schedule_reflow(app); }
}

fn schedule_reflow(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    { let _ = app; macos::schedule_reflow(); }
    #[cfg(not(target_os = "macos"))]
    {
        let app = app.clone();
        std::thread::spawn(move || {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || reflow(&handle));
        });
    }
}

fn dismiss_labels(app: &AppHandle, labels: Vec<String>) {
    if labels.is_empty() { return; }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        for label in labels {
            if let Some(win) = handle.get_webview_window(&label) {
                if let Err(e) = win.destroy() {
                    crate::app_log(&format!("关闭悬浮提醒失败：{e}"));
                    continue;
                }
            }
            OVERLAYS.lock().unwrap().remove(&label);
        }
        schedule_reflow(&handle);
    });
}

/// 只清理返回来源时已经显示的提醒；之后收到的新提醒仍正常显示。
pub fn dismiss_source(app: &AppHandle, source: &str) {
    let labels = OVERLAYS.lock().unwrap().labels_for_source(source);
    dismiss_labels(app, labels);
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 不把图标 data URL 塞进导航 URL：大图标会超出 WebKit 的 URL 长度限制，留下空白窗口。
fn overlay_data(title: &str, body: &str, host: &str, icon: Option<&str>) -> serde_json::Value {
    serde_json::json!({"title": title, "body": body, "host": host, "icon": icon})
}

/// 右上角、透明、置顶、不抢焦点、所有桌面可见的悬浮卡片；新卡片排在现有卡片的最下面。
/// agent 是发起这条提醒的工具（claude/codex），用来找这个工具有没有配置自定义图标
/// （core::icons::resolve_data_url）；找不到就按 source_host 的 bundle id 现读那个 App 自己的图标。
/// 返回窗口 label。
pub fn show(app: &AppHandle, title: &str, body: &str, host: &str, source_host: &str, agent: &str) -> Result<String, String> {
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let label = format!("overlay-{millis}-{}", SEQ.fetch_add(1, Ordering::Relaxed));
    // 窗口必须在主线程创建；socket 线程里调用时交给主线程执行。图标的自动查找（macOS：NSWorkspace/
    // NSImage）也放进来一起做：这些 AppKit 调用没有保证线程安全，别在 socket 工作线程上调。
    let app2 = app.clone();
    let label2 = label.clone();
    let (title, body, host, source_host, agent) = (title.to_string(), body.to_string(), host.to_string(), source_host.to_string(), agent.to_string());
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let icon = crate::core::icons::resolve_data_url(&crate::core::paths::Paths::from_env(), &agent, &source_host);
        let data = overlay_data(&title, &body, &host, icon.as_deref());
        let (x, base_y) = origin(&app2);
        let y = base_y + OVERLAYS.lock().unwrap().entries.len() as f64 * (HEIGHT + GAP);
        let builder = WebviewWindowBuilder::new(&app2, &label2, WebviewUrl::App("overlay.html".into()))
            .initialization_script(format!("window.__AGENTPULSE_OVERLAY__ = {data};"))
            .title("AgentPulse 提醒")
            .inner_size(WIDTH, HEIGHT).position(x, y)
            .decorations(false).transparent(true).shadow(true).always_on_top(true)
            .skip_taskbar(true).visible_on_all_workspaces(true).focused(false).resizable(false);
        // 原生材质模糊的是窗口背后的桌面；网页的 backdrop-filter 无法做到这一点。
        #[cfg(target_os = "macos")]
        let builder = builder.effects(tauri::window::EffectsBuilder::new()
            .effect(tauri::window::Effect::Popover)
            .state(tauri::window::EffectState::Active)
            .radius(20.0).build());
        let r = builder.build().map_err(|e| e.to_string()).and_then(|win| {
            if let Err(e) = make_nonactivating_panel(&win) {
                let _ = win.destroy();
                return Err(e);
            }
            OVERLAYS.lock().unwrap().entries.push((label2.clone(), source_host));
            // 在实际创建后计时；按窗口 ID 清理，保留同来源后来出现的通知。
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(5));
                dismiss_labels(&app2, vec![label2]);
            });
            Ok(())
        });
        let _ = tx.send(r);
    }).map_err(|e| e.to_string())?;
    rx.recv_timeout(std::time::Duration::from_secs(5)).map_err(|_| "创建悬浮窗超时".to_string())??;
    Ok(label)
}

/// macOS：把 tao 建出来的普通 NSWindow 换成"非激活面板"（NSPanel + NonactivatingPanel）。
/// 普通窗口被点一下，AgentPulse 自己就成了前台 App——面板窗口跟着跳到最前面，之后 `open -b` 宿主
/// 只是把宿主叠上来，用户看到的是"点了卡片先跳到 AgentPulse"。非激活面板被点击时 App 不激活、
/// 无边框面板也不会成为 key 窗口，宿主一直握着键盘焦点，点击照样传给网页里的按钮。
/// 旧版 Swift 小程序（历史提交里的 notifier/main.swift）就是这么做的，在这台机器上验证过。
/// NSPanel 不比 NSWindow 多任何实例变量，直接换实例的类是 tauri-nspanel 等项目通用的做法；
/// 换类之后 tao 在子类里加的 canBecomeKeyWindow / sendEvent 覆盖一起没了，对只接受点击的卡片没有影响。
#[cfg(target_os = "macos")]
pub(crate) fn make_nonactivating_panel(window: &tauri::WebviewWindow) -> Result<(), String> {
    use objc2::runtime::AnyObject;
    use objc2::ClassType;
    use objc2_app_kit::{NSPanel, NSWindow, NSWindowStyleMask};
    let ptr = window.ns_window().map_err(|e| e.to_string())?;
    if ptr.is_null() {
        return Err("拿不到悬浮窗的 NSWindow".into());
    }
    // SAFETY: 只在主线程调用（show() 的 run_on_main_thread 闭包里）；ptr 是刚建好、还活着的 NSWindow；
    // 换成 NSPanel 不改变内存布局，之后按 NSPanel 调用的方法都是 NSWindow/NSPanel 自己的方法。
    unsafe {
        let window: &NSWindow = &*(ptr as *const NSWindow);
        // WKWebView 已给原窗口注册 KVO。先分离内容，让它正常注销；直接换 isa 会丢掉
        // KVO 子类信息，销毁 WebView 时因 contentLayoutRect 观察者不存在而崩溃。
        // 保留根 contentView：tao 的同步 resize 回调仍会读取它。
        let content = window.contentView().ok_or("悬浮窗缺少 contentView")?;
        let subviews = content.subviews();
        for view in subviews.iter() { view.removeFromSuperview(); }
        let obj: &AnyObject = &*(ptr as *const AnyObject);
        AnyObject::set_class(obj, NSPanel::class());
        let panel: &NSPanel = &*(ptr as *const NSPanel);
        panel.setStyleMask(panel.styleMask() | NSWindowStyleMask::NonactivatingPanel);
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setHidesOnDeactivate(false);
        // 重新挂回后 WebKit 会为 NSPanel 注册自己的观察者。
        for view in subviews.iter() { content.addSubview(&view); }
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn make_nonactivating_panel(_window: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// 系统通知中心（受专注模式和系统通知设置控制）。
pub fn system_notify(app: &AppHandle, title: &str, body: &str, host: &str, source_host: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        crate::notifications::send(title, body, host, source_host)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (host, source_host);
        use tauri_plugin_notification::NotificationExt;
        app.notification().builder().title(title).body(body).show().map_err(|e| e.to_string())
    }
}

/// 把运行会话的 App 切到前台（macOS 用 bundle id）。
pub fn activate(app: &AppHandle, bundle: &str) -> Result<(), String> {
    if bundle.is_empty() {
        return Ok(());
    }
    let labels = OVERLAYS.lock().unwrap().labels_for_source(bundle);
    #[cfg(target_os = "macos")]
    {
        let before = std::time::SystemTime::now();
        let status = std::process::Command::new("open").arg("-b").arg(bundle).status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("无法切回来源 App {bundle}：{status}"));
        }
        // 来源已在前台时没有激活事件；仅在跳转成功后清理，并保留跳转期间到达的新消息。
        crate::notifications::dismiss_source(bundle, before);
    }
    // 来源已经在前台时 macOS 不会再发一次激活事件，点击路径仍需显式清理。
    dismiss_labels(app, labels);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returning_to_source_selects_its_whole_group_only() {
        let registry = Registry { entries: vec![("a1".into(), "com.a".into()), ("b1".into(), "com.b".into()), ("a2".into(), "com.a".into()), ("unknown".into(), "".into())] };
        assert_eq!(registry.labels_for_source("com.a"), ["a1", "a2"]);
        assert!(registry.labels_for_source("").is_empty());
        assert!(registry.labels_for_source("com.c").is_empty());
    }

    #[test]
    fn dismissal_snapshot_preserves_new_arrivals_and_remaining_order() {
        let mut registry = Registry { entries: vec![("a1".into(), "com.a".into()), ("b1".into(), "com.b".into()), ("a2".into(), "com.a".into())] };
        let to_close = registry.labels_for_source("com.a");
        registry.entries.push(("a3".into(), "com.a".into()));
        for id in to_close { assert!(registry.remove(&id)); }
        assert_eq!(registry.entries, [("b1".into(), "com.b".into()), ("a3".into(), "com.a".into())]);
        assert!(!registry.remove("a1"));
    }

    #[test]
    fn expired_window_does_not_clear_newer_messages_or_other_sources() {
        let mut registry = Registry { entries: vec![("first".into(), "com.a".into())] };
        let expiring = "first";
        registry.entries.push(("later".into(), "com.a".into()));
        registry.entries.push(("other".into(), "com.b".into()));
        assert!(registry.remove(expiring));
        // 手动关闭或返回来源后，旧计时器再触发也不影响仍在显示的窗口。
        assert!(!registry.remove(expiring));
        assert_eq!(registry.labels_for_source("com.a"), ["later"]);
        assert_eq!(registry.labels_for_source("com.b"), ["other"]);
    }

    #[test]
    fn overlay_data_roundtrips_unicode_quotes_and_large_icons() {
        let icon = format!("data:image/png;base64,{}", "A".repeat(100_000));
        let original = overlay_data("任务完成", "引号\"、换行\n、<script> & ?", "com.example", Some(&icon));
        let decoded: serde_json::Value = serde_json::from_str(&original.to_string()).unwrap();
        assert_eq!(decoded["title"], "任务完成");
        assert_eq!(decoded["body"], "引号\"、换行\n、<script> & ?");
        assert_eq!(decoded["host"], "com.example");
        assert_eq!(decoded["icon"], icon);
        assert!(overlay_data("T", "B", "", None)["icon"].is_null());
    }
}
