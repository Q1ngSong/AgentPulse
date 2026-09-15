//! 桌宠素材目录与片段定义。界面与配置均使用从 1 开始的闭区间帧号。
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use super::config::Target;

pub const PHASES: [&str; 4] = ["working", "sleeping", "permission_request", "task_complete"];
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Animation {
    pub folder: String,
    pub frames: usize,
    pub frame_ms: u64,
    pub enter: [usize; 2],
    pub cycle: [usize; 2],
    pub exit: [usize; 2],
    pub hold: bool,
}
pub type Animations = BTreeMap<String, Animation>;
pub fn defaults() -> Animations {
    PHASES.into_iter().map(|phase| (phase.into(), Animation {
        folder: String::new(), frames: 50, frame_ms: 80, enter: [1,5], cycle: [6,45], exit: [46,50], hold: phase == "task_complete",
    })).collect()
}
pub fn from_target(target: &Target) -> Result<Animations, String> {
    let animations: Animations = serde_json::from_value(target.extra.get("animations").cloned().ok_or("桌宠缺少动作配置")?)
        .map_err(|e| format!("桌宠动作配置无效：{e}"))?;
    validate(&animations)?;
    Ok(animations)
}
pub fn validate(animations: &Animations) -> Result<(), String> {
    if animations.len() != 4 || PHASES.iter().any(|p| !animations.contains_key(*p)) { return Err("桌宠需要配置全部四种状态".into()); }
    for (phase, a) in animations {
        if !(3..=240).contains(&a.frames) || !(16..=1000).contains(&a.frame_ms) { return Err(format!("{phase}：帧数需为 3–240，帧间隔需为 16–1000 毫秒")); }
        if !a.folder.is_empty() && !std::path::Path::new(&a.folder).is_absolute() { return Err(format!("{phase}：请填写文件夹的绝对路径，或留空使用内置素材")); }
        for [start,end] in [a.enter,a.cycle,a.exit] {
            if start == 0 || start > end || end > a.frames { return Err(format!("{phase}：片段帧号越界或起点大于终点")); }
        }
        if a.enter[1] >= a.cycle[0] || a.cycle[1] >= a.exit[0] { return Err(format!("{phase}：进入、循环、退出片段必须依次排列且不能重叠")); }
        if a.folder.is_empty() && a.frames != 50 { return Err(format!("{phase}：内置素材固定为 50 帧")); }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_ranges_and_all_states() {
        let mut a=defaults(); assert!(validate(&a).is_ok());
        a.get_mut("working").unwrap().cycle=[5,45]; assert!(validate(&a).is_err());
        a=defaults(); a.get_mut("sleeping").unwrap().exit=[46,51]; assert!(validate(&a).is_err());
        a=defaults(); a.remove("task_complete"); assert!(validate(&a).is_err());
        a=defaults(); a.get_mut("working").unwrap().folder="relative".into(); assert!(validate(&a).is_err());
    }
}
