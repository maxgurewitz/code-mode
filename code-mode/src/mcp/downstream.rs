use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::{Peer, RunningService},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde::Serialize;
use serde_json::Value;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom},
    process::ChildStderr,
    sync::Mutex,
};

use super::config::{Config, ServerEntry};

/// A live connection to a downstream MCP server.
struct LiveConnection {
    peer: Peer<RoleClient>,
    /// Keep the RunningService alive so the connection isn't dropped.
    _service: RunningService<RoleClient, ()>,
    session: Arc<LogSessionRecord>,
}

#[derive(Debug, Clone)]
struct LogSessionRecord {
    server: String,
    session_id: String,
    log_path: PathBuf,
    pid: Option<u32>,
    started_at_unix_ms: u64,
}

impl LogSessionRecord {
    fn summary(&self, active: bool) -> LogSessionSummary {
        LogSessionSummary {
            server: self.server.clone(),
            session_id: self.session_id.clone(),
            log_path: self.log_path.display().to_string(),
            pid: self.pid,
            started_at_unix_ms: self.started_at_unix_ms,
            active,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogSessionSummary {
    pub server: String,
    pub session_id: String,
    pub log_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub started_at_unix_ms: u64,
    pub active: bool,
}

#[derive(Debug, Serialize)]
pub struct LogReadResult {
    pub server: String,
    pub session_id: String,
    pub log_path: String,
    pub offset: u64,
    pub next_offset: u64,
    pub eof: bool,
    pub text: String,
}

/// Manages connections to downstream MCP servers for tool routing.
pub struct DownstreamManager {
    configs: HashMap<String, ServerEntry>,
    connections: Mutex<HashMap<String, Arc<LiveConnection>>>,
    sessions: Mutex<HashMap<String, Arc<LogSessionRecord>>>,
    log_root: PathBuf,
    session_counter: AtomicU64,
}

impl DownstreamManager {
    /// Build a manager from the merged config, keeping only stdio servers.
    pub fn from_config(config: &Config) -> Self {
        let configs: HashMap<String, ServerEntry> = config
            .servers
            .iter()
            .filter(|(_, e)| e.transport == "stdio" && e.command.is_some())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Self {
            configs,
            connections: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            log_root: resolve_log_root(config),
            session_counter: AtomicU64::new(0),
        }
    }

    /// Connect to a downstream server (or return a cached connection).
    async fn get_connection(&self, server_name: &str) -> Result<Arc<LiveConnection>> {
        let mut connections = self.connections.lock().await;

        if let Some(conn) = connections.get(server_name) {
            if !conn._service.is_closed() {
                return Ok(Arc::clone(conn));
            }
            // Connection is dead, remove it
            connections.remove(server_name);
        }

        let entry = self
            .configs
            .get(server_name)
            .with_context(|| format!("no downstream server configured: {server_name}"))?;

        let command = entry.command.as_deref().unwrap();
        let (transport, stderr) =
            TokioChildProcess::builder(tokio::process::Command::new(command).configure(|cmd| {
                cmd.args(&entry.args);
                cmd.envs(&entry.env);
            }))
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn downstream server: {server_name}"))?;

        let session = self.create_log_session(server_name, transport.id()).await?;
        self.spawn_stderr_logger(stderr, session.log_path.clone());

        let service = ().serve(transport).await.with_context(|| {
            format!(
                "failed to initialize downstream client: {server_name} (stderr log: {})",
                session.log_path.display()
            )
        })?;

        let peer = service.peer().clone();
        let conn = Arc::new(LiveConnection {
            peer,
            _service: service,
            session,
        });

        connections.insert(server_name.to_string(), Arc::clone(&conn));
        Ok(conn)
    }

    /// Check if a server name is known.
    pub fn has_server(&self, server_name: &str) -> bool {
        self.configs.contains_key(server_name)
    }

    /// Call a tool on a downstream server.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult> {
        if !self.configs.contains_key(server_name) {
            bail!("unknown downstream server: {server_name}");
        }

        let conn = self.get_connection(server_name).await?;

        let params = CallToolRequestParams::new(tool_name.to_string()).with_arguments(arguments);

        let result = conn
            .peer
            .call_tool(params)
            .await
            .with_context(|| format!("failed to call {server_name}.{tool_name}"))?;

        Ok(self.attach_log_metadata(result, server_name, &conn.session))
    }

    pub async fn current_log_sessions(&self, server: Option<&str>) -> Vec<LogSessionSummary> {
        let connections = self.connections.lock().await;
        let mut sessions: Vec<_> = connections
            .iter()
            .filter(|(name, conn)| {
                !conn._service.is_closed()
                    && server.map(|server| server == name.as_str()).unwrap_or(true)
            })
            .map(|(_, conn)| conn.session.summary(true))
            .collect();
        sessions.sort_by(|left, right| {
            left.server
                .cmp(&right.server)
                .then(left.started_at_unix_ms.cmp(&right.started_at_unix_ms))
        });
        sessions
    }

    pub async fn read_log(
        &self,
        session_id: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<LogReadResult> {
        let session = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(session_id)
                .cloned()
                .with_context(|| format!("unknown log session: {session_id}"))?
        };

        let mut file = File::open(&session.log_path)
            .await
            .with_context(|| format!("failed to open log file: {}", session.log_path.display()))?;
        let file_len = file
            .metadata()
            .await
            .with_context(|| format!("failed to read metadata: {}", session.log_path.display()))?
            .len();
        let offset = offset.min(file_len);
        file.seek(SeekFrom::Start(offset))
            .await
            .with_context(|| format!("failed to seek log file: {}", session.log_path.display()))?;

        let mut buffer = vec![0; max_bytes];
        let read = file
            .read(&mut buffer)
            .await
            .with_context(|| format!("failed to read log file: {}", session.log_path.display()))?;
        buffer.truncate(read);

        let next_offset = offset + read as u64;

        Ok(LogReadResult {
            server: session.server.clone(),
            session_id: session.session_id.clone(),
            log_path: session.log_path.display().to_string(),
            offset,
            next_offset,
            eof: next_offset >= file_len,
            text: String::from_utf8_lossy(&buffer).into_owned(),
        })
    }

    async fn create_log_session(
        &self,
        server_name: &str,
        pid: Option<u32>,
    ) -> Result<Arc<LogSessionRecord>> {
        let started_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let sequence = self.session_counter.fetch_add(1, Ordering::Relaxed);
        let safe_server_name = sanitize_path_component(server_name);
        let session_id = format!("{safe_server_name}-{started_at_unix_ms}-{sequence}");
        let server_dir = self.log_root.join(&safe_server_name);
        std::fs::create_dir_all(&server_dir)
            .with_context(|| format!("failed to create log directory: {}", server_dir.display()))?;

        let log_path = server_dir.join(format!("{session_id}.stderr.log"));
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("failed to create log file: {}", log_path.display()))?;

        let session = Arc::new(LogSessionRecord {
            server: server_name.to_string(),
            session_id: session_id.clone(),
            log_path,
            pid,
            started_at_unix_ms,
        });

        self.sessions
            .lock()
            .await
            .insert(session_id, Arc::clone(&session));

        Ok(session)
    }

    fn spawn_stderr_logger(&self, stderr: Option<ChildStderr>, log_path: PathBuf) {
        let Some(mut stderr) = stderr else {
            return;
        };

        tokio::spawn(async move {
            let Ok(mut file) = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .await
            else {
                return;
            };

            let _ = tokio::io::copy(&mut stderr, &mut file).await;
            let _ = file.flush().await;
        });
    }

    fn attach_log_metadata(
        &self,
        mut result: CallToolResult,
        server_name: &str,
        session: &LogSessionRecord,
    ) -> CallToolResult {
        let mut meta = result.meta.take().unwrap_or_default();
        meta.0.insert(
            "codeModeServer".into(),
            Value::String(server_name.to_string()),
        );
        meta.0.insert(
            "codeModeSessionId".into(),
            Value::String(session.session_id.clone()),
        );
        meta.0.insert(
            "codeModeLogPath".into(),
            Value::String(session.log_path.display().to_string()),
        );
        result.meta = Some(meta);
        result
    }
}

fn resolve_log_root(config: &Config) -> PathBuf {
    let base_dir = config
        .base_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(".code-mode"));
    let base_dir = if base_dir.is_absolute() {
        base_dir
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(base_dir)
    };
    base_dir.join("logs")
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect();

    if sanitized.is_empty() {
        "session".into()
    } else {
        sanitized
    }
}
