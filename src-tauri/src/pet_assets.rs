//! 默认桌宠图片按需下载并校验缓存；自定义动作只读取用户指定的序号 PNG。
use std::{fs, path::{Path, PathBuf}};
use serde_json::{json, Value};
use base64::{Engine, engine::general_purpose::STANDARD};
use crate::core::{pet_animation::{self, Animations}};

#[derive(serde::Deserialize)]
struct Asset { name: String, phase: String, bytes: u64, sha256: String }
#[derive(serde::Deserialize)]
struct Manifest { base_url: String, files: Vec<Asset> }
fn manifest() -> Result<Manifest, String> {
    serde_json::from_str(include_str!("../pet-assets.json")).map_err(|e| e.to_string())
}
fn valid_asset(bytes: &[u8], asset: &Asset) -> bool {
    use sha2::{Digest, Sha256};
    bytes.len() as u64 == asset.bytes && format!("{:x}", Sha256::digest(bytes)) == asset.sha256
}
/// 校验现有缓存；下载完整并校验后才原子替换，失败不留下半个图片。
fn cached_asset(dir: &Path, asset: &Asset, fetch: impl FnOnce() -> Result<Vec<u8>, String>) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let path = dir.join(&asset.name);
    if let Ok(bytes) = fs::read(&path) {
        if valid_asset(&bytes, asset) { return Ok(bytes); }
    }
    let bytes = fetch()?;
    if !valid_asset(&bytes, asset) { return Err(format!("桌宠素材 {} 校验失败，请重试", asset.name)); }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    tmp.write_all(&bytes).map_err(|e| e.to_string())?;
    tmp.persist(&path).map_err(|e| e.to_string())?;
    Ok(bytes)
}
fn download_assets(paths: &crate::core::paths::Paths) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
    use std::io::Read;
    use std::time::Duration;
    let manifest = manifest()?;
    let dir = paths.data_dir.join("pets").join("v1");
    let _lock = crate::core::paths::FileLock::acquire(&dir.join("download.lock")).map_err(|e| e.to_string())?;
    let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90)).user_agent("AgentPulse-pet/1").build().map_err(|e| e.to_string())?;
    manifest.files.iter().map(|asset| {
        let bytes = cached_asset(&dir, asset, || {
            let response = client.get(format!("{}/{}", manifest.base_url, asset.name)).send()
                .and_then(|r| r.error_for_status()).map_err(|e| format!("下载桌宠素材失败，请检查网络后重试：{e}"))?;
            if response.content_length().is_some_and(|n| n > asset.bytes) { return Err("桌宠素材大小不符".into()); }
            let mut bytes = Vec::new();
            response.take(asset.bytes + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            Ok(bytes)
        })?;
        Ok((asset.name.clone(), bytes))
    }).collect()
}
/// 所有图片放在用户数据目录；只在启用默认猫咪或预览时下载。
pub fn prepare(animations: &Animations) -> Result<(), String> {
    if animations.values().any(|a| a.folder.is_empty()) { download_assets(&crate::core::paths::Paths::from_env())?; }
    validate_files(animations)
}
#[tauri::command]
pub async fn load_builtin_pet() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let assets = download_assets(&crate::core::paths::Paths::from_env())?;
        Ok(Value::Object(assets.into_iter().filter(|(name, _)| name.ends_with(".webp"))
            .map(|(name, bytes)| (name, json!(format!("data:image/webp;base64,{}", STANDARD.encode(bytes))))).collect()))
    }).await.map_err(|e| e.to_string())?
}

fn png_files(folder: &str) -> Result<Vec<PathBuf>, String> {
    let path=Path::new(folder);
    if !path.is_absolute() || !path.is_dir() { return Err("请输入存在的文件夹绝对路径".into()); }
    let mut files=Vec::new();
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry=entry.map_err(|e|e.to_string())?; let p=entry.path();
        if p.extension().and_then(|v|v.to_str()).is_some_and(|v|v.eq_ignore_ascii_case("png")) {
            if !entry.file_type().map_err(|e|e.to_string())?.is_file() { return Err("PNG 帧必须是普通文件，不能是目录或符号链接".into()); }
            let number=p.file_stem().and_then(|v|v.to_str()).filter(|v|!v.is_empty() && v.bytes().all(|b|b.is_ascii_digit()))
                .and_then(|v|v.parse::<usize>().ok()).ok_or("PNG 文件名必须是序号，例如 0001.png")?;
            files.push((number,p));
        }
    }
    files.sort_by_key(|(n,_)|*n);
    if !(3..=240).contains(&files.len()) { return Err("文件夹需要包含 3–240 张序号 PNG".into()); }
    if files.iter().enumerate().any(|(i,(n,_))|*n!=i+1) { return Err("PNG 序号必须从 1 连续排列，不能缺帧或重复".into()); }
    Ok(files.into_iter().map(|(_,p)|p).collect())
}
fn read_png(path: &Path) -> Result<(Vec<u8>, Vec<u8>, u32, u32),String> {
    if fs::metadata(path).map_err(|e|e.to_string())?.len()>4*1024*1024 { return Err("单帧 PNG 不能超过 4 MB".into()); }
    let bytes=fs::read(path).map_err(|e|e.to_string())?;
    if bytes.len()<24 || &bytes[..8]!=b"\x89PNG\r\n\x1a\n" { return Err("不是有效 PNG".into()); }
    let w=u32::from_be_bytes(bytes[16..20].try_into().unwrap());let h=u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if w==0 || h==0 || w>1024 || h>1024 { return Err("单帧尺寸必须在 1–1024 像素内".into()); }
    let image=tauri::image::Image::from_bytes(&bytes).map_err(|e|e.to_string())?;
    let mut pixels=image.rgba().to_vec();
    for p in pixels.chunks_exact_mut(4) { if p[3]==0 { p.fill(0); } }
    Ok((bytes,pixels,w,h))
}
fn inspect(folder: &str, include_frames: bool) -> Result<Value,String> {
    let files=png_files(folder)?;let mut size=None;let mut total=0usize;let mut pixels=0u64;let mut frames=Vec::new();
    for path in &files {
        let (bytes,_,w,h)=read_png(path)?;total+=bytes.len();
        pixels+=u64::from(w)*u64::from(h);
        if pixels>16_777_216 { return Err("单个状态的总像素量超过 1600 万，请缩小素材尺寸或减少帧数".into()); }
        if total>64*1024*1024 { return Err("单个状态的素材不能超过 64 MB".into()); }
        if size.is_some_and(|s|s!=(w,h)) { return Err("同一状态的所有帧必须尺寸一致".into()); } size=Some((w,h));
        if include_frames { frames.push(format!("data:image/png;base64,{}",STANDARD.encode(bytes))); }
    }
    let (w,h)=size.unwrap();
    Ok(json!({"count":files.len(),"width":w,"height":h,"frames":frames}))
}
pub fn validate_files(animations: &Animations) -> Result<(),String> {
    pet_animation::validate(animations)?;
    // 全部使用默认素材时，固定的资源校验已保证共同首尾帧，无需再次解码。
    let mixed = animations.values().any(|a| !a.folder.is_empty());
    let neutral = if mixed && animations.values().any(|a| a.folder.is_empty()) {
        let path = crate::core::paths::Paths::from_env().data_dir.join("pets/v1/pet-v1-neutral.png");
        let bytes = fs::read(path).map_err(|e| format!("默认猫咪素材尚未下载：{e}"))?;
        let manifest = manifest()?;
        let asset = manifest.files.iter().find(|a| a.phase.is_empty()).ok_or("缺少首尾帧记录")?;
        if !valid_asset(&bytes, asset) { return Err("默认猫咪首尾帧校验失败，请重新启用桌宠".into()); }
        Some(tauri::image::Image::from_bytes(&bytes).map_err(|e| e.to_string())?)
    } else { None };
    let mut baseline=None;
    for (phase,a) in animations {
        let endpoints=if a.folder.is_empty() {
            // 默认素材范围也允许调整；首尾必须仍取共同基准帧。
            if a.enter[0]!=1 || a.exit[1]!=50 { return Err(format!("{phase}：默认素材首尾帧必须为 1 和 50")); }
            let Some(neutral) = &neutral else { continue };
            let mut pixels=neutral.rgba().to_vec(); for p in pixels.chunks_exact_mut(4) {if p[3]==0 {p.fill(0);}}
            (pixels.clone(),pixels,neutral.width(),neutral.height())
        } else {
            let info=inspect(&a.folder,false).map_err(|e|format!("{phase}：{e}"))?;
            if info["count"].as_u64()!=Some(a.frames as u64) {return Err(format!("{phase}：配置帧数与文件夹实际帧数不一致，请重新检查文件夹"));}
            let files=png_files(&a.folder)?;
            let (_,first,w,h)=read_png(&files[a.enter[0]-1])?;let (_,last,_,_)=read_png(&files[a.exit[1]-1])?;
            (first,last,w,h)
        };
        if endpoints.0!=endpoints.1 {return Err(format!("{phase}：进入片段首帧与退出片段末帧必须一致"));}
        let signature=(endpoints.0,endpoints.2,endpoints.3);
        if baseline.as_ref().is_some_and(|b|b!=&signature) { return Err("四个状态必须使用相同的首尾帧和画布尺寸".into()); } baseline=Some(signature);
    }
    Ok(())
}
#[tauri::command]
pub async fn inspect_pet_folder(folder: String) -> Result<Value,String> {
    tauri::async_runtime::spawn_blocking(move || inspect(&folder,false)).await.map_err(|e|e.to_string())?
}
#[tauri::command]
pub async fn load_pet_frames(folder: String) -> Result<Value,String> {
    tauri::async_runtime::spawn_blocking(move || inspect(&folder,true)).await.map_err(|e|e.to_string())?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn download_cache_is_verified_atomic_and_works_offline() {
        use sha2::{Digest, Sha256};
        let dir = tempfile::tempdir().unwrap();
        let data = b"verified asset";
        let asset = Asset { name: "test.webp".into(), phase: "working".into(), bytes: data.len() as u64,
            sha256: format!("{:x}", Sha256::digest(data)) };
        let path = dir.path().join(&asset.name);
        assert!(cached_asset(dir.path(), &asset, || Err("offline".into())).is_err());
        assert!(!path.exists(), "失败的下载不生成缓存");
        assert!(cached_asset(dir.path(), &asset, || Ok(b"wrong content".to_vec())).is_err());
        assert!(!path.exists(), "内容不符的下载不生成缓存");
        assert_eq!(cached_asset(dir.path(), &asset, || Ok(data.to_vec())).unwrap(), data);
        assert_eq!(cached_asset(dir.path(), &asset, || panic!("离线缓存不应联网")).unwrap(), data);
        fs::write(&path, b"corrupted").unwrap();
        assert_eq!(cached_asset(dir.path(), &asset, || Ok(data.to_vec())).unwrap(), data);
        assert_eq!(fs::read(&path).unwrap(), data);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "没有残留的下载临时文件");
    }
    #[test]
    fn manifest_has_exactly_the_four_phases_and_neutral_image() {
        let manifest = manifest().unwrap();
        assert!(manifest.base_url.starts_with("https://github.com/Q1ngSong/AgentPulse/releases/download/"));
        assert_eq!(manifest.files.len(), 5);
        for phase in pet_animation::PHASES.into_iter().chain([""]) {
            assert_eq!(manifest.files.iter().filter(|a| a.phase == phase).count(), 1);
        }
        for asset in &manifest.files {
            assert!(!asset.name.contains('/') && !asset.name.contains('\\'));
            assert!(asset.bytes > 0 && asset.bytes < 8 * 1024 * 1024);
            assert_eq!(asset.sha256.len(), 64);
        }
    }
    #[test]
    fn numerical_order_and_missing_or_duplicate_frames() {
        let d=tempfile::tempdir().unwrap();
        for n in 1..=12 {fs::write(d.path().join(format!("{n}.png")),include_bytes!("../icons/32x32.png")).unwrap();}
        let files=png_files(d.path().to_str().unwrap()).unwrap();assert_eq!(files[1].file_name().unwrap(),"2.png");
        fs::write(d.path().join("01.png"),b"invalid").unwrap();assert!(png_files(d.path().to_str().unwrap()).is_err());
        fs::remove_file(d.path().join("01.png")).unwrap();fs::remove_file(d.path().join("5.png")).unwrap();assert!(png_files(d.path().to_str().unwrap()).is_err());
    }
    #[test]
    fn rejects_bad_png_and_mismatched_endpoints() {
        let d=tempfile::tempdir().unwrap();
        for n in 1..=3 {fs::write(d.path().join(format!("{n}.png")),b"invalid").unwrap();}
        assert!(inspect(d.path().to_str().unwrap(),true).is_err());
        for n in 1..=3 {fs::write(d.path().join(format!("{n}.png")),include_bytes!("../icons/32x32.png")).unwrap();}
        assert_eq!(inspect(d.path().to_str().unwrap(),false).unwrap()["count"],3);
        let mut a=pet_animation::defaults(); for work in a.values_mut() { work.folder=d.path().to_str().unwrap().into();work.frames=3;work.enter=[1,1];work.cycle=[2,2];work.exit=[3,3]; }
        assert!(validate_files(&a).is_ok());
        fs::write(d.path().join("3.png"),include_bytes!("../icons/128x128.png")).unwrap();assert!(validate_files(&a).is_err());
    }
}
