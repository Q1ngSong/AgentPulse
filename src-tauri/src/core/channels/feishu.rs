//! 飞书群机器人：发送卡片消息，支持签名校验。
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use super::http::http_json;
use super::{Channel, Message};
use crate::core::config::Target;

pub struct Feishu;

impl Channel for Feishu {
    fn send(&self, target: &Target, message: &Message) -> Result<Option<String>, String> {
        let url = target.str("webhook").trim().to_string();
        if !url.starts_with("https://") {
            return Err("ValueError: 未填写有效的飞书 Webhook 地址".into());
        }
        let color = if message.event == "permission_request" { "orange" } else { "green" };
        let body = if message.body.is_empty() { " " } else { &message.body };
        let mut msg = json!({
            "msg_type": "interactive",
            "card": {
                "header": {"title": {"tag": "plain_text", "content": message.title}, "template": color},
                "elements": [{"tag": "div", "text": {"tag": "lark_md", "content": body}}],
            },
        });
        let secret = target.str("secret").trim();
        if !secret.is_empty() {
            let ts = chrono::Utc::now().timestamp().to_string();
            let key = format!("{ts}\n{secret}");
            let mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|e| e.to_string())?;
            let sign = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
            msg["timestamp"] = Value::String(ts);
            msg["sign"] = Value::String(sign);
        }
        let data = http_json(&url, &msg)?;
        check_response(&data)?;
        Ok(None)
    }
}

fn check_response(data: &Value) -> Result<(), String> {
    let code = data.get("code").or(data.get("StatusCode")).and_then(Value::as_i64).ok_or("飞书响应缺少有效的 code / StatusCode，未确认发送成功")?;
    if code != 0 {
        let m = data.get("msg").or(data.get("StatusMessage")).and_then(Value::as_str).unwrap_or("");
        return Err(format!("RuntimeError: 飞书返回 code={code} {m}").trim().to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_success_is_accepted() {
        assert!(check_response(&json!({"code": 0})).is_ok());
        assert!(check_response(&json!({"StatusCode": 0})).is_ok());
        for value in [json!({}), json!({"code": null}), json!({"code": "0"}),
            json!({"code": "0", "StatusCode": 0}), json!({"code": 19001, "msg": "failed"}),
            json!({"StatusCode": 1, "StatusMessage": "failed"})] {
            assert!(check_response(&value).is_err(), "{value}");
        }
    }
}
