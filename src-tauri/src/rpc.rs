use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

use rmpv::Value;

use crate::models::RuntimeMapping;

/// RPC 到会话层的事件模型：区分上下文推送、映射推送、连接关闭与读取错误。
#[derive(Debug, Clone)]
pub enum RpcEvent {
    Context {
        session_id: String,
        json: String,
    },
    Mappings {
        session_id: String,
        mappings: Vec<RuntimeMapping>,
    },
    Closed(String),
    Error {
        session_id: String,
        error: String,
    },
}

/// RPC 请求/响应链路的配对结果：保留 Neovim 返回的错误或结果，唤醒对应等待请求。
#[derive(Debug)]
struct RpcResponse {
    error: Value,
    result: Value,
}

/// RPC 并发状态的共享索引：按请求 ID 保存响应通道，连接关闭时可统一唤醒等待者。
type PendingRequests = Arc<Mutex<HashMap<u64, Sender<RpcResponse>>>>;

/// RPC 与执行链路的单会话通道：持有 socket 写端、请求关联表和连接身份。
pub struct NvimConnection {
    pub session_id: String,
    pub profile_id: String,
    pub socket_path: PathBuf,
    writer: Arc<Mutex<UnixStream>>,
    pending: PendingRequests,
    next_id: Arc<AtomicU64>,
}

impl NvimConnection {
    /// RPC 建连入口：启动读取循环，注册 companion 并导出首批映射，失败时关闭等待请求。
    pub fn connect(
        session_id: String,
        profile_id: String,
        socket_path: impl AsRef<Path>,
        event_sender: Sender<RpcEvent>,
    ) -> Result<Self, String> {
        let socket_path = socket_path.as_ref().to_path_buf();
        let stream = UnixStream::connect(&socket_path)
            .map_err(|error| format!("连接 Neovim socket 失败：{error}"))?;
        let reader_stream = stream
            .try_clone()
            .map_err(|error| format!("复制 Neovim socket 失败：{error}"))?;
        let writer = Arc::new(Mutex::new(stream));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicU64::new(1));
        let connection = Self {
            session_id: session_id.clone(),
            profile_id: profile_id.clone(),
            socket_path,
            writer,
            pending: pending.clone(),
            next_id,
        };
        let pending_reader = pending.clone();
        let session_reader = session_id.clone();
        let event_sender_for_loop = event_sender.clone();
        thread::spawn(move || {
            read_loop(
                reader_stream,
                pending_reader,
                event_sender_for_loop,
                session_reader,
            );
        });
        let startup = || {
            connection.register()?;
            let mappings = connection.export_keymaps()?;
            event_sender
                .send(RpcEvent::Mappings {
                    session_id: session_id.clone(),
                    mappings,
                })
                .map_err(|_| "LazyKeys 会话事件接收端已关闭".to_string())?;
            Ok::<(), String>(())
        };
        if let Err(error) = startup() {
            if let Ok(writer) = connection.writer.lock() {
                let _ = writer.shutdown(Shutdown::Both);
            }
            close_pending(&connection.pending, "Neovim 初始化失败");
            return Err(error);
        }
        Ok(connection)
    }

    /// RPC 调用的默认入口：以统一超时发送请求并等待配对响应。
    fn request(
        &self,
        method: &str,
        params: Vec<Value>,
    ) -> Result<Value, String> {
        self.request_with_timeout(method, params, Duration::from_secs(10))
    }

    /// RPC 请求/响应核心：登记待处理请求、限时写入并等待响应，返回结果或 Neovim 错误。
    fn request_with_timeout(
        &self,
        method: &str,
        params: Vec<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| "Neovim pending lock poisoned".to_string())?
            .insert(id, sender);
        let _pending = PendingGuard {
            id,
            pending: &self.pending,
        };
        let message = Value::Array(vec![
            Value::from(0),
            Value::from(id),
            Value::from(method),
            Value::Array(params),
        ]);
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &message)
            .map_err(|error| error.to_string())?;
        let mut writer = lock_writer(&self.writer, deadline)?;
        writer
            .set_write_timeout(Some(
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ))
            .map_err(|error| format!("设置 Neovim 写超时失败：{error}"))?;
        writer
            .write_all(&bytes)
            .map_err(|error| format!("发送 Neovim 请求失败：{error}"))?;
        drop(writer);
        let response = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| format!("等待 Neovim 响应失败：{error}"))?;
        if response.error.is_nil() {
            Ok(response.result)
        } else {
            Err(format!("Neovim {} 返回错误：{:?}", method, response.error))
        }
    }

    /// RPC 初始化握手：取得 channel 并注册会话身份，开启后续上下文推送。
    fn register(&self) -> Result<(), String> {
        let api_info = self.request("nvim_get_api_info", vec![])?;
        let channel = api_info
            .as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_u64)
            .ok_or("无法取得 Neovim RPC channel")?;
        self.request(
            "nvim_exec_lua",
            vec![
                Value::from("require('lazykeys').register(...)"),
                Value::Array(vec![
                    Value::from(channel),
                    Value::from(self.session_id.as_str()),
                    Value::from(self.profile_id.as_str()),
                ]),
            ],
        )?;
        Ok(())
    }

    /// RPC 到搜索的映射导入：调用 companion 导出 JSON 并解码为运行时映射。
    pub fn export_keymaps(&self) -> Result<Vec<RuntimeMapping>, String> {
        let result = self.request(
            "nvim_exec_lua",
            vec![
                Value::from(
                    "return vim.json.encode(require('lazykeys').export_keymaps())",
                ),
                Value::Array(vec![]),
            ],
        )?;
        let json = result.as_str().ok_or("keymap 导出结果不是 JSON")?;
        serde_json::from_str(json)
            .map_err(|error| format!("解析 keymap 导出失败：{error}"))
    }

    /// 执行链路的最终写入：校验非空按键后通过 nvim_input 送入 Neovim。
    pub fn input(&self, lhs: &str) -> Result<(), String> {
        if lhs.trim().is_empty() {
            return Err("不能执行空命令".into());
        }
        self.request("nvim_input", vec![Value::from(lhs)])?;
        Ok(())
    }

    #[cfg(test)]
    pub fn for_test(session_id: &str, profile_id: &str) -> (Self, UnixStream) {
        let (stream, peer) = UnixStream::pair().unwrap();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let reader = stream.try_clone().unwrap();
        let (event_sender, _event_receiver) = mpsc::channel();
        thread::spawn({
            let pending = pending.clone();
            let session_id = session_id.to_string();
            move || read_loop(reader, pending, event_sender, session_id)
        });
        (
            Self {
                session_id: session_id.into(),
                profile_id: profile_id.into(),
                socket_path: PathBuf::from(format!("/tmp/{session_id}.sock")),
                writer: Arc::new(Mutex::new(stream)),
                pending,
                next_id: Arc::new(AtomicU64::new(1)),
            },
            peer,
        )
    }

    #[cfg(test)]
    fn eval(&self, script: &str) -> Result<Value, String> {
        self.request("nvim_eval", vec![Value::from(script)])
    }
}

/// RPC 写入串行化：在请求截止时间内等待 socket 写锁，避免跨请求字节交错。
fn lock_writer(
    writer: &Mutex<UnixStream>,
    deadline: Instant,
) -> Result<MutexGuard<'_, UnixStream>, String> {
    loop {
        match writer.try_lock() {
            Ok(writer) => return Ok(writer),
            Err(TryLockError::Poisoned(_)) => {
                return Err("Neovim writer lock poisoned".into());
            }
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err("等待 Neovim writer 锁超时".into());
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// RPC 待处理请求的生命周期守卫：无论成功、超时或写失败都清理请求 ID。
struct PendingGuard<'a> {
    id: u64,
    pending: &'a PendingRequests,
}

impl Drop for PendingGuard<'_> {
    /// RPC 请求收尾：移除已结束的等待项，防止超时或失败留下悬挂通道。
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.id);
        }
    }
}

/// RPC 断开收尾：清空全部等待请求并逐个返回错误，避免调用方永久等待。
fn close_pending(pending: &PendingRequests, reason: &str) {
    let senders = pending
        .lock()
        .map(|mut pending| {
            pending
                .drain()
                .map(|(_, sender)| sender)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for sender in senders {
        let _ = sender.send(RpcResponse {
            error: Value::from(reason),
            result: Value::Nil,
        });
    }
}

/// RPC 接收主循环：增量解码 socket 字节流，退出时唤醒等待者并上报关闭。
fn read_loop(
    mut reader: UnixStream,
    pending: PendingRequests,
    event_sender: Sender<RpcEvent>,
    session_id: String,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    let failure = 'read: loop {
        match reader.read(&mut chunk) {
            Ok(0) => break None,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                let mut cursor = std::io::Cursor::new(buffer.as_slice());
                loop {
                    let position = cursor.position() as usize;
                    match rmpv::decode::read_value(&mut cursor) {
                        Ok(value) => {
                            handle_message(
                                value,
                                &pending,
                                &event_sender,
                                &session_id,
                            );
                            let consumed = cursor.position() as usize;
                            buffer.drain(..consumed);
                            cursor = std::io::Cursor::new(buffer.as_slice());
                        }
                        Err(error)
                            if error.kind()
                                == std::io::ErrorKind::UnexpectedEof =>
                        {
                            cursor.set_position(position as u64);
                            break;
                        }
                        Err(error) => {
                            break 'read Some(format!(
                                "解析 Neovim RPC 失败：{error}"
                            ));
                        }
                    }
                }
            }
            Err(error) => break Some(format!("读取 Neovim RPC 失败：{error}")),
        }
    };
    if let Some(error) = failure.clone() {
        let _ = event_sender.send(RpcEvent::Error {
            session_id: session_id.clone(),
            error,
        });
    }
    close_pending(&pending, failure.as_deref().unwrap_or("Neovim 连接已关闭"));
    let _ = event_sender.send(RpcEvent::Closed(session_id));
}

/// 响应按请求 ID 配回等待通道，未知或已超时请求直接忽略。
fn handle_response(items: &[Value], pending: &PendingRequests) {
    let Some(id) = items.get(1).and_then(Value::as_u64) else {
        return;
    };
    let response = RpcResponse {
        error: items.get(2).cloned().unwrap_or(Value::Nil),
        result: items.get(3).cloned().unwrap_or(Value::Nil),
    };
    if let Some(sender) =
        pending.lock().ok().and_then(|mut map| map.remove(&id))
    {
        let _ = sender.send(response);
    }
}

/// 上下文通知只搬运 JSON，不提前解释编辑器状态。
fn send_context(
    items: &[Value],
    event_sender: &Sender<RpcEvent>,
    session_id: &str,
) {
    if let Some(json) = notification_json(items) {
        let _ = event_sender.send(RpcEvent::Context {
            session_id: session_id.to_string(),
            json: json.to_string(),
        });
    }
}

/// 映射通知在会话边界解码，格式错误通过事件显式上报。
fn send_mappings(
    items: &[Value],
    event_sender: &Sender<RpcEvent>,
    session_id: &str,
) {
    let Some(json) = notification_json(items) else {
        return;
    };
    match serde_json::from_str(json) {
        Ok(mappings) => {
            let _ = event_sender.send(RpcEvent::Mappings {
                session_id: session_id.to_string(),
                mappings,
            });
        }
        Err(error) => {
            let _ = event_sender.send(RpcEvent::Error {
                session_id: session_id.to_string(),
                error: format!("解析 Neovim keymap 推送失败：{error}"),
            });
        }
    }
}

/// companion 通知按方法分流，其他 Neovim 通知不影响读取循环。
fn handle_notification(
    items: &[Value],
    event_sender: &Sender<RpcEvent>,
    session_id: &str,
) {
    match items.get(1).and_then(Value::as_str) {
        Some("lazykeys_context") => {
            send_context(items, event_sender, session_id)
        }
        Some("lazykeys_mappings") => {
            send_mappings(items, event_sender, session_id)
        }
        _ => {}
    }
}

/// RPC 解复用阶段：把响应配回请求，把 companion 通知转成会话事件。
fn handle_message(
    value: Value,
    pending: &PendingRequests,
    event_sender: &Sender<RpcEvent>,
    session_id: &str,
) {
    let Some(items) = value.as_array() else {
        return;
    };
    match items.first().and_then(Value::as_u64) {
        Some(1) => handle_response(items, pending),
        Some(2) => handle_notification(items, event_sender, session_id),
        _ => {}
    }
}

/// RPC 通知解码辅助：从参数数组提取 companion 携带的 JSON 文本。
fn notification_json(items: &[Value]) -> Option<&str> {
    items
        .get(2)
        .and_then(Value::as_array)
        .and_then(|params| params.first())
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::process::{Command, Stdio};

    use super::*;

    struct ChildGuard(std::process::Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn timeout_removes_pending_request() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("pending.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0_u8; 256];
            let _ = stream.read(&mut bytes);
            thread::sleep(Duration::from_millis(150));
        });
        let stream = UnixStream::connect(&socket).unwrap();
        let connection = NvimConnection {
            session_id: "test".into(),
            profile_id: "test".into(),
            socket_path: socket,
            writer: Arc::new(Mutex::new(stream)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        };
        let error = connection
            .request_with_timeout(
                "nvim_get_api_info",
                vec![],
                Duration::from_millis(20),
            )
            .unwrap_err();
        assert!(error.contains("响应"));
        assert!(connection.pending.lock().unwrap().is_empty());
        server.join().unwrap();
    }

    #[test]
    fn blocked_socket_write_obeys_request_deadline_and_cleans_pending() {
        let (connection, _peer) =
            NvimConnection::for_test("write-blocked", "test");
        let started = Instant::now();
        let error = connection
            .request_with_timeout(
                "nvim_input",
                vec![Value::from("x".repeat(8 * 1024 * 1024))],
                Duration::from_millis(50),
            )
            .unwrap_err();
        assert!(error.contains("发送") || error.contains("锁"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(connection.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn malformed_frame_wakes_pending_and_closes_session() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (response_sender, response_receiver) = mpsc::channel();
        pending.lock().unwrap().insert(7, response_sender);
        let (event_sender, event_receiver) = mpsc::channel();
        let pending_reader = pending.clone();
        let loop_thread = thread::spawn(move || {
            read_loop(reader, pending_reader, event_sender, "session-a".into());
        });
        writer.write_all(&[0xc1]).unwrap();
        writer.shutdown(Shutdown::Both).unwrap();
        let response = response_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(!response.error.is_nil());
        let mut saw_closed = false;
        while let Ok(event) = event_receiver.try_recv() {
            if matches!(event, RpcEvent::Closed(ref id) if id == "session-a") {
                saw_closed = true;
            }
        }
        assert!(saw_closed);
        assert!(pending.lock().unwrap().is_empty());
        loop_thread.join().unwrap();
    }

    /// 真实 Neovim 夹具只负责 companion、临时 socket 与子进程生命周期。
    fn real_nvim_fixture() -> (tempfile::TempDir, PathBuf, ChildGuard) {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("lazykeys.sock");
        let companion = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../companion/lazykeys.nvim")
            .canonicalize()
            .unwrap();
        let init = temp.path().join("init.lua");
        std::fs::write(
            &init,
            format!(
                "vim.opt.rtp:prepend('{}')\nrequire('lazykeys').setup({{ profile_id = 'test', session_id = 'test-session' }})\n",
                companion.display()
            ),
        )
        .unwrap();
        let child = ChildGuard(
            Command::new("nvim")
                .args([
                    "--headless",
                    "--listen",
                    socket.to_str().unwrap(),
                    "-u",
                    init.to_str().unwrap(),
                    "-i",
                    "NONE",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        (temp, socket, child)
    }

    /// 限时等待真实 socket 可用，成功后返回唯一连接。
    fn connect_real_nvim(
        socket: &Path,
        sender: Sender<RpcEvent>,
    ) -> NvimConnection {
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(connection) = NvimConnection::connect(
                "test-session".into(),
                "test".into(),
                socket,
                sender.clone(),
            ) {
                return connection;
            }
        }
        panic!("connect to real Neovim");
    }

    /// 驱动编辑、参数输入与最长映射，覆盖真实按键链路。
    fn drive_real_editing(connection: &NvimConnection) -> Result<(), String> {
        connection.eval("setline(1, ['alpha beta gamma'])")?;
        connection.eval("cursor(1, 7)")?;
        connection.input("ciw")?;
        std::thread::sleep(Duration::from_millis(100));
        connection.input("delta")?;
        connection.input("<Esc>")?;
        connection.input("rX")?;
        connection.input("fY")?;
        connection.input("/topsecret\r")?;
        connection.eval("execute('nnoremap ab :echo 1<cr>')")?;
        connection.eval("execute('nnoremap abc :echo 2<cr>')")?;
        assert_eq!(
            connection
                .eval(
                    "luaeval(\"require('lazykeys').has_longer_mapping('ab', 'n')\")",
                )?
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            connection
                .eval(
                    "luaeval(\"require('lazykeys').has_longer_mapping('abc', 'n')\")",
                )?
                .as_bool(),
            Some(false)
        );
        connection.input("a")?;
        std::thread::sleep(Duration::from_millis(50));
        connection.input("b")?;
        std::thread::sleep(Duration::from_millis(100));
        connection.input("c")?;
        std::thread::sleep(Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(
            connection.eval("getline('.')")?.as_str(),
            Some("alpha deltX gamma")
        );
        Ok(())
    }

    /// 上下文观察结果集中记录各语义事件，避免测试主函数维护多个局部标志。
    #[derive(Default)]
    struct ContextObservations {
        context: bool,
        pending: bool,
        redacted_argument: bool,
        search_prompt: bool,
        immediate_completion: bool,
        recent_input: bool,
        longest_completion: bool,
    }

    impl ContextObservations {
        /// 每个预期语义都必须出现，且敏感参数与插入文本不得泄漏。
        fn assert_complete(&self) {
            assert!(self.context, "companion should emit context");
            assert!(self.pending, "companion should emit the pending command");
            assert!(
                self.redacted_argument,
                "command arguments must be redacted"
            );
            assert!(
                self.immediate_completion,
                "ciw should complete on mode change"
            );
            assert!(
                self.search_prompt,
                "search type may be emitted without its argument"
            );
            assert!(
                self.recent_input,
                "recent safe input should be visible immediately"
            );
            assert!(
                self.longest_completion,
                "the complete longest mapping should finish"
            );
        }
    }

    /// 汇总上下文事件并逐条检查隐私边界。
    fn observe_context(
        receiver: &mpsc::Receiver<RpcEvent>,
    ) -> ContextObservations {
        let mut observed = ContextObservations::default();
        while let Ok(event) = receiver.try_recv() {
            if let RpcEvent::Context { session_id, json } = event {
                assert_eq!(session_id, "test-session");
                observed.context = true;
                observed.pending |= json.contains("\"pending\":\"ci\"");
                observed.redacted_argument |= json.contains("<redacted>");
                observed.search_prompt |= json.contains("\"pending\":\"/\"");
                observed.immediate_completion |=
                    json.contains("\"lastInput\":\"ciw\"");
                observed.recent_input |= json.contains("\"lastInput\":\"/\"");
                observed.longest_completion |=
                    json.contains("\"lastInput\":\"abc\"");
                assert!(!json.contains("delta"));
                assert!(!json.contains("topsecret"));
                assert!(!json.contains("rX"));
                assert!(!json.contains("fY"));
            }
        }
        observed
    }

    /// 最近输入只短期保留，超时后必须清空。
    fn assert_recent_input_expires(receiver: &mpsc::Receiver<RpcEvent>) {
        std::thread::sleep(Duration::from_millis(1600));
        let mut latest_recent_input = None;
        while let Ok(event) = receiver.try_recv() {
            if let RpcEvent::Context { json, .. } = event {
                let context: crate::models::EditorContext =
                    serde_json::from_str(&json).unwrap();
                latest_recent_input = Some(context.last_input);
            }
        }
        assert_eq!(latest_recent_input.as_deref(), Some(""));
    }

    #[test]
    fn real_nvim_context_input_and_privacy() {
        let (_temp, socket, _child) = real_nvim_fixture();
        let (sender, receiver) = mpsc::channel();
        let connection = connect_real_nvim(&socket, sender);
        let mappings = connection.export_keymaps().unwrap();
        assert!(!mappings.is_empty(), "companion should export real keymaps");
        drive_real_editing(&connection).unwrap();
        observe_context(&receiver).assert_complete();
        assert_recent_input_expires(&receiver);
    }
}
