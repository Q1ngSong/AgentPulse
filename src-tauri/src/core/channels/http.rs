//! 外部提醒方式共用的 JSON HTTP 请求（阻塞式，8 秒超时）。
use std::time::Duration;

use serde_json::Value;

pub fn http_json(url: &str, body: &Value) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(8)).build().map_err(|e| e.to_string())?;
    let resp = client.post(url).json(body).send().map_err(|e| format!("URLError: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP 请求失败：{}", resp.status()));
    }
    let text = resp.text().map_err(|e| e.to_string())?;
    if text.trim().is_empty() { return Err("服务返回空响应，未确认发送成功".into()); }
    let data: Value = serde_json::from_str(&text).map_err(|e| format!("响应不是 JSON：{e}"))?;
    if !data.is_object() { return Err("响应必须是 JSON 对象".into()); }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    #[test]
    fn http_requires_success_status_and_json_object() {
        for (status, body, ok) in [
            (200, "{\"code\":0}", true),
            (500, "", false), (503, "{}", false), (500, "{\"code\":0}", false), (429, "", false),
            (302, "{}", false), (204, "", false), (200, "  ", false),
            (200, "not-json", false), (200, "null", false), (200, "[]", false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut reader = BufReader::new(&stream);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" { break; }
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") { length = value.trim().parse().unwrap(); }
                    }
                }
                reader.read_exact(&mut vec![0; length]).unwrap();
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            });
            let result = http_json(&url, &serde_json::json!({"test": true}));
            server.join().unwrap();
            assert_eq!(result.is_ok(), ok, "HTTP {status}, body={body:?}: {result:?}");
        }
    }
}
