//! 悬浮窗左侧的来源图标：优先用户为某个工具手动上传的自定义图标
//! （<数据目录>/icons/<tool>.<ext>，每个工具最多一张，上传新的会覆盖旧的);
//! 没有自定义图标时，按事件的来源 App bundle id 现读那个 App 自己的图标
//! （macOS：NSWorkspace，见 icons_macos.m；其他平台没有这个能力，直接跳过)。
//! 两者都没有就不显示图标——不是失败，悬浮窗本来就能没有图标。
use std::fs;
use std::io;
use std::path::PathBuf;

use base64::Engine;

use super::config::TOOLS;
use super::paths::Paths;

pub const ALLOWED_EXT: [&str; 4] = ["png", "jpg", "jpeg", "gif"];
pub const MAX_BYTES: usize = 1024 * 1024;

fn icons_dir(paths: &Paths) -> io::Result<PathBuf> {
    let d = paths.icons();
    fs::create_dir_all(&d)?;
    Ok(d)
}

fn mime_of(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// 某个工具当前的自定义图标文件；工具名不认识、或者没传过，都是 None。
fn custom_icon_path(paths: &Paths, tool: &str) -> Option<PathBuf> {
    if !TOOLS.contains(&tool) {
        return None;
    }
    let dir = paths.icons();
    ALLOWED_EXT.iter().map(|ext| dir.join(format!("{tool}.{ext}"))).find(|p| p.is_file())
}

fn to_data_url(mime: &str, data: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(data))
}

/// 保存某个工具的自定义图标（覆盖旧的，哪怕扩展名不一样）。filename 只用来判断格式。
pub fn save_upload(paths: &Paths, tool: &str, filename: &str, data: &[u8]) -> Result<(), String> {
    if !TOOLS.contains(&tool) {
        return Err("未知的工具".into());
    }
    if data.is_empty() {
        return Err("文件为空".into());
    }
    if data.len() > MAX_BYTES {
        return Err("图标不能超过 1 MB".into());
    }
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    if !ALLOWED_EXT.contains(&ext.as_str()) {
        return Err("只支持 png / jpg / gif 格式".into());
    }
    let dir = icons_dir(paths).map_err(|e| e.to_string())?;
    for old in ALLOWED_EXT.iter().map(|e| dir.join(format!("{tool}.{e}"))) {
        let _ = fs::remove_file(old);
    }
    fs::write(dir.join(format!("{tool}.{ext}")), data).map_err(|e| e.to_string())
}

/// 删掉某个工具的自定义图标；本来就没有也不算错。
pub fn delete(paths: &Paths, tool: &str) -> Result<(), String> {
    if let Some(p) = custom_icon_path(paths, tool) {
        fs::remove_file(p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 某个工具当前的自定义图标（data: URL），给设置页预览用；没传过就是 None。
pub fn custom_data_url(paths: &Paths, tool: &str) -> Option<String> {
    let p = custom_icon_path(paths, tool)?;
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    let data = fs::read(&p).ok()?;
    Some(to_data_url(mime_of(&ext), &data))
}

/// 悬浮窗要用的图标（data: URL）：自定义图标优先，没有就按来源 App 的 bundle id 现读它自己的图标；
/// 都没有就是 None——悬浮窗照样能显示，只是没有图标。
pub fn resolve_data_url(paths: &Paths, tool: &str, host_bundle: &str) -> Option<String> {
    if let Some(url) = custom_data_url(paths, tool) {
        return Some(url);
    }
    let data = native::icon_for_bundle(host_bundle)?;
    Some(to_data_url("image/png", &data))
}

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};

    extern "C" {
        fn ap_icon_for_bundle(bundle_id: *const c_char, out_data: *mut *mut u8, out_len: *mut usize) -> c_int;
        fn ap_icon_free(data: *mut u8);
    }

    /// 本机装着的某个 App 的图标（PNG）；bundle id 找不到对应的 App 就是 None。
    pub fn icon_for_bundle(bundle_id: &str) -> Option<Vec<u8>> {
        if bundle_id.is_empty() {
            return None;
        }
        let c = CString::new(bundle_id).ok()?;
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut len: usize = 0;
        // SAFETY: ap_icon_for_bundle 只在成功（返回非 0）时才会把 out_data/out_len 设成一段
        // 它用 malloc 分配、并把所有权转交给调用方的缓冲区；失败时两个指针都不会被写。
        // 缓冲区只在这里读一次、拷贝进 Vec，再用配对的 ap_icon_free 释放，不会重复释放或悬垂。
        unsafe {
            if ap_icon_for_bundle(c.as_ptr(), &mut data, &mut len) == 0 || data.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(data, len).to_vec();
            ap_icon_free(data);
            Some(v)
        }
    }
}
#[cfg(not(target_os = "macos"))]
mod native {
    pub fn icon_for_bundle(_bundle_id: &str) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::paths::temp_paths;
    use super::*;

    const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47];

    #[test]
    fn save_upload_rejects_unknown_tool_and_bad_ext_and_oversize() {
        let (_d, p) = temp_paths();
        assert!(save_upload(&p, "notatool", "icon.png", PNG_MAGIC).is_err());
        assert!(save_upload(&p, "claude", "icon.svg", PNG_MAGIC).is_err());
        assert!(save_upload(&p, "claude", "icon.png", b"").is_err());
        assert!(save_upload(&p, "claude", "icon.png", &vec![0u8; MAX_BYTES + 1]).is_err());
        assert!(custom_data_url(&p, "claude").is_none());
    }

    #[test]
    fn save_upload_overwrites_previous_icon_even_with_different_extension() {
        let (_d, p) = temp_paths();
        save_upload(&p, "claude", "old.png", PNG_MAGIC).unwrap();
        assert!(custom_data_url(&p, "claude").unwrap().starts_with("data:image/png;base64,"));
        save_upload(&p, "claude", "new.jpg", b"\xFF\xD8\xFF").unwrap();
        let url = custom_data_url(&p, "claude").unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"), "{url}");
        // 旧的 .png 该被清掉，目录里只剩一张
        assert_eq!(fs::read_dir(p.icons()).unwrap().count(), 1);
    }

    #[test]
    fn delete_then_custom_data_url_is_none() {
        let (_d, p) = temp_paths();
        save_upload(&p, "codex", "icon.gif", b"GIF89a").unwrap();
        assert!(custom_data_url(&p, "codex").is_some());
        delete(&p, "codex").unwrap();
        assert!(custom_data_url(&p, "codex").is_none());
        // 再删一次（本来就没有）不该报错
        assert!(delete(&p, "codex").is_ok());
    }

    #[test]
    fn resolve_prefers_custom_icon_over_bundle_lookup() {
        let (_d, p) = temp_paths();
        save_upload(&p, "claude", "icon.png", PNG_MAGIC).unwrap();
        let url = resolve_data_url(&p, "claude", "com.anthropic.claudefordesktop").unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn resolve_without_custom_icon_or_bundle_match_is_none() {
        let (_d, p) = temp_paths();
        assert!(resolve_data_url(&p, "claude", "com.does.not.exist.anywhere").is_none());
    }
}
