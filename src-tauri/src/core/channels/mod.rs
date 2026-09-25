//! 提醒方式：每种一个模块，按 target.kind 从 Registry 取。
//! 每个 Channel 的 send 返回附加说明或 None，失败返回 Err，由 send_one 统一计时并吞掉错误，
//! 保证一个提醒方式失败不影响其他。
use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::config::Target;
use super::events::Context;
use super::paths::Paths;

pub mod desktop;
pub mod pet;
pub mod feishu;
pub mod http;
pub mod sound;
pub mod wxtest;

/// 一条要发出的提醒。
#[derive(Clone, Debug, Default)]
pub struct Message {
    pub event: String,
    pub title: String,
    pub body: String,
    pub agent: String,
    /// 谁触发的：hook / simulate / test / delayed（和 dispatch 的 source 一致）。
    pub ctx: Context,
}

/// 飞书卡片的正文：第一行写来源「📍 设备 · App · 项目」，多台设备推到同一个群时靠它分辨；
/// 取不到的部分跳过，全都没有就只发原正文。微信模板按字段分行显示，见 wxtest::template_data。
pub fn external_body(message: &Message) -> String {
    let ctx = &message.ctx;
    let source: Vec<&str> = [&ctx.device, &ctx.app, &ctx.project].into_iter().map(String::as_str).filter(|s| !s.is_empty()).collect();
    let source = if source.is_empty() { String::new() } else { format!("📍 {}", source.join(" · ")) };
    [source.as_str(), message.body.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n")
}

pub trait Channel: Send + Sync {
    fn send(&self, target: &Target, message: &Message) -> Result<Option<String>, String>;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SendResult {
    pub target_id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
    pub ms: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

pub struct Registry {
    channels: HashMap<String, Box<dyn Channel>>,
}

impl Registry {
    pub fn empty() -> Self {
        Self { channels: HashMap::new() }
    }

    /// 内置提醒方式。
    pub fn default_for(paths: &Paths) -> Self {
        let mut r = Self::empty();
        r.register("pet", Box::new(pet::Pet { paths: paths.clone() }));
        r.register("desktop", Box::new(desktop::Desktop { paths: paths.clone() }));
        r.register("sound", Box::new(sound::Sound { paths: paths.clone() }));
        r.register("feishu", Box::new(feishu::Feishu));
        r.register("wxtest", Box::new(wxtest::WxTest { paths: paths.clone() }));
        r
    }

    pub fn register(&mut self, kind: &str, ch: Box<dyn Channel>) {
        self.channels.insert(kind.into(), ch);
    }

    /// 用 target 对应的提醒方式发送，返回结果（不抛错）。
    pub fn send_one(&self, target: &Target, message: &Message) -> SendResult {
        let start = Instant::now();
        let mut r = SendResult { target_id: Some(target.id.clone()), name: target.name.clone(), kind: Some(target.kind.clone()), ..Default::default() };
        match self.channels.get(&target.kind) {
            None => { r.ok = false; r.error = Some(format!("KeyError: '{}'", target.kind)); }
            Some(ch) => match ch.send(target, message) {
                Ok(info) => { r.ok = true; r.info = info; }
                Err(e) => { r.ok = false; r.error = Some(e); }
            },
        }
        r.ms = start.elapsed().as_millis() as u64;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(device: &str, app: &str, project: &str, body: &str) -> Message {
        Message { body: body.into(), ctx: Context { device: device.into(), app: app.into(), project: project.into(), ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn external_body_starts_with_source_line() {
        assert_eq!(external_body(&message("Mac mini", "Claude", "AgentPulse", "✅ 任务完成")), "📍 Mac mini · Claude · AgentPulse\n✅ 任务完成");
        assert_eq!(external_body(&message("", "Claude", "", "正文")), "📍 Claude\n正文", "取不到的部分跳过");
        assert_eq!(external_body(&message("Mac mini", "Claude", "p", "")), "📍 Mac mini · Claude · p");
        assert_eq!(external_body(&message("", "", "", "正文")), "正文", "旧队列条目没有来源信息，照旧只发正文");
    }
}
