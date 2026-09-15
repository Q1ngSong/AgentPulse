//! macOS 探测实现：lsappinfo / ioreg，都是系统自带命令，不需要额外权限。
use std::process::Command;

/// 用系统应用注册表解析实际路径和显示名，兼容重命名或移动后的 App。
fn app_info(bundle_id: &str) -> Option<(std::path::PathBuf, String)> {
    use objc2::{class, msg_send, rc::autoreleasepool, runtime::AnyObject};
    let id = std::ffi::CString::new(bundle_id).ok()?;
    autoreleasepool(|_| unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let bundle: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: id.as_ptr()];
        let url: *mut AnyObject = msg_send![workspace, URLForApplicationWithBundleIdentifier: bundle];
        if url.is_null() { return None; }
        let path: *mut AnyObject = msg_send![url, path];
        let text: *const std::ffi::c_char = msg_send![path, UTF8String];
        if text.is_null() { return None; }
        let path_buf = std::path::PathBuf::from(std::ffi::CStr::from_ptr(text).to_str().ok()?);
        let manager: *mut AnyObject = msg_send![class!(NSFileManager), defaultManager];
        let name: *mut AnyObject = msg_send![manager, displayNameAtPath: path];
        let text: *const std::ffi::c_char = msg_send![name, UTF8String];
        if text.is_null() { return None; }
        let name = std::ffi::CStr::from_ptr(text).to_str().ok()?.trim_end_matches(".app").to_string();
        Some((path_buf, name))
    })
}


/// 通过应用注册信息定位 Codex，支持改名或移动后的 .app。
pub fn codex_binary() -> Option<std::path::PathBuf> {
    let binary = app_info("com.openai.codex")?.0.join("Contents/Resources/codex");
    binary.is_file().then_some(binary)
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let child = Command::new(cmd).args(args).stdin(std::process::Stdio::null()).output().ok()?;
    Some(String::from_utf8_lossy(&child.stdout).to_string())
}

pub fn frontmost_bundle() -> String {
    let asn = run("lsappinfo", &["front"]).map(|s| s.trim().to_string()).unwrap_or_default();
    if asn.is_empty() {
        return String::new();
    }
    let out = run("lsappinfo", &["info", "-only", "bundleid", &asn]).unwrap_or_default();
    out.split_once('=').map(|(_, v)| v.trim().trim_matches('"').to_string()).unwrap_or_default()
}

pub fn idle_seconds() -> f64 {
    let out = run("ioreg", &["-c", "IOHIDSystem", "-d", "4"]).unwrap_or_default();
    for line in out.lines() {
        if line.contains("HIDIdleTime") {
            if let Some((_, v)) = line.rsplit_once('=') {
                if let Ok(ns) = v.trim().parse::<f64>() {
                    return ns / 1e9;
                }
            }
        }
    }
    f64::INFINITY
}

pub fn screen_locked() -> bool {
    run("ioreg", &["-n", "Root", "-d1"]).map(|s| s.contains("\"CGSSessionScreenIsLocked\"=Yes")).unwrap_or(false)
}
