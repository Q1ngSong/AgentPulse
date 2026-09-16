//! macOS 系统通知。来源跟着每条通知保存，点击处理不依赖 React 页面或内存中的“最后一条消息”。
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use tauri::{AppHandle, Manager};

static APP: OnceLock<AppHandle> = OnceLock::new();
static RESPONSES: Mutex<Responses> = Mutex::new(Responses { handled: VecDeque::new() });
static REOPEN_GUARD: Mutex<ReopenGuard> = Mutex::new(ReopenGuard { last_default_click: None });

// macOS 会在通知正文点击后额外发 Reopen；事件本身不带通知 ID。
// 只拦截紧随正文点击的短窗口，不能让一次通知永久吃掉下一次 Dock 点击。
const NOTIFICATION_REOPEN_WINDOW: Duration = Duration::from_millis(250);

struct ReopenGuard {
    last_default_click: Option<Instant>,
}

impl ReopenGuard {
    fn suppresses(&self, now: Instant) -> bool {
        self.last_default_click.is_some_and(|click| now.saturating_duration_since(click) < NOTIFICATION_REOPEN_WINDOW)
    }
}

pub fn should_show_main_on_reopen() -> bool {
    !REOPEN_GUARD.lock().map(|guard| guard.suppresses(Instant::now())).unwrap_or(false)
}

extern "C" fn on_default_click() {
    if let Ok(mut guard) = REOPEN_GUARD.lock() {
        guard.last_default_click = Some(Instant::now());
    }
}

extern "C" {
    fn ap_notifications_init(
        response: extern "C" fn(*const c_char, *const c_char, *const c_char),
        launch: extern "C" fn(c_int),
        default_click: extern "C" fn(),
    );
    fn ap_notifications_send(
        id: *const c_char,
        title: *const c_char,
        body: *const c_char,
        host: *const c_char,
        source_host: *const c_char,
        context: *mut c_void,
        completion: extern "C" fn(*mut c_void, *const c_char),
    );
    fn ap_notifications_dismiss_source(source: *const c_char, before: f64);
}

pub fn init() {
    // SAFETY: run() 在主线程、创建 Tauri 事件循环之前调用；回调有进程级生命周期。
    unsafe { ap_notifications_init(on_response, on_launch, on_default_click) };
}

pub fn set_app(app: AppHandle) {
    let _ = APP.set(app);
}

/// 只在 IPC 工作线程调用。等待系统接受请求；native 从接受时起独立计时，5 秒后清除此条通知。
pub fn send(title: &str, body: &str, host: &str, source_host: &str) -> Result<(), String> {
    let cstring = |s: &str| CString::new(s).map_err(|_| "通知内容不能包含 NUL 字符".to_string());
    let id = cstring(&uuid::Uuid::new_v4().to_string())?;
    let title = cstring(title)?;
    let body = cstring(body)?;
    let host = cstring(host)?;
    let source_host = cstring(source_host)?;
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let context = Box::into_raw(Box::new(tx)).cast();
    // SAFETY: native 在返回前复制所有字符串；context 的唯一所有权交给恰好调用一次的 completion。
    // 即使接收端超时，Sender 仍由回调回收，不会引用栈或已释放的内存。
    unsafe {
        ap_notifications_send(id.as_ptr(), title.as_ptr(), body.as_ptr(), host.as_ptr(), source_host.as_ptr(), context, on_sent);
    }
    rx.recv_timeout(Duration::from_secs(4)).map_err(|_| {
        "尚未确认系统通知是否送达：macOS 仍在等待权限或处理请求，请检查通知设置（稍后仍可能送达）".to_string()
    })?
}

/// 只移除聚焦前已经送达的同来源通知；不取消之后的新通知或其他来源的提醒。
pub fn dismiss_source(source: &str, before: SystemTime) {
    if source.is_empty() { return; }
    let (Ok(source), Ok(before)) = (CString::new(source), before.duration_since(SystemTime::UNIX_EPOCH)) else { return; };
    // SAFETY: native 在返回前复制来源字符串；异步查询由系统通知中心持有。
    unsafe { ap_notifications_dismiss_source(source.as_ptr(), before.as_secs_f64()) };
}

extern "C" fn on_sent(context: *mut c_void, error: *const c_char) {
    // SAFETY: native 原样返回 send() 分配的指针，且只调用一次；error 仅在本次调用中有效。
    let sender = unsafe { Box::from_raw(context.cast::<mpsc::Sender<Result<(), String>>>()) };
    let result = if error.is_null() { Ok(()) } else { Err(unsafe { copy_string(error) }) };
    if let Err(e) = &result { crate::app_log(&format!("系统通知发送失败：{e}")); }
    let _ = sender.send(result);
}

extern "C" fn on_launch(from_notification: c_int) {
    if let Some(app) = APP.get() {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if from_notification != 0 || crate::background_launch() || !should_show_main_on_reopen() {
                crate::app_log("后台通知模式：主界面保持关闭");
                crate::hide_dock(&handle);
            } else {
                crate::show_main(&handle);
            }
        });
    }
}

extern "C" fn on_response(id: *const c_char, action: *const c_char, host: *const c_char) {
    // SAFETY: native 在回调期间持有这三个非空 UTF-8 字符串，离开回调前复制。
    let (id, action, host) = unsafe { (copy_string(id), copy_string(action), copy_string(host)) };
    let click = RESPONSES.lock().ok().and_then(|mut responses| responses.click(&id, &action, &host));
    if let Some(click) = click {
        if let Some(app) = APP.get() {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                // 正文点击会让系统激活 AgentPulse。先收起主面板，再把焦点交给来源；
                // 不隐藏其他悬浮提醒，也不在后台「返回来源」按钮上改变面板可见性。
                if click.hide_main {
                    if let Some(window) = handle.get_webview_window("main") {
                        if let Err(e) = window.hide() { crate::app_log(&format!("收起通知主面板失败：{e}")); }
                    }
                    crate::hide_dock(&handle);
                }
                if let Some(bundle) = click.target {
                    crate::app_log(&format!("系统通知点击：{id} → {bundle}"));
                    if let Err(e) = crate::overlay::activate(&handle, &bundle) {
                        crate::app_log(&format!("系统通知跳转失败：{e}"));
                    }
                }
            });
        }
    }
}

unsafe fn copy_string(ptr: *const c_char) -> String {
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

struct Responses {
    handled: VecDeque<String>,
}

#[derive(Debug, PartialEq)]
struct Click {
    hide_main: bool,
    target: Option<String>,
}

impl Responses {
    fn click(&mut self, id: &str, action: &str, host: &str) -> Option<Click> {
        // 冷启动可能同时收到 launch userInfo 和 delegate 回调，同一通知只切换一次。
        if id.is_empty() || self.handled.iter().any(|seen| seen == id) { return None; }
        self.handled.push_back(id.into());
        if self.handled.len() > 256 { self.handled.pop_front(); }
        if matches!(action, "default" | "return") {
            Some(Click { hide_main: action == "default", target: (!host.is_empty()).then(|| host.into()) })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaved_notifications_return_to_their_own_source() {
        let mut responses = Responses { handled: VecDeque::new() };
        assert_eq!(responses.click("codex-new", "default", "com.openai.codex"),
            Some(Click { hide_main: true, target: Some("com.openai.codex".into()) }));
        assert_eq!(responses.click("claude-old", "return", "com.apple.Terminal"),
            Some(Click { hide_main: false, target: Some("com.apple.Terminal".into()) }));
    }

    #[test]
    fn close_only_missing_host_and_unknown_actions_never_activate() {
        let mut responses = Responses { handled: VecDeque::new() };
        assert!(responses.click("dismissed", "dismiss", "com.openai.codex").is_none());
        assert_eq!(responses.click("close-only", "default", ""), Some(Click { hide_main: true, target: None }));
        assert!(responses.click("unknown", "other", "com.openai.codex").is_none());
        assert!(responses.click("", "default", "com.openai.codex").is_none());
    }

    #[test]
    fn launch_and_delegate_do_not_activate_the_same_notification_twice() {
        let mut responses = Responses { handled: VecDeque::new() };
        assert!(responses.click("one", "default", "com.openai.codex").is_some());
        assert!(responses.click("one", "default", "com.openai.codex").is_none());
        assert!(responses.click("close", "default", "").is_some());
        assert!(responses.click("close", "default", "").is_none());
    }

    #[test]
    fn notification_reopen_is_suppressed_but_later_dock_open_is_allowed() {
        let now = Instant::now();
        let mut guard = ReopenGuard { last_default_click: None };
        assert!(!guard.suppresses(now));
        guard.last_default_click = Some(now);
        assert!(guard.suppresses(now));
        assert!(guard.suppresses(now + Duration::from_millis(100)));
        assert!(!guard.suppresses(now + NOTIFICATION_REOPEN_WINDOW));
        assert!(!guard.suppresses(now + Duration::from_secs(1)));
    }

    #[test]
    fn successive_notifications_extend_only_the_current_reopen_window() {
        let now = Instant::now();
        let mut guard = ReopenGuard { last_default_click: Some(now) };
        guard.last_default_click = Some(now + Duration::from_millis(200));
        assert!(guard.suppresses(now + Duration::from_millis(300)));
        assert!(!guard.suppresses(now + Duration::from_millis(450)));
    }
}
