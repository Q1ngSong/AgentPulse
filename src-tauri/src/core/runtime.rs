//! 运行环境：把路径、宿主探测、提醒方式、睡眠与后台进程启动收在一起，测试时逐项替换。
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::channels::Registry;
use super::paths::Paths;
use super::platform::{HostProbe, SystemProbe};

/// App 和 Hook 共用的二进制目录；开发期在 target/，打包后在 Contents/MacOS/。
pub fn hook_binary() -> PathBuf {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    dir.join(if cfg!(windows) { "agentpulse-hook.exe" } else { "agentpulse-hook" })
}

fn worker_command(paths: &Paths) -> Command {
    let mut cmd = Command::new(hook_binary());
    cmd.arg("--queue-worker").env("AGENTPULSE_HOME", &paths.data_dir)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // 独立进程组，Hook 退出后继续运行
    }
    cmd
}

pub struct Runtime {
    pub paths: Paths,
    pub home: PathBuf,
    pub probe: Box<dyn HostProbe>,
    pub channels: Registry,
    pub sleep: Box<dyn Fn(Duration) + Send + Sync>,
    /// 启动后台队列 worker（独立进程）。
    pub spawn_worker: Box<dyn Fn() + Send + Sync>,
}

impl Runtime {
    /// 真实环境：系统探测、内置提醒方式、真睡眠，用同目录的 Hook 二进制起 worker。
    pub fn system(paths: Paths) -> Self {
        let home = super::paths::home_dir();
        let worker_paths = paths.clone();
        Self {
            channels: Registry::default_for(&paths),
            paths,
            home,
            probe: Box::new(SystemProbe),
            sleep: Box::new(std::thread::sleep),
            spawn_worker: Box::new(move || {
                let _ = worker_command(&worker_paths).spawn();
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_uses_hook_binary_and_same_data_directory() {
        let (_d, paths) = super::super::paths::temp_paths();
        let cmd = worker_command(&paths);
        let exe = std::env::current_exe().unwrap();
        assert_eq!(std::path::Path::new(cmd.get_program()).parent(), exe.parent());
        assert_eq!(std::path::Path::new(cmd.get_program()).file_stem().unwrap(), "agentpulse-hook");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), vec!["--queue-worker"]);
        assert!(cmd.get_envs().any(|(key, value)| key == "AGENTPULSE_HOME" && value == Some(paths.data_dir.as_os_str())));
    }
}
