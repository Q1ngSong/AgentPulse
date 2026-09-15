fn main() {
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rerun-if-changed=src/overlay/macos.m");
        cc::Build::new()
            .file("src/overlay/macos.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("agentpulse_overlay");
        println!("cargo:rerun-if-changed=src/notifications/macos.m");
        cc::Build::new()
            .file("src/notifications/macos.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("agentpulse_notifications");
        println!("cargo:rerun-if-changed=src/core/icons_macos.m");
        cc::Build::new()
            .file("src/core/icons_macos.m")
            .flag("-fobjc-arc")
            .compile("agentpulse_icons");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=UserNotifications");
    }
    tauri_build::build()
}
