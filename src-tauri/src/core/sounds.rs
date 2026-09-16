//! 声音库：系统音效 + <数据目录>/sounds/ 下用户上传的声音，两个工具共用。
//! 声音用字符串引用："system:Ping" 或 "custom:下班了.m4a"。
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::paths::Paths;

pub const SYSTEM_SOUNDS: [&str; 14] = ["Basso", "Blow", "Bottle", "Frog", "Funk", "Glass", "Hero",
    "Morse", "Ping", "Pop", "Purr", "Sosumi", "Submarine", "Tink"];
pub const ALLOWED_EXT: [&str; 6] = ["mp3", "wav", "aiff", "aif", "m4a", "caf"];
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
pub const FALLBACK: &str = "system:Glass";

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Sound {
    pub r#ref: String,
    pub name: String,
    pub kind: String,
    pub ext: String,
    pub size: Option<u64>,
}

fn sounds_dir(paths: &Paths) -> io::Result<PathBuf> {
    let d = paths.sounds();
    fs::create_dir_all(&d)?;
    Ok(d)
}

fn split_ext(name: &str) -> (String, String) {
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i + 1..].to_lowercase()),
        _ => (name.to_string(), String::new()),
    }
}

/// 清洗上传文件名：去掉路径和危险字符，保留中文；扩展名不允许时报错。
pub fn clean_name(filename: &str) -> Result<String, String> {
    let name = filename.replace('\\', "/");
    let name = name.rsplit('/').next().unwrap_or("").trim();
    let (stem, ext) = split_ext(name);
    if !ALLOWED_EXT.contains(&ext.as_str()) {
        return Err("只支持 mp3 / wav / aiff / m4a / caf 格式".into());
    }
    let stem: String = stem
        .chars()
        .filter(|c| !c.is_control() && !"/:*?\"<>|".contains(*c))
        .collect::<String>()
        .trim_matches(|c| c == ' ' || c == '.')
        .chars()
        .take(60)
        .collect();
    if stem.is_empty() {
        return Err("文件名无效".into());
    }
    Ok(format!("{stem}.{ext}"))
}

/// 列出声音库：系统音效在前，自定义按修改时间排序。
pub fn list_sounds(paths: &Paths) -> io::Result<Vec<Sound>> {
    let mut out: Vec<Sound> = SYSTEM_SOUNDS
        .iter()
        .map(|n| Sound { r#ref: format!("system:{n}"), name: n.to_string(), kind: "system".into(), ext: "aiff".into(), size: None })
        .collect();
    let mut custom: Vec<(std::time::SystemTime, Sound)> = Vec::new();
    for entry in fs::read_dir(sounds_dir(paths)?)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        let fname = entry.file_name().to_string_lossy().to_string();
        let (stem, ext) = split_ext(&fname);
        if meta.is_file() && ALLOWED_EXT.contains(&ext.as_str()) {
            custom.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                Sound { r#ref: format!("custom:{fname}"), name: stem, kind: "custom".into(), ext, size: Some(meta.len()) }));
        }
    }
    custom.sort_by_key(|(t, _)| *t);
    out.extend(custom.into_iter().map(|(_, s)| s));
    Ok(out)
}

/// 保存上传的声音，同名时自动加序号；返回新引用。
pub fn save_upload(paths: &Paths, filename: &str, data: &[u8]) -> Result<String, String> {
    if data.len() > MAX_BYTES {
        return Err("声音文件不能超过 5 MB".into());
    }
    if data.is_empty() {
        return Err("文件为空".into());
    }
    let name = clean_name(filename)?;
    let (stem, ext) = split_ext(&name);
    let dir = sounds_dir(paths).map_err(|e| e.to_string())?;
    let mut dest = dir.join(&name);
    let mut n = 1;
    while dest.exists() {
        n += 1;
        dest = dir.join(format!("{stem} {n}.{ext}"));
    }
    fs::write(&dest, data).map_err(|e| e.to_string())?;
    Ok(format!("custom:{}", dest.file_name().unwrap().to_string_lossy()))
}

/// 声音引用 → 音频文件路径；找不到返回 None。
pub fn resolve(paths: &Paths, r#ref: &str) -> Option<PathBuf> {
    let (kind, name) = r#ref.split_once(':')?;
    match kind {
        "system" if SYSTEM_SOUNDS.contains(&name) => Some(PathBuf::from(format!("/System/Library/Sounds/{name}.aiff"))),
        "custom" => {
            let fname = Path::new(name).file_name()?;
            let p = paths.sounds().join(fname);
            p.is_file().then_some(p)
        }
        _ => None,
    }
}

/// 重命名自定义声音（保留扩展名），返回新引用。
pub fn rename(paths: &Paths, r#ref: &str, new_stem: &str) -> Result<String, String> {
    let src = resolve(paths, r#ref).filter(|_| r#ref.starts_with("custom:")).ok_or("只能重命名自定义声音")?;
    let ext = split_ext(&src.file_name().unwrap().to_string_lossy()).1;
    let new = clean_name(&format!("{new_stem}.{ext}"))?;
    let dest = paths.sounds().join(&new);
    if dest.exists() {
        return Err("已有同名声音".into());
    }
    fs::rename(&src, &dest).map_err(|e| e.to_string())?;
    Ok(format!("custom:{new}"))
}

/// 删除自定义声音。
pub fn delete(paths: &Paths, r#ref: &str) -> Result<(), String> {
    let p = resolve(paths, r#ref).filter(|_| r#ref.starts_with("custom:")).ok_or("只能删除已存在的自定义声音")?;
    fs::remove_file(p).map_err(|e| e.to_string())
}

/// 后台播放声音；volume 0-100。找不到声音报错。
pub fn play(paths: &Paths, r#ref: &str, volume: u32) -> Result<(), String> {
    let p = resolve(paths, r#ref).ok_or_else(|| format!("找不到声音：{ref}", ref = r#ref))?;
    let vol = (volume.min(100) as f64) / 100.0;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("afplay")
            .arg("-v").arg(format!("{vol:.2}")).arg(&p)
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(format!("当前平台暂不支持播放声音（{} @ {vol}）", p.display()))
}

#[cfg(test)]
mod tests {
    use super::super::paths::temp_paths;
    use super::*;

    #[test]
    fn clean_name_blocks_traversal_and_bad_ext() {
        assert_eq!(clean_name("../../etc/下班了.M4A").unwrap(), "下班了.m4a");
        for bad in ["evil.sh", "noext", ".m4a", "a/../.mp3"] {
            assert!(clean_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn upload_rename_delete() {
        let (_d, p) = temp_paths();
        let r = save_upload(&p, "金币.wav", b"RIFF").unwrap();
        assert_eq!(r, "custom:金币.wav");
        assert_eq!(save_upload(&p, "金币.wav", b"RIFF").unwrap(), "custom:金币 2.wav");
        assert!(list_sounds(&p).unwrap().iter().any(|s| s.r#ref == "custom:金币.wav"));
        assert_eq!(list_sounds(&p).unwrap()[0].r#ref, "system:Basso");
        let new = rename(&p, &r, "硬币").unwrap();
        assert_eq!(new, "custom:硬币.wav");
        delete(&p, &new).unwrap();
        assert!(resolve(&p, &new).is_none());
        assert!(delete(&p, "system:Ping").is_err());
        assert!(save_upload(&p, "big.mp3", &vec![0u8; MAX_BYTES + 1]).is_err());
        assert!(resolve(&p, "custom:../config.json").is_none());
    }
}
