//! 提醒核心（从 Python 版 agentpulse/ 一比一翻译）。
//! 依赖方向：paths ← config / log ← events ← channels ← dispatch → queue → attention → platform。
pub mod attention;
pub mod channels;
pub mod codex;
pub mod config;
pub mod dispatch;
pub mod events;
pub mod icons;
pub mod integrations;
pub mod log;
pub mod paths;
pub mod pet;
pub mod platform;
pub mod queue;
pub mod runtime;
pub mod sounds;

pub mod pet_animation;
