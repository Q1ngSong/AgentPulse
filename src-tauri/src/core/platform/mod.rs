//! 平台相关代码。阶段 1 只有从环境变量推断宿主 App 这一项（各平台通用）；
//! 前台 App、空闲时间、锁屏等在阶段 2 按 target_os 分文件实现。

/// 运行 Claude Code / Codex 的 App：终端、IDE 或各自的桌面版。Hook 进程会继承这些环境变量。
pub const TERM_PROGRAM_BUNDLES: [(&str, &str); 8] = [
    ("Apple_Terminal", "com.apple.Terminal"), ("iTerm.app", "com.googlecode.iterm2"), ("ghostty", "com.mitchellh.ghostty"),
    ("WezTerm", "com.github.wez.wezterm"), ("vscode", "com.microsoft.VSCode"), ("WarpTerminal", "dev.warp.Warp-Stable"),
    ("kitty", "net.kovidgoyal.kitty"), ("zed", "dev.zed.Zed"),
];
pub const AGENT_APP_BUNDLES: [(&str, &str); 2] = [("claude", "com.anthropic.claudefordesktop"), ("codex", "com.openai.codex")];

pub fn agent_app_bundle(agent: &str) -> &'static str {
    AGENT_APP_BUNDLES.iter().find(|(a, _)| *a == agent).map(|(_, b)| *b).unwrap_or("")
}

/// 提醒来自哪个 App：环境变量认不出宿主时，按工具自己的桌面 App 算。
pub fn source_bundle<'a>(host: &'a str, agent: &str) -> &'a str {
    if host.is_empty() { agent_app_bundle(agent) } else { host }
}

/// 当前进程所在的宿主 App 的 bundle id；识别不出返回空字符串。
pub fn host_bundle_id() -> String {
    if let Ok(bid) = std::env::var("__CFBundleIdentifier") {
        if !bid.is_empty() && !bid.starts_with("com.agentpulse") {
            return bid;
        }
    }
    let tp = std::env::var("TERM_PROGRAM").unwrap_or_default();
    TERM_PROGRAM_BUNDLES.iter().find(|(k, _)| *k == tp).map(|(_, b)| b.to_string()).unwrap_or_default()
}

/// 系统状态探测：前台 App、空闲时间、锁屏。测试用假实现替换。
pub trait HostProbe: Send + Sync {
    /// 前台 App 的 bundle id；取不到返回空字符串。
    fn frontmost_bundle(&self) -> String;
    /// 距离上一次键盘/鼠标操作的秒数；取不到时返回很大的数（视为不在电脑前）。
    fn idle_seconds(&self) -> f64;
    /// 屏幕是否锁定；取不到按未锁定处理。
    fn screen_locked(&self) -> bool;
}

pub struct SystemProbe;

#[cfg(target_os = "macos")]
mod macos;

/// 来源选择器展示本机已安装的 App 名称；未知来源保留原始标识。
pub fn app_name(bundle_id: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    { return macos::app_name(bundle_id); }
    #[allow(unreachable_code)]
    { let _ = bundle_id; None }
}

/// 本机名称：macOS 取系统设置里的「电脑名称」，用户改名后跟着变；取不到返回 None。
pub fn device_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    { return macos::device_name(); }
    #[allow(unreachable_code)]
    None
}

/// 优先使用桌面版自带 Codex，再查 CLI 安装位置；不启动桌面窗口。
pub fn codex_binary() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "macos")]
    if let Some(binary) = macos::codex_binary() { return Ok(binary); }
    let search = std::env::var_os("PATH").unwrap_or_default();
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    std::env::split_paths(&search)
        .chain([std::path::PathBuf::from("/opt/homebrew/bin"), std::path::PathBuf::from("/usr/local/bin")])
        .map(|dir| dir.join(name)).find(|path| path.is_file())
        .ok_or_else(|| "找不到 Codex 程序；请安装 Codex，或在 Codex CLI 的 /hooks 中信任 AgentPulse。".into())
}

impl HostProbe for SystemProbe {
    fn frontmost_bundle(&self) -> String {
        #[cfg(target_os = "macos")]
        { return macos::frontmost_bundle(); }
        #[allow(unreachable_code)]
        String::new()
    }
    fn idle_seconds(&self) -> f64 {
        #[cfg(target_os = "macos")]
        { return macos::idle_seconds(); }
        #[allow(unreachable_code)]
        f64::INFINITY
    }
    fn screen_locked(&self) -> bool {
        #[cfg(target_os = "macos")]
        { return macos::screen_locked(); }
        #[allow(unreachable_code)]
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_bundle_falls_back_to_the_agent_app() {
        assert_eq!(source_bundle("com.apple.Terminal", "codex"), "com.apple.Terminal");
        assert_eq!(source_bundle("", "codex"), "com.openai.codex");
        assert_eq!(source_bundle("", "claude"), "com.anthropic.claudefordesktop");
        assert_eq!(source_bundle("", "unknown"), "");
    }
}
