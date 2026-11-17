use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::models::{EditorContext, RuntimeMapping, SessionSummary};
use crate::rpc::{NvimConnection, RpcEvent};

/// RPC 到会话状态的内部事件：携带上下文、映射、关闭与错误，驱动后续搜索和目标切换。
#[derive(Debug, Clone)]
pub enum SessionEvent {
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

/// 会话链路的单连接状态：组合 RPC 连接、工作目录、焦点、最新上下文与映射版本。
struct SessionRecord {
    connection: Arc<NvimConnection>,
    cwd: String,
    active: bool,
    context: Option<EditorContext>,
    mappings: Vec<RuntimeMapping>,
    mapping_revision: u64,
}

/// 会话与执行链路的协调器：管理多条 Neovim 连接，并维护当前目标及其搜索/执行快照。
#[derive(Clone)]
pub struct SessionManager {
    records: Arc<Mutex<HashMap<String, SessionRecord>>>,
    event_sender: Sender<SessionEvent>,
}

/// RPC 事件在会话边界统一改名并转发，接收端关闭时停止泵送。
fn forward_rpc_events(
    receiver: std::sync::mpsc::Receiver<RpcEvent>,
    event_sender: Sender<SessionEvent>,
) {
    while let Ok(event) = receiver.recv() {
        let mapped = match event {
            RpcEvent::Context { session_id, json } => {
                SessionEvent::Context { session_id, json }
            }
            RpcEvent::Mappings {
                session_id,
                mappings,
            } => SessionEvent::Mappings {
                session_id,
                mappings,
            },
            RpcEvent::Closed(session) => SessionEvent::Closed(session),
            RpcEvent::Error { session_id, error } => {
                SessionEvent::Error { session_id, error }
            }
        };
        if event_sender.send(mapped).is_err() {
            break;
        }
    }
}

impl SessionManager {
    /// 会话链路初始化：创建共享连接表和事件出口。
    pub fn new(event_sender: Sender<SessionEvent>) -> Self {
        Self {
            records: Arc::new(Mutex::new(HashMap::new())),
            event_sender,
        }
    }

    /// 会话建立入口：连接 socket、登记运行时状态、标记活动目标，并转发后续 RPC 事件。
    pub fn connect(
        &self,
        profile_id: &str,
        socket_path: impl AsRef<Path>,
        cwd: Option<&str>,
    ) -> Result<SessionSummary, String> {
        let session_id = Uuid::new_v4().to_string();
        let socket_path = socket_path.as_ref().to_path_buf();
        let (sender, receiver) = std::sync::mpsc::channel();
        let connection = NvimConnection::connect(
            session_id.clone(),
            profile_id.to_string(),
            &socket_path,
            sender,
        )?;
        let summary = SessionSummary {
            session_id: session_id.clone(),
            profile_id: profile_id.to_string(),
            socket_path: socket_path.display().to_string(),
            cwd: cwd.unwrap_or("").to_string(),
            connected: true,
            active: true,
        };
        self.records
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?
            .insert(
                session_id.clone(),
                SessionRecord {
                    connection: Arc::new(connection),
                    cwd: cwd.unwrap_or("").to_string(),
                    active: false,
                    context: None,
                    mappings: Vec::new(),
                    mapping_revision: 0,
                },
            );
        self.mark_active(&session_id);
        let event_sender = self.event_sender.clone();
        std::thread::spawn(move || forward_rpc_events(receiver, event_sender));
        Ok(summary)
    }

    /// 启动到会话建立的等待阶段：在 Neovim socket 就绪前限时重试。
    pub fn connect_with_retry(
        &self,
        profile_id: &str,
        socket_path: impl AsRef<Path>,
        cwd: Option<&str>,
    ) -> Result<SessionSummary, String> {
        let socket_path = socket_path.as_ref();
        let mut last_error = String::new();
        for _ in 0..60 {
            match self.connect(profile_id, socket_path, cwd) {
                Ok(summary) => return Ok(summary),
                Err(error) => {
                    last_error = error;
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
            }
        }
        Err(last_error)
    }

    /// 搜索与执行前的目标解析：优先校验显式会话，否则使用当前活动连接。
    pub fn target_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<String, String> {
        let records =
            self.records.lock().map_err(|_| "session lock poisoned")?;
        match session_id {
            Some(id) if records.contains_key(id) => Ok(id.to_string()),
            Some(_) => Err("指定的 Neovim 会话不存在或已断开".into()),
            None => records
                .values()
                .find(|record| record.active)
                .map(|record| record.connection.session_id.clone())
                .ok_or("没有已连接的 Neovim 会话".into()),
        }
    }

    /// 执行确认的目标说明：从会话状态提取 Profile 和工作目录。
    pub fn target_description(
        &self,
        session_id: &str,
    ) -> Result<String, String> {
        self.records
            .lock()
            .map_err(|_| "session lock poisoned")?
            .get(session_id)
            .map(|record| {
                format!(
                    "{} · {}",
                    record.connection.profile_id,
                    if record.cwd.is_empty() {
                        "未知目录"
                    } else {
                        &record.cwd
                    }
                )
            })
            .ok_or("指定的 Neovim 会话不存在或已断开".into())
    }

    /// 搜索到执行的一致性快照：同时取出映射及其版本，供结果确认时校验未变化。
    pub fn mapping_snapshot(
        &self,
        session_id: &str,
    ) -> Result<(Vec<RuntimeMapping>, u64), String> {
        self.records
            .lock()
            .map_err(|_| "session lock poisoned")?
            .get(session_id)
            .map(|record| (record.mappings.clone(), record.mapping_revision))
            .ok_or("指定的 Neovim 会话不存在或已断开".into())
    }

    /// 搜索链路的映射读取：返回指定会话的当前运行时映射。
    pub fn mappings_for(
        &self,
        session_id: &str,
    ) -> Result<Vec<RuntimeMapping>, String> {
        Ok(self.mapping_snapshot(session_id)?.0)
    }

    #[cfg(test)]
    pub fn execute(
        &self,
        session_id: Option<&str>,
        lhs: &str,
    ) -> Result<(), String> {
        let target = self.target_session(session_id)?;
        let connection = self
            .records
            .lock()
            .map_err(|_| "session lock poisoned")?
            .get(&target)
            .map(|record| record.connection.clone())
            .ok_or("指定的 Neovim 会话不存在或已断开")?;
        connection.input(lhs)
    }

    /// 执行链路的并发保护：仅当映射版本仍匹配查询结果时，才向目标会话写入按键。
    pub fn execute_with_revision(
        &self,
        session_id: &str,
        lhs: &str,
        expected_revision: u64,
    ) -> Result<(), String> {
        let connection = self
            .records
            .lock()
            .map_err(|_| "session lock poisoned")?
            .get(session_id)
            .and_then(|record| {
                if record.mapping_revision == expected_revision {
                    Some(record.connection.clone())
                } else {
                    None
                }
            })
            .ok_or("Neovim 映射已变化，请重新查询并确认后再执行")?;
        connection.input(lhs)
    }

    /// 会话到 RPC 的列表读取：把内部连接记录投影为可选择、可显示的摘要。
    pub fn list(&self) -> Vec<SessionSummary> {
        self.records
            .lock()
            .map(|records| {
                records
                    .values()
                    .map(|record| SessionSummary {
                        session_id: record.connection.session_id.clone(),
                        profile_id: record.connection.profile_id.clone(),
                        socket_path: record
                            .connection
                            .socket_path
                            .display()
                            .to_string(),
                        cwd: record.cwd.clone(),
                        connected: true,
                        active: record.active,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 上下文到搜索/执行的状态更新：保存最新映射并递增版本，使旧查询结果不能贸然执行。
    pub fn record_mappings(
        &self,
        session_id: &str,
        mappings: Vec<RuntimeMapping>,
    ) {
        if let Ok(mut records) = self.records.lock()
            && let Some(record) = records.get_mut(session_id)
        {
            record.mappings = mappings;
            record.mapping_revision = record.mapping_revision.wrapping_add(1);
        }
    }

    /// 搜索链路的快捷读取：取当前活动会话的运行时映射。
    pub fn active_mappings(&self) -> Vec<RuntimeMapping> {
        self.records
            .lock()
            .ok()
            .and_then(|records| {
                records
                    .values()
                    .find(|record| record.active)
                    .map(|record| record.mappings.clone())
            })
            .unwrap_or_default()
    }

    /// 上下文到目标切换的更新：保存最新现场，仅由聚焦会话改变活动目标。
    pub fn record_context(
        &self,
        context: EditorContext,
    ) -> Option<EditorContext> {
        let mut records = self.records.lock().ok()?;
        let record = records.get_mut(&context.session_id)?;
        record.context = Some(context.clone());
        if !context.focused {
            return None;
        }
        for (id, record) in records.iter_mut() {
            record.active = id == &context.session_id;
        }
        Some(context)
    }

    /// 会话切换入口：按稳定顺序选择下一个连接，并返回其最近上下文。
    pub fn cycle_active(&self) -> Option<EditorContext> {
        let mut records = self.records.lock().ok()?;
        let mut ids: Vec<_> = records.keys().cloned().collect();
        ids.sort();
        if ids.is_empty() {
            return None;
        }
        let current = records
            .values()
            .find(|record| record.active)
            .map(|record| record.connection.session_id.clone());
        let next = current
            .and_then(|id| ids.iter().position(|candidate| candidate == &id))
            .map(|index| ids[(index + 1) % ids.len()].clone())
            .unwrap_or_else(|| ids[0].clone());
        for (id, record) in records.iter_mut() {
            record.active = id == &next;
        }
        records.get(&next).and_then(|record| record.context.clone())
    }

    /// 会话状态的内部归一：确保只有指定连接被视为活动目标。
    fn mark_active(&self, session_id: &str) {
        if let Ok(mut records) = self.records.lock() {
            for (id, record) in records.iter_mut() {
                record.active = id == session_id;
            }
        }
    }

    /// 会话关闭收尾：移除连接记录，并报告被关闭的是否活动目标。
    pub fn disconnect(&self, session_id: &str) -> bool {
        self.records
            .lock()
            .ok()
            .and_then(|mut records| records.remove(session_id))
            .map(|record| record.active)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_session(
        manager: &SessionManager,
        id: &str,
    ) -> std::os::unix::net::UnixStream {
        let (connection, peer) = NvimConnection::for_test(id, "test");
        manager.records.lock().unwrap().insert(
            id.into(),
            SessionRecord {
                connection: Arc::new(connection),
                cwd: "/tmp".into(),
                active: false,
                context: None,
                mappings: Vec::new(),
                mapping_revision: 0,
            },
        );
        peer
    }

    fn mapping(lhs: &str) -> RuntimeMapping {
        RuntimeMapping {
            mode: "n".into(),
            lhs: lhs.into(),
            rhs: String::new(),
            desc: lhs.into(),
            scope: "global".into(),
        }
    }

    fn focused_context(id: &str) -> EditorContext {
        EditorContext {
            session_id: id.into(),
            focused: true,
            ..EditorContext::default()
        }
    }

    fn unfocused_context(id: &str) -> EditorContext {
        EditorContext {
            session_id: id.into(),
            focused: false,
            ..EditorContext::default()
        }
    }

    #[test]
    fn mappings_follow_focused_session_and_survive_a_b_a_transitions() {
        let (sender, _) = std::sync::mpsc::channel();
        let manager = SessionManager::new(sender);
        let _peer_a = add_session(&manager, "a");
        let _peer_b = add_session(&manager, "b");
        manager.record_mappings("a", vec![mapping("aa")]);
        manager.record_mappings("b", vec![mapping("bb")]);
        assert!(manager.record_context(focused_context("a")).is_some());
        assert_eq!(manager.active_mappings()[0].lhs, "aa");
        assert!(manager.record_context(focused_context("b")).is_some());
        assert_eq!(manager.active_mappings()[0].lhs, "bb");
        assert!(manager.record_context(unfocused_context("a")).is_none());
        assert_eq!(manager.active_mappings()[0].lhs, "bb");
        assert!(manager.record_context(focused_context("a")).is_some());
        assert_eq!(manager.active_mappings()[0].lhs, "aa");
        assert_eq!(manager.cycle_active().unwrap().session_id, "b");
        assert_eq!(manager.active_mappings()[0].lhs, "bb");
    }

    #[test]
    fn execution_rejects_changed_mapping_revision_without_writing() {
        let (sender, _) = std::sync::mpsc::channel();
        let manager = SessionManager::new(sender);
        let _peer = add_session(&manager, "a");
        let (_, initial_revision) = manager.mapping_snapshot("a").unwrap();
        manager.record_mappings("a", vec![mapping("new")]);
        let error = manager
            .execute_with_revision("a", "w", initial_revision)
            .unwrap_err();
        assert!(error.contains("映射已变化"));
    }

    #[test]
    fn execution_accepts_unchanged_mapping_revision() {
        let (sender, _) = std::sync::mpsc::channel();
        let manager = SessionManager::new(sender);
        let mut peer = add_session(&manager, "a");
        manager.record_mappings("a", vec![mapping("w")]);
        let (_, revision) = manager.mapping_snapshot("a").unwrap();
        let responder = std::thread::spawn(move || {
            let mut bytes = [0_u8; 256];
            let read = std::io::Read::read(&mut peer, &mut bytes).unwrap();
            let mut cursor = std::io::Cursor::new(&bytes[..read]);
            let request = rmpv::decode::read_value(&mut cursor).unwrap();
            let id = request.as_array().unwrap()[1].as_u64().unwrap();
            let response = rmpv::Value::Array(vec![
                rmpv::Value::from(1),
                rmpv::Value::from(id),
                rmpv::Value::Nil,
                rmpv::Value::Nil,
            ]);
            let mut output = Vec::new();
            rmpv::encode::write_value(&mut output, &response).unwrap();
            std::io::Write::write_all(&mut peer, &output).unwrap();
        });
        manager.execute_with_revision("a", "w", revision).unwrap();
        responder.join().unwrap();
    }

    #[test]
    fn invalid_explicit_session_is_not_replaced_by_active_session() {
        let (sender, _) = std::sync::mpsc::channel();
        let manager = SessionManager::new(sender);
        let error = manager.execute(Some("missing"), "diw").unwrap_err();
        assert!(error.contains("指定"));
    }
}
