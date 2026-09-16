//! 只读订阅 Codex 桌面会话的待审批请求，不参与审批决策。
//! 使用本机桌面 IPC v11；不认识的协议或丢失的状态一律报错，不推测用户是否需要批准。
use std::io;
use std::path::Path;
use serde_json::{json, Value};

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

/// 只保留 requests；会话正文、工具输出和内部自动审核项目不缓存、不记录。
#[derive(Default)]
struct Requests {
    revision: Option<u64>,
    values: Value,
}

impl Requests {
    fn apply(&mut self, change: &Value) -> io::Result<()> {
        let revision = change["revision"].as_u64().ok_or_else(|| invalid("missing stream revision"))?;
        if self.revision.is_some_and(|previous| revision < previous) { return Err(invalid("stale Codex approval state")); }
        let values = match change["type"].as_str() {
            Some("snapshot") => change["conversationState"]["requests"].clone(),
            Some("patches") => {
                if self.revision.is_none() || self.revision != change["baseRevision"].as_u64() {
                    return Err(invalid("Codex approval stream revision gap"));
                }
                let mut values = self.values.clone();
                for patch in change["patches"].as_array().ok_or_else(|| invalid("missing patches"))? {
                    let path = patch["path"].as_array().ok_or_else(|| invalid("invalid patch path"))?;
                    if path.is_empty() { return Err(invalid("unsupported root patch")); }
                    if path[0].as_str() == Some("requests") { apply_patch(&mut values, &path[1..], patch)?; }
                }
                values
            }
            _ => return Err(invalid("unknown Codex stream change")),
        };
        if !values.is_array() { return Err(invalid("invalid Codex requests")); }
        self.values = values;
        self.revision = Some(revision);
        Ok(())
    }

    fn approvals(&self, thread: &str) -> Vec<Value> {
        self.values.as_array().into_iter().flatten().filter(|r| {
            matches!(r["method"].as_str(), Some("item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval" | "item/permissions/requestApproval"))
                && r["params"]["threadId"].as_str() == Some(thread)
                && (r["id"].is_string() || r["id"].is_number())
                && r["completed"] != true
        }).cloned().collect()
    }
}

fn apply_patch(value: &mut Value, path: &[Value], patch: &Value) -> io::Result<()> {
    let op = patch["op"].as_str().unwrap_or("");
    if path.is_empty() {
        if matches!(op, "add" | "replace") && patch.get("value").is_some() {
            *value = patch["value"].clone();
            return Ok(());
        }
        return Err(invalid("invalid requests replacement"));
    }
    let last = path.len() == 1;
    if let Some(array) = value.as_array_mut() {
        let index = path[0].as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| invalid("invalid array index"))?;
        if last {
            match op {
                "add" if index <= array.len() && patch.get("value").is_some() => { array.insert(index, patch["value"].clone()); return Ok(()); }
                "remove" if index < array.len() => { array.remove(index); return Ok(()); }
                _ => {}
            }
        }
        return apply_patch(array.get_mut(index).ok_or_else(|| invalid("missing array element"))?, &path[1..], patch);
    }
    let object = value.as_object_mut().ok_or_else(|| invalid("invalid patch parent"))?;
    let key = path[0].as_str().ok_or_else(|| invalid("invalid object key"))?;
    if last {
        match op {
            "add" if patch.get("value").is_some() => { object.insert(key.into(), patch["value"].clone()); return Ok(()); }
            "remove" if object.remove(key).is_some() => return Ok(()),
            _ => {}
        }
    }
    apply_patch(object.get_mut(key).ok_or_else(|| invalid("missing object field"))?, &path[1..], patch)
}

#[cfg(unix)]
mod transport {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    pub struct Observer {
        socket: UnixStream,
        buffer: Vec<u8>,
        client: String,
        owner: String,
        thread: String,
        state: Requests,
    }

    impl Observer {
        pub fn connect(home: &Path, thread: &str) -> io::Result<Self> {
            let root = std::env::var_os("CODEX_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".codex"));
            Self::at(&root.join("ipc/ipc.sock"), thread)
        }

        fn at(path: &Path, thread: &str) -> io::Result<Self> {
            let socket = UnixStream::connect(path)?;
            socket.set_read_timeout(Some(Duration::from_millis(200)))?;
            socket.set_write_timeout(Some(Duration::from_secs(2)))?;
            let mut this = Self { socket, buffer: Vec::new(), client: String::new(), owner: String::new(), thread: thread.into(), state: Requests::default() };
            let init = this.request("initialize", 0, json!({"clientType":"agentpulse-observer"}))?;
            this.client = init["result"]["clientId"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| invalid("missing IPC client id"))?.into();
            let owner = this.request("thread-owner-discovery", 1, json!({"hostId":"local", "conversationId":thread}))?;
            this.owner = owner["handledByClientId"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| invalid("missing thread owner"))?.into();
            this.follow(true)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while this.state.revision.is_none() {
                if Instant::now() >= deadline { return Err(io::Error::new(io::ErrorKind::TimedOut, "Codex did not send approval snapshot")); }
                this.poll()?;
            }
            Ok(this)
        }

        fn send(&mut self, value: Value) -> io::Result<()> {
            let bytes = serde_json::to_vec(&value)?;
            self.socket.write_all(&(bytes.len() as u32).to_le_bytes())?;
            self.socket.write_all(&bytes)
        }

        // 读取超时保留半帧，避免会话大快照被截断后错读长度。
        fn read(&mut self) -> io::Result<Option<Value>> {
            loop {
                if self.buffer.len() >= 4 {
                    let size = u32::from_le_bytes(self.buffer[..4].try_into().unwrap()) as usize;
                    if size > 64 * 1024 * 1024 { return Err(invalid("Codex IPC frame too large")); }
                    if self.buffer.len() >= size + 4 {
                        let value = serde_json::from_slice(&self.buffer[4..size + 4])?;
                        self.buffer.drain(..size + 4);
                        return Ok(Some(value));
                    }
                }
                let mut bytes = [0u8; 16384];
                match self.socket.read(&mut bytes) {
                    Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Codex IPC disconnected")),
                    Ok(n) => self.buffer.extend_from_slice(&bytes[..n]),
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
        }

        fn discovery(&mut self, value: &Value) -> io::Result<bool> {
            if value["type"] != "client-discovery-request" { return Ok(false); }
            self.send(json!({"type":"client-discovery-response", "requestId":value["requestId"], "response":{"canHandle":false}}))?;
            Ok(true)
        }

        fn request(&mut self, method: &str, version: u64, params: Value) -> io::Result<Value> {
            let id = uuid::Uuid::new_v4().to_string();
            let mut request = json!({"type":"request", "requestId":id, "method":method, "version":version, "params":params, "timeoutMs":3000});
            if !self.client.is_empty() { request["sourceClientId"] = json!(self.client); }
            self.send(request)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Some(value) = self.read()? {
                    if self.discovery(&value)? { continue; }
                    if value["type"] == "response" && value["requestId"] == id {
                        return if value["resultType"] == "success" { Ok(value) } else { Err(invalid("Codex desktop does not own this thread")) };
                    }
                }
            }
            Err(io::Error::new(io::ErrorKind::TimedOut, "Codex IPC discovery timed out"))
        }

        fn follow(&mut self, following: bool) -> io::Result<()> {
            self.send(json!({"type":"broadcast", "method":"thread-stream-following-changed", "version":1,
                "sourceClientId":self.client, "targetClientIds":[self.owner],
                "params":{"hostId":"local", "conversationId":self.thread, "following":following}}))
        }

        pub fn approvals(&self) -> Vec<Value> { self.state.approvals(&self.thread) }

        /// 只应用已发现的拥有者、当前会话、当前版本的状态。返回是否有新状态。
        pub fn poll(&mut self) -> io::Result<bool> {
            let Some(value) = self.read()? else { return Ok(false) };
            if self.discovery(&value)? || value["type"] != "broadcast" { return Ok(false); }
            if value["method"] == "ipc-connection-reset" { return Err(invalid("Codex IPC reset")); }
            if value["sourceClientId"] != self.owner { return Ok(false); }
            let params = &value["params"];
            if value["method"] == "client-status-changed" && params["clientId"] == self.owner && params["status"] == "disconnected" {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Codex thread owner disconnected"));
            }
            if params["hostId"] != "local" || params["conversationId"] != self.thread { return Ok(false); }
            if value["method"] == "thread-stream-following-status-requested" {
                self.follow(true)?;
            } else if value["method"] == "thread-stream-state-changed" {
                if value["version"] != 11 { return Err(invalid("unsupported Codex approval stream version")); }
                self.state.apply(&params["change"])?;
                return Ok(true);
            }
            Ok(false)
        }
    }

    impl Drop for Observer {
        fn drop(&mut self) {
            if !self.owner.is_empty() { let _ = self.follow(false); }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn incompatible_protocol_and_owner_disconnect_stop_observation() {
            for message in [
                json!({"type":"broadcast","method":"client-status-changed","sourceClientId":"owner","params":{"clientId":"owner","status":"disconnected"}}),
                json!({"type":"broadcast","method":"thread-stream-state-changed","version":12,"sourceClientId":"owner","params":{"hostId":"local","conversationId":"t"}}),
            ] {
                let (socket, mut writer) = UnixStream::pair().unwrap();
                let mut observer = Observer { socket, buffer: Vec::new(), client: "observer".into(), owner: "owner".into(), thread: "t".into(), state: Requests::default() };
                let bytes = serde_json::to_vec(&message).unwrap();
                writer.write_all(&(bytes.len() as u32).to_le_bytes()).unwrap();
                writer.write_all(&bytes).unwrap();
                assert!(observer.poll().is_err());
                assert!(observer.approvals().is_empty());
            }
        }

        #[test]
        fn observer_only_follows_and_tracks_matching_owner_and_thread() {
            use std::os::unix::net::UnixListener;
            fn receive(socket: &mut UnixStream) -> Value {
                let mut len = [0; 4];
                socket.read_exact(&mut len).unwrap();
                let mut bytes = vec![0; u32::from_le_bytes(len) as usize];
                socket.read_exact(&mut bytes).unwrap();
                serde_json::from_slice(&bytes).unwrap()
            }
            fn send(socket: &mut UnixStream, value: Value) {
                let bytes = serde_json::to_vec(&value).unwrap();
                socket.write_all(&(bytes.len() as u32).to_le_bytes()).unwrap();
                socket.write_all(&bytes).unwrap();
            }
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ipc.sock");
            let listener = UnixListener::bind(&path).unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let init = receive(&mut socket);
                assert_eq!(init["method"], "initialize");
                send(&mut socket, json!({"type":"response","requestId":init["requestId"],"resultType":"success","result":{"clientId":"observer"}}));
                let owner = receive(&mut socket);
                assert_eq!(owner["method"], "thread-owner-discovery");
                send(&mut socket, json!({"type":"response","requestId":owner["requestId"],"resultType":"success","handledByClientId":"owner"}));
                let follow = receive(&mut socket);
                assert_eq!(follow["method"], "thread-stream-following-changed");
                assert_eq!(follow["params"]["following"], true);
                let request = json!({"id":3,"method":"item/commandExecution/requestApproval","params":{"threadId":"t","turnId":"turn"}});
                send(&mut socket, json!({"type":"broadcast","method":"thread-stream-state-changed","version":11,"sourceClientId":"owner",
                    "params":{"hostId":"local","conversationId":"t","change":{"type":"snapshot","revision":1,"conversationState":{"requests":[request]}}}}));
                send(&mut socket, json!({"type":"client-discovery-request","requestId":"must-not-own"}));
                assert_eq!(receive(&mut socket), json!({"type":"client-discovery-response","requestId":"must-not-own","response":{"canHandle":false}}));
                let mut removal = json!({"type":"broadcast","method":"thread-stream-state-changed","version":11,"sourceClientId":"stranger",
                    "params":{"hostId":"local","conversationId":"t","change":{"type":"patches","baseRevision":1,"revision":2,"patches":[{"op":"remove","path":["requests",0]}]}}});
                send(&mut socket, removal.clone());
                removal["sourceClientId"] = json!("owner");
                removal["params"]["conversationId"] = json!("other");
                send(&mut socket, removal.clone());
                removal["params"]["conversationId"] = json!("t");
                send(&mut socket, removal);
                let unfollow = receive(&mut socket);
                assert_eq!(unfollow["method"], "thread-stream-following-changed");
                assert_eq!(unfollow["params"]["following"], false);
            });
            let mut observer = Observer::at(&path, "t").unwrap();
            assert_eq!(observer.approvals().len(), 1);
            observer.poll().unwrap(); // 拒绝作为请求处理者
            observer.poll().unwrap(); // 非拥有者
            assert_eq!(observer.approvals().len(), 1);
            observer.poll().unwrap(); // 不同会话
            assert_eq!(observer.approvals().len(), 1);
            observer.poll().unwrap();
            assert!(observer.approvals().is_empty());
            drop(observer);
            server.join().unwrap();
        }

        #[test]
        fn partial_frames_survive_read_timeouts_and_disconnect_fails() {
            let (socket, mut writer) = UnixStream::pair().unwrap();
            socket.set_read_timeout(Some(Duration::from_millis(10))).unwrap();
            let mut observer = Observer { socket, buffer: Vec::new(), client: String::new(), owner: String::new(), thread: "t".into(), state: Requests::default() };
            let payload = br#"{"type":"broadcast"}"#;
            let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
            bytes.extend_from_slice(payload);
            writer.write_all(&bytes[..2]).unwrap();
            assert!(observer.read().unwrap().is_none());
            writer.write_all(&bytes[2..8]).unwrap();
            assert!(observer.read().unwrap().is_none());
            writer.write_all(&bytes[8..]).unwrap();
            assert_eq!(observer.read().unwrap().unwrap()["type"], "broadcast");
            drop(writer);
            assert!(observer.read().is_err());
        }
    }
}

#[cfg(unix)]
pub use transport::Observer;

#[cfg(not(unix))]
pub struct Observer;
#[cfg(not(unix))]
impl Observer {
    pub fn connect(_: &Path, _: &str) -> io::Result<Self> { Err(io::Error::new(io::ErrorKind::Unsupported, "Codex desktop approval observation requires Unix IPC")) }
    pub fn approvals(&self) -> Vec<Value> { Vec::new() }
    pub fn poll(&mut self) -> io::Result<bool> { Ok(false) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(id: Value, method: &str, thread: &str) -> Value {
        json!({"id":id,"method":method,"params":{"threadId":thread,"turnId":"turn"}})
    }
    fn snapshot(requests: Value) -> Value {
        json!({"type":"snapshot","revision":1,"conversationState":{"requests":requests}})
    }
    #[test]
    fn internal_reviews_and_other_threads_are_not_user_approvals() {
        let mut state = Requests::default();
        state.apply(&snapshot(json!([
            request(json!(1), "item/autoApprovalReview/started", "t"),
            request(json!(2), "item/autoApprovalReview/completed", "t"),
            request(json!(3), "item/commandExecution/requestApproval", "other"),
            request(json!(4), "item/tool/requestUserInput", "t")
        ]))).unwrap();
        assert!(state.approvals("t").is_empty());
    }
    #[test]
    fn real_approvals_are_added_and_removed_by_stream_patches() {
        let mut state = Requests::default();
        state.apply(&snapshot(json!([]))).unwrap();
        for (i, method) in ["item/commandExecution/requestApproval", "item/fileChange/requestApproval", "item/permissions/requestApproval"].iter().enumerate() {
            let r = request(json!(i), method, "t");
            state.apply(&json!({"type":"patches","baseRevision":i+1,"revision":i+2,"patches":[{"op":"add","path":["requests",i],"value":r}]})).unwrap();
        }
        assert_eq!(state.approvals("t").len(), 3);
        state.apply(&json!({"type":"patches","baseRevision":4,"revision":5,"patches":[{"op":"remove","path":["requests",1]}]})).unwrap();
        assert_eq!(state.approvals("t").iter().map(|r| r["id"].as_u64().unwrap()).collect::<Vec<_>>(), [0, 2]);
        state.apply(&json!({"type":"patches","baseRevision":5,"revision":6,"patches":[{"op":"replace","path":["requests"],"value":[]}]})).unwrap();
        assert!(state.approvals("t").is_empty());
    }
    #[test]
    fn revision_gaps_and_malformed_patches_are_not_approval_evidence() {
        let mut state = Requests::default();
        state.apply(&snapshot(json!([]))).unwrap();
        assert!(state.apply(&json!({"type":"patches","baseRevision":99,"revision":100,"patches":[]})).is_err());
        assert!(state.apply(&json!({"type":"snapshot","revision":0,"conversationState":{"requests":[]}})).is_err());
        assert!(state.apply(&json!({"type":"patches","baseRevision":1,"revision":2,"patches":[{"op":"remove","path":["requests",100]}]})).is_err());
        assert!(state.approvals("t").is_empty());
    }
}
