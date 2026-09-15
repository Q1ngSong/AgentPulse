//! 仅读取用户指定目录中的序号 PNG；返回校验后的帧，不开放任意文件协议。
use std::{fs, path::{Path, PathBuf}};
use serde_json::{json, Value};
use base64::{Engine, engine::general_purpose::STANDARD};
use crate::core::{pet_animation::{self, Animations}};

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
    let neutral=tauri::image::Image::from_bytes(include_bytes!("../../assets/pet/neutral.png")).map_err(|e|e.to_string())?;
    let mut baseline=None;
    for (phase,a) in animations {
        let endpoints=if a.folder.is_empty() {
            // 内置素材范围也允许调整；首尾必须仍取共同基准帧。
            if a.enter[0]!=1 || a.exit[1]!=50 { return Err(format!("{phase}：内置素材首尾帧必须为 1 和 50")); }
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
    fn numerical_order_and_missing_or_duplicate_frames() {
        let d=tempfile::tempdir().unwrap();
        for n in 1..=12 {fs::write(d.path().join(format!("{n}.png")),include_bytes!("../../assets/pet/keyframe.png")).unwrap();}
        let files=png_files(d.path().to_str().unwrap()).unwrap();assert_eq!(files[1].file_name().unwrap(),"2.png");
        fs::write(d.path().join("01.png"),b"invalid").unwrap();assert!(png_files(d.path().to_str().unwrap()).is_err());
        fs::remove_file(d.path().join("01.png")).unwrap();fs::remove_file(d.path().join("5.png")).unwrap();assert!(png_files(d.path().to_str().unwrap()).is_err());
    }
    #[test]
    fn rejects_bad_png_and_mismatched_endpoints() {
        let d=tempfile::tempdir().unwrap();
        for n in 1..=3 {fs::write(d.path().join(format!("{n}.png")),b"invalid").unwrap();}
        assert!(inspect(d.path().to_str().unwrap(),true).is_err());
        for n in 1..=3 {fs::write(d.path().join(format!("{n}.png")),include_bytes!("../../assets/pet/neutral.png")).unwrap();}
        assert_eq!(inspect(d.path().to_str().unwrap(),false).unwrap()["count"],3);
        let mut a=pet_animation::defaults();let work=a.get_mut("working").unwrap();work.folder=d.path().to_str().unwrap().into();work.frames=3;work.enter=[1,1];work.cycle=[2,2];work.exit=[3,3];
        assert!(validate_files(&a).is_ok());
        fs::write(d.path().join("3.png"),include_bytes!("../../assets/pet/keyframe.png")).unwrap();assert!(validate_files(&a).is_err());
    }
}
