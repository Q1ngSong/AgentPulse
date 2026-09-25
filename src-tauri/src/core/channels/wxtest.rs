//! 微信（公众平台测试号）：直接调用微信接口发模板消息。
//! 微信只显示模板里「名称：{{字段.DATA}}」这样的行，可用字段 device、app、project、title、body、time。
use std::fs;

use serde_json::{json, Map, Value};

use super::http::http_json;
use super::{Channel, Message};
use crate::core::config::Target;
use crate::core::paths::{atomic_write_json, chmod_private, Paths};

pub struct WxTest {
    pub paths: Paths,
}

impl WxTest {
    /// access_token 有效期约 2 小时，按 appid 缓存到本地，提前 5 分钟过期。
    fn access_token(&self, appid: &str, secret: &str, refresh: bool) -> Result<String, String> {
        let cache_path = self.paths.wx_token_cache();
        let mut cache: Map<String, Value> = fs::read_to_string(&cache_path).ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default();
        let now = crate::core::log::now();
        if !refresh {
            if let Some(hit) = cache.get(appid) {
                if hit.get("expires_at").and_then(Value::as_f64).unwrap_or(0.0) > now {
                    if let Some(t) = hit.get("token").and_then(Value::as_str) {
                        return Ok(t.to_string());
                    }
                }
            }
        }
        let data = http_json("https://api.weixin.qq.com/cgi-bin/stable_token",
            &json!({"grant_type": "client_credential", "appid": appid, "secret": secret, "force_refresh": false}))?;
        let token = data.get("access_token").and_then(Value::as_str).ok_or_else(|| format!(
            "RuntimeError: 获取 access_token 失败：errcode={} {}（检查 appID / appsecret）",
            data.get("errcode").map(|v| v.to_string()).unwrap_or_default(), data.get("errmsg").and_then(Value::as_str).unwrap_or("")))?;
        let expires = data.get("expires_in").and_then(Value::as_f64).unwrap_or(7200.0);
        cache.insert(appid.into(), json!({"token": token, "expires_at": now + expires - 300.0}));
        atomic_write_json(&cache_path, &Value::Object(cache)).map_err(|e| e.to_string())?;
        chmod_private(&cache_path);
        Ok(token.to_string())
    }
}

impl Channel for WxTest {
    fn send(&self, target: &Target, message: &Message) -> Result<Option<String>, String> {
        let appid = target.str("appid").trim().to_string();
        let secret = target.str("secret").trim().to_string();
        let template_id = target.str("template_id").trim().to_string();
        let openids: Vec<String> = target.str("openid").replace('，', ",").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        let missing: Vec<&str> = [("appID", appid.is_empty()), ("appsecret", secret.is_empty()), ("openid", openids.is_empty()), ("模板 ID", template_id.is_empty())]
            .iter().filter(|(_, m)| *m).map(|(n, _)| *n).collect();
        if !missing.is_empty() {
            return Err(format!("ValueError: 未填写：{}", missing.join("、")));
        }
        let data = template_data(message, &chrono::Local::now().format("%m-%d %H:%M:%S").to_string());
        let (mut errors, mut sent) = (Vec::new(), Vec::new());
        for openid in &openids {
            for attempt in 0..2 {
                let token = self.access_token(&appid, &secret, attempt == 1)?;
                let r = http_json(&format!("https://api.weixin.qq.com/cgi-bin/message/template/send?access_token={token}"),
                    &json!({"touser": openid, "template_id": template_id, "data": data}))?;
                let code = response_code(&r)?;
                if [40001, 40014, 42001].contains(&code) && attempt == 0 {
                    continue; // token 失效，刷新后重试一次
                }
                let short: String = openid.chars().take(8).collect();
                if code != 0 {
                    errors.push(format!("{short}…: errcode={code} {}", r.get("errmsg").and_then(Value::as_str).unwrap_or("")));
                } else {
                    sent.push(format!("{short}… msgid={}", r.get("msgid").map(|v| v.to_string()).unwrap_or_default()));
                }
                break;
            }
        }
        if !errors.is_empty() {
            return Err(format!("RuntimeError: 微信返回错误 {}", errors.join("；")));
        }
        Ok(Some(format!("微信已接收：{}", sent.join("；"))))
    }
}

/// 模板消息的数据：设备、应用、项目各占一个字段，模板里各写一行「名称：{{字段.DATA}}」。
/// 微信会丢掉不带名称的行、可能截断过长的值，所以不把来源拼进正文。
fn template_data(message: &Message, time: &str) -> Value {
    let ctx = &message.ctx;
    let or_dash = |s: &str| if s.is_empty() { "—".to_string() } else { s.to_string() };
    json!({"device": {"value": or_dash(&ctx.device)}, "app": {"value": or_dash(&ctx.app)}, "project": {"value": or_dash(&ctx.project)},
        "title": {"value": message.title}, "body": {"value": if message.body.is_empty() { " " } else { &message.body }}, "time": {"value": time}})
}

fn response_code(data: &Value) -> Result<i64, String> {
    data.get("errcode").and_then(Value::as_i64)
        .ok_or_else(|| "微信响应缺少有效的 errcode，未确认发送成功".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_requires_numeric_errcode() {
        assert_eq!(response_code(&json!({"errcode": 0, "msgid": 1})).unwrap(), 0);
        assert_eq!(response_code(&json!({"errcode": 40014})).unwrap(), 40014);
        for value in [json!({}), json!({"errcode": null}), json!({"errcode": "0"}), json!(false)] {
            assert!(response_code(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn template_data_puts_each_identifier_in_its_own_field() {
        let mut message = Message { title: "Claude Code: 修 bug".into(), body: "✅ 任务完成 · 好了".into(), ..Default::default() };
        (message.ctx.device, message.ctx.app, message.ctx.project) = ("Mac mini".into(), "Claude".into(), "AgentPulse".into());
        let data = template_data(&message, "09-25 15:40:27");
        for (key, value) in [("device", "Mac mini"), ("app", "Claude"), ("project", "AgentPulse"),
            ("title", "Claude Code: 修 bug"), ("body", "✅ 任务完成 · 好了"), ("time", "09-25 15:40:27")] {
            assert_eq!(data[key]["value"], value, "{key}");
        }
        // 取不到的来源写成「—」，不留空行；空正文沿用原来的空格
        let data = template_data(&Message::default(), "t");
        assert_eq!((data["device"]["value"].as_str(), data["project"]["value"].as_str(), data["body"]["value"].as_str()), (Some("—"), Some("—"), Some(" ")));
    }
}
