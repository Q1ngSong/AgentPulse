use std::ffi::{c_char, CStr};
use std::sync::OnceLock;
use tauri::AppHandle;

static APP: OnceLock<AppHandle> = OnceLock::new();

extern "C" {
    fn ap_overlay_observe_activations(callback: extern "C" fn(*const c_char));
    fn ap_overlay_reflow_later(callback: extern "C" fn());
}

pub fn schedule_reflow() {
    // 销毁回调里不能重入 Tauri 的窗口集合，等当前事件处理完再重新排列。
    unsafe { ap_overlay_reflow_later(on_reflow) };
}

extern "C" fn on_reflow() {
    if let Some(app) = APP.get() { super::reflow(app); }
}

pub fn observe_activations(app: AppHandle) {
    if APP.set(app).is_ok() {
        // SAFETY: setup 在主线程调用；回调和 AppHandle 的生命周期覆盖整个进程。
        unsafe { ap_overlay_observe_activations(on_activation) };
    }
}

extern "C" fn on_activation(bundle: *const c_char) {
    if bundle.is_null() { return; }
    let before = std::time::SystemTime::now();
    // SAFETY: native 在回调结束前持有 bundle 字符串；dismiss_source 当场复制匹配的窗口 ID。
    let bundle = unsafe { CStr::from_ptr(bundle) }.to_string_lossy();
    if let Some(app) = APP.get() {
        super::dismiss_source(app, &bundle);
        crate::notifications::dismiss_source(&bundle, before);
    }
}
