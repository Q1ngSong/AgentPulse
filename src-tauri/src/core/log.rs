//! 记录：提醒记录 events.jsonl 与活动记录 activity.jsonl 的读写。
use std::fs::{self, OpenOptions};
use std::io::{self, Write};

use serde_json::{json, Value};

use super::paths::{FileLock, Paths};

/// 截断过长的字符串和列表，避免原始 Hook 数据撑大记录文件。
pub fn trim(v: &Value, n: usize) -> Value {
    match v {
        Value::String(s) => {
            if s.chars().count() <= n {
                v.clone()
            } else {
                Value::String(format!("{}…(已截断)", s.chars().take(n).collect::<String>()))
            }
        }
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), trim(x, n))).collect()),
        Value::Array(a) => Value::Array(a.iter().take(50).map(|x| trim(x, n)).collect()),
        _ => v.clone(),
    }
}

fn append_line(path: &std::path::Path, line: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")
}

fn tail_lines(path: &std::path::Path, keep: usize) -> io::Result<()> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(keep);
    fs::write(path, format!("{}\n", lines[start..].join("\n")))
}

fn read_jsonl(path: &std::path::Path, limit: usize) -> Vec<Value> {
    let Ok(_lock) = FileLock::acquire(&path.with_extension("lock")) else { return Vec::new() };
    let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(limit);
    lines[start..].iter().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// 追加一条提醒记录；文件超过 2MB 时只保留最后 500 条。
pub fn append_event(paths: &Paths, entry: &Value) -> io::Result<()> {
    let path = paths.events();
    // 所有进程共用稳定的锁文件，完整追加和轮转在同一临界区内。
    let _lock = FileLock::acquire(&path.with_extension("lock"))?;
    append_line(&path, &entry.to_string())?;
    if fs::metadata(&path)?.len() > 2_000_000 {
        tail_lines(&path, 500)?;
    }
    Ok(())
}

/// 最近 limit 条提醒记录，最新的在前。
pub fn read_events(paths: &Paths, limit: usize) -> Vec<Value> {
    let mut v = read_jsonl(&paths.events(), limit);
    v.reverse();
    v
}

pub fn clear_events(paths: &Paths) -> io::Result<()> {
    let _lock = FileLock::acquire(&paths.events().with_extension("lock"))?;
    if paths.events().exists() {
        fs::write(paths.events(), "")?;
    }
    Ok(())
}

/// 记录活动及 Hook 所在的 App：kind="prompt" 为 UserPromptSubmit，"tool" 为 PostToolUse。
pub fn record_activity(paths: &Paths, agent: &str, host_bundle: &str, payload: &Value, kind: &str) -> io::Result<()> {
    let _lock = FileLock::acquire(&paths.activity().with_extension("lock"))?;
    let rec = json!({
        "ts": now(), "agent": agent, "host_bundle": host_bundle, "kind": kind,
        "session_id": payload.get("session_id").cloned().unwrap_or(Value::Null),
        "tool_name": payload.get("tool_name").cloned().unwrap_or(Value::Null),
    });
    append_line(&paths.activity(), &rec.to_string())?;
    let count = fs::read_to_string(paths.activity())?.lines().count();
    if count > 400 {
        tail_lines(&paths.activity(), 200)?;
    }
    Ok(())
}

/// 最近 limit 条活动记录，时间正序。
pub fn read_activity(paths: &Paths, limit: usize) -> Vec<Value> {
    read_jsonl(&paths.activity(), limit)
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::super::paths::temp_paths;
    use super::*;

    #[test]
    fn events_roundtrip_newest_first_and_clear() {
        let (_d, p) = temp_paths();
        append_event(&p, &json!({"id": "a"})).unwrap();
        append_event(&p, &json!({"id": "b"})).unwrap();
        let v = read_events(&p, 10);
        assert_eq!(v[0]["id"], "b");
        assert_eq!(read_events(&p, 1).len(), 1);
        clear_events(&p).unwrap();
        assert!(read_events(&p, 10).is_empty());
    }

    #[test]
    fn activity_records_kind_and_rotates() {
        let (_d, p) = temp_paths();
        for i in 0..401 {
            record_activity(&p, "claude", "com.apple.Terminal", &json!({"session_id": i}), if i % 2 == 0 { "prompt" } else { "tool" }).unwrap();
        }
        let a = read_activity(&p, 1000);
        assert_eq!(a.len(), 200, "超过 400 条后只留 200");
        assert_eq!(a.last().unwrap()["kind"], "prompt");
        assert_eq!(a.last().unwrap()["host_bundle"], "com.apple.Terminal");
        assert_eq!(a.last().unwrap()["session_id"], 400);
    }

    #[test]
    fn concurrent_events_keep_every_complete_record() {
        let (_d, p) = temp_paths();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for writer in 0..8 {
                let (p, barrier) = (&p, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for i in 0..300 {
                        append_event(p, &json!({"id": writer * 300 + i})).unwrap();
                    }
                });
            }
        });
        let records = read_events(&p, 2400);
        assert_eq!(records.len(), 2400);
        let ids: std::collections::BTreeSet<_> = records.iter().map(|v| v["id"].as_u64().unwrap()).collect();
        assert_eq!(ids.len(), 2400);
    }

    #[test]
    fn concurrent_activity_remains_complete_during_rotation() {
        let (_d, p) = temp_paths();
        std::thread::scope(|scope| {
            for writer in 0..6 {
                let p = &p;
                scope.spawn(move || {
                    for i in 0..100 {
                        record_activity(p, "claude", "com.apple.Terminal", &json!({"session_id": writer * 100 + i}), "prompt").unwrap();
                    }
                });
            }
        });
        let records = read_activity(&p, 1000);
        assert_eq!(records.len(), 399, "第 401 条轮转为 200 条，随后再写入 199 条");
        let ids: std::collections::BTreeSet<_> = records.iter().map(|v| v["session_id"].as_u64().unwrap()).collect();
        assert_eq!(ids.len(), records.len());
    }

    #[test]
    fn trim_cuts_long_strings_and_arrays() {
        let v = json!({"s": "x".repeat(10), "a": (0..60).collect::<Vec<_>>()});
        let t = trim(&v, 4);
        assert_eq!(t["s"], "xxxx…(已截断)");
        assert_eq!(t["a"].as_array().unwrap().len(), 50);
    }
}
