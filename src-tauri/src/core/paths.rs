//! 路径与文件工具：数据目录、各数据文件位置、原子写 JSON、文件锁。
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::Serialize;

/// 用户主目录：HOME（macOS/Linux）或 USERPROFILE（Windows）。
pub fn home_dir() -> PathBuf {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

/// 数据目录下所有文件的位置。用结构体而不是全局变量，测试时可各自指向临时目录。
#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    /// AGENTPULSE_HOME 或 ~/.agentpulse。
    pub fn from_env() -> Self {
        if let Ok(p) = std::env::var("AGENTPULSE_HOME") {
            return Self { data_dir: PathBuf::from(p) };
        }
        Self { data_dir: home_dir().join(".agentpulse") }
    }

    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { data_dir: dir.into() }
    }

    pub fn config(&self) -> PathBuf { self.data_dir.join("config.json") }
    pub fn events(&self) -> PathBuf { self.data_dir.join("events.jsonl") }
    pub fn activity(&self) -> PathBuf { self.data_dir.join("activity.jsonl") }
    pub fn backups(&self) -> PathBuf { self.data_dir.join("backups") }
    pub fn queue(&self) -> PathBuf { self.data_dir.join("queue.json") }
    pub fn queue_lock(&self) -> PathBuf { self.data_dir.join("queue.lock") }
    pub fn worker_lock(&self) -> PathBuf { self.data_dir.join("worker.lock") }
    pub fn codex_permission_pending(&self) -> PathBuf { self.data_dir.join("codex-permission-pending.json") }
    pub fn codex_permission_lock(&self) -> PathBuf { self.data_dir.join("codex-permission.lock") }
    pub fn wx_token_cache(&self) -> PathBuf { self.data_dir.join("wx-access-token.json") }
    pub fn sounds(&self) -> PathBuf { self.data_dir.join("sounds") }
    pub fn icons(&self) -> PathBuf { self.data_dir.join("icons") }
    pub fn logs(&self) -> PathBuf { self.data_dir.join("logs") }
    pub fn hook_errors(&self) -> PathBuf { self.data_dir.join("hook-errors.log") }
}

/// 先写临时文件再替换，避免写到一半的文件被读到。
pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{}.", path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp")))
        .suffix(".tmp")
        .tempfile_in(dir)?;
    {
        use io::Write;
        let text = serde_json::to_string_pretty(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        tmp.write_all(text.as_bytes())?;
        tmp.write_all(b"\n")?;
    }
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// 保留原始字节的配置备份；临时文件从创建时就是 0600，完成后才移到备份路径。
pub fn backup_private(source: &Path, dest: &Path) -> io::Result<()> {
    let dir = dest.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    io::copy(&mut File::open(source)?, &mut tmp)?;
    tmp.persist_noclobber(dest).map_err(|e| e.error)?;
    Ok(())
}

/// 只允许本人读写（配置里有密钥）。
pub fn chmod_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// flock 文件锁；drop 时释放。
pub struct FileLock {
    _file: File,
}

impl FileLock {
    /// 阻塞直到拿到锁。
    pub fn acquire(path: &Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).read(true).write(true).append(true).open(path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }

    /// 拿不到锁立即返回 None。
    pub fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).read(true).write(true).append(true).open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.raw_os_error() == Some(33) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
pub(crate) fn temp_paths() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("home"));
    (dir, paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_dirs_and_replaces() {
        let (_d, p) = temp_paths();
        let f = p.data_dir.join("a").join("b.json");
        atomic_write_json(&f, &serde_json::json!({"x": 1})).unwrap();
        atomic_write_json(&f, &serde_json::json!({"x": 2})).unwrap();
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["x"], 2);
        assert!(fs::read_dir(f.parent().unwrap()).unwrap().count() == 1, "没有残留临时文件");
    }

    #[test]
    fn try_lock_reports_contention() {
        let (_d, p) = temp_paths();
        let lock = p.worker_lock();
        let held = FileLock::acquire(&lock).unwrap();
        assert!(FileLock::try_acquire(&lock).unwrap().is_none());
        drop(held);
        assert!(FileLock::try_acquire(&lock).unwrap().is_some());
    }
}
