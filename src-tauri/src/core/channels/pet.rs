//! 桌宠提醒经过统一分发，使用稳定实例 id 投递到对应桌宠窗口。
use serde_json::json;
use super::{Channel, Message};
use crate::core::{config::Target, paths::Paths, pet::{Phase, Update}, pet_animation};
pub struct Pet { pub paths: Paths }
impl Channel for Pet {
    fn send(&self, target: &Target, message: &Message) -> Result<Option<String>, String> {
        let update = Update {
            id: uuid::Uuid::new_v4().to_string(),
            phase: if message.event == "permission_request" { Phase::PermissionRequest } else { Phase::TaskComplete },
            agent: message.agent.clone(), host: message.ctx.host_bundle.clone(), session: message.ctx.session_id.clone().unwrap_or_default(),
            hook_event: message.ctx.hook_event.clone(), tool_use_id: message.ctx.tool_use_id.clone(), observed_at: message.ctx.observed_at,
            tool_input_key: message.ctx.tool_input_key.clone(),
            title: message.title.clone(), detail: message.body.clone(), animations: Some(pet_animation::from_target(target)?),
        };
        let reply = super::desktop::send_to_app(&self.paths, &json!({"kind":"pet_notify","instance":target.id,"update":update}), true)?;
        Ok(Some(if reply["skipped"] == true { "后台通知模式，桌宠已暂停" } else { "已交给桌宠显示" }.into()))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    #[test]
    fn paused_pet_reply_is_not_reported_as_displayed() {
        let (_dir, paths) = crate::core::paths::temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let listener = UnixListener::bind(paths.data_dir.join("app.sock")).unwrap();
        let receive = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&socket).read_line(&mut line).unwrap();
            socket.write_all(b"{\"ok\":true,\"skipped\":true}\n").unwrap();
        });
        let target = serde_json::from_value(json!({"id":"pet-1", "type":"pet", "animations":pet_animation::defaults()})).unwrap();
        assert_eq!(Pet { paths }.send(&target, &Message::default()).unwrap().as_deref(), Some("后台通知模式，桌宠已暂停"));
        receive.join().unwrap();
    }

    #[test]
    fn permission_delivery_preserves_hook_identity_and_capture_time() {
        let (_dir, paths) = crate::core::paths::temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let listener = UnixListener::bind(paths.data_dir.join("app.sock")).unwrap();
        let receive = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&socket).read_line(&mut line).unwrap();
            socket.write_all(b"{\"ok\":true}\n").unwrap();
            serde_json::from_str::<serde_json::Value>(&line).unwrap()
        });
        let message = Message {
            event: "permission_request".into(), agent: "codex".into(),
            ctx: crate::core::events::Context {
                session_id: Some("session".into()), hook_event: "PermissionRequest".into(),
                tool_use_id: "call".into(), tool_input_key: "input".into(), observed_at: 12.5,
                ..Default::default()
            }, ..Default::default()
        };
        let target = serde_json::from_value(json!({"id":"pet-1", "type":"pet", "animations":pet_animation::defaults()})).unwrap();
        Pet { paths }.send(&target, &message).unwrap();
        let request = receive.join().unwrap();
        let update: Update = serde_json::from_value(request["update"].clone()).unwrap();
        assert_eq!(request["instance"], "pet-1");
        assert_eq!(update.phase, Phase::PermissionRequest);
        assert_eq!((update.agent.as_str(), update.host.as_str(), update.session.as_str()), ("codex", "", "session"));
        assert_eq!((update.hook_event.as_str(), update.tool_use_id.as_str(), update.tool_input_key.as_str()), ("PermissionRequest", "call", "input"));
        assert_eq!(update.observed_at, 12.5);
    }
}
