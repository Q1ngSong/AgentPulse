//! 提示音：从共用声音库播放该事件选中的声音；声音已被删除时退回 Glass 并在结果里说明。
use serde_json::Value;

use super::{Channel, Message};
use crate::core::config::Target;
use crate::core::paths::Paths;
use crate::core::sounds;

pub struct Sound {
    pub paths: Paths,
}

impl Channel for Sound {
    fn send(&self, target: &Target, message: &Message) -> Result<Option<String>, String> {
        let r#ref = target.extra.get("sounds").and_then(|m| m.get(&message.event)).and_then(Value::as_str)
            .filter(|s| !s.is_empty()).unwrap_or(sounds::FALLBACK).to_string();
        let volume = target.extra.get("volume").and_then(Value::as_f64).unwrap_or(100.0).max(0.0) as u32;
        if sounds::resolve(&self.paths, &r#ref).is_none() {
            sounds::play(&self.paths, sounds::FALLBACK, volume)?;
            return Ok(Some(format!("声音 {} 不存在，已改用 Glass", r#ref)));
        }
        sounds::play(&self.paths, &r#ref, volume)?;
        Ok(None)
    }
}
