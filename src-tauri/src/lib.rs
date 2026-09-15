//! AgentPulse 桌面 App：React 面板（Tauri 命令直调 Rust 核心）、托盘、悬浮窗、系统通知、
//! 以及供 agentpulse-hook 使用的本地 socket。

pub mod commands;
pub mod core;
pub mod ipc;
pub mod overlay;
pub mod pet;
pub mod pet_assets;
#[cfg(target_os = "macos")]
mod notifications;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::core::paths::Paths;

// 冷启动先不创建交互窗口，等启动来源明确后再恢复；通知和 Hook 唤醒均保持后台。
static INTERACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) fn interactive() -> bool { INTERACTIVE.load(Ordering::Acquire) }
pub(crate) fn background_launch() -> bool {
    std::env::args_os().any(|arg| arg == core::channels::desktop::BACKGROUND_ARG)
}

/// 追加一行到 <数据目录>/logs/app.log；超过 2MB 时把旧文件改名为 .1。
pub fn app_log(msg: &str) {
    use std::io::Write;
    let dir = Paths::from_env().logs();
    if std::fs::create_dir_all(&dir).is_err() { return; }
    let path: PathBuf = dir.join("app.log");
    if path.metadata().map(|m| m.len() > 2_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("app.log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

/// 重新显示主窗口时，先恢复 Dock 图标。
///
/// 变更: 2026-09-15 改用 set_dock_visibility 而不是 set_activation_policy(Regular)：
/// 后者在某些 macOS 版本上重建的 Dock 图标是系统默认的占位图标，不是 App 自己的图标；
/// set_dock_visibility 底层走的是更老的 TransformProcessType，图标不会丢。
fn show_main(app: &AppHandle) {
    if !INTERACTIVE.swap(true, Ordering::AcqRel) { pet::restore(app.clone()); }
    #[cfg(target_os = "macos")]
    let _ = app.set_dock_visibility(true);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("AgentPulse")
        .inner_size(1000.0, 680.0)
        .min_inner_size(860.0, 560.0)
        .center()
        .build();
}

pub fn run() {
    app_log("AgentPulse 启动");
    // UNUserNotificationCenter 的 delegate 必须在 macOS 完成启动前注册。
    #[cfg(target_os = "macos")]
    notifications::init();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_state, commands::save_tool_targets, commands::copy_target, commands::save_templates,
            commands::test_target, commands::save_pets, commands::test_pet, commands::simulate, commands::list_sounds, commands::upload_sound,
            commands::rename_sound, commands::delete_sound, commands::preview_sound,
            commands::install_integration, commands::uninstall_integration, commands::read_events, commands::clear_events,
            commands::review_codex_hooks, commands::trust_codex_hooks,
            commands::reveal, commands::activate_app,
            commands::upload_tool_icon, commands::delete_tool_icon,
            commands::get_auto_launch, commands::set_auto_launch,
            pet_assets::inspect_pet_folder, pet_assets::load_pet_frames,
            pet::get_pet_state, pet::close_pet, pet::activate_pet_source,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            overlay::init(&handle);
            ipc::start(handle.clone(), &Paths::from_env());
            #[cfg(target_os = "macos")]
            if background_launch() { let _ = handle.set_dock_visibility(false); }

            let open = MenuItem::with_id(app, "open", "打开面板", true, None::<&str>)?;
            let test = MenuItem::with_id(app, "test-overlay", "测试悬浮窗", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &test, &quit])?;
            // 托盘图标：macOS 不会像 Dock 那样自动做圆角遮罩，用带圆角的独立图标文件，
            // 避免直角方块。
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?;
            TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, ev| match ev.id().as_ref() {
                    "open" => show_main(app),
                    "test-overlay" => {
                        let _ = overlay::show(app, "Claude Code: 测试对话", "✅ 任务完成 · 这是一条来自 AgentPulse 的悬浮窗测试", "com.anthropic.claudefordesktop", "com.anthropic.claudefordesktop", "claude");
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // macOS 由启动通知决定是否显示面板；通知冷启动时只处理来源跳转。
            #[cfg(not(target_os = "macos"))]
            show_main(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label().starts_with("overlay-") && matches!(event, WindowEvent::Destroyed) {
                overlay::on_destroyed(window.app_handle(), window.label());
            }
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                    // 关闭主窗口后只留菜单栏图标：从 Dock 和 Cmd+Tab 里隐藏（其他平台是空操作）
                    #[cfg(target_os = "macos")]
                    let _ = window.app_handle().set_dock_visibility(false);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    #[cfg(target_os = "macos")]
    notifications::set_app(app.handle().clone());
    app.run(|app, event| match event {
        tauri::RunEvent::Exit => {
            ipc::shutdown(&Paths::from_env());
            app_log("AgentPulse 退出");
        }
        // 关闭主窗口后 App 仍在后台跑（只是隐藏了 Dock 图标）；这时如果用户点了 Dock 里钉住的图标，
        // macOS 发的是 Reopen 而不是重新启动一个进程，不接这个事件的话点了不会有任何反应。
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            if notifications::should_show_main_on_reopen() {
                show_main(app);
            } else {
                app_log("忽略系统通知点击附带的 Reopen");
            }
        }
        _ => {}
    });
}
