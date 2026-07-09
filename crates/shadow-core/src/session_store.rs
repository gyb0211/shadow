//! SessionStore trait -- 会话持久化抽象
//!
//! 提供 session 的 load / append_message / save / list / list_with_metadata.
//!
//! - `append_message`: 单条追加 (运行时对话流, 推荐)
//! - `save`: 全量覆盖 (初始化 / 修复 / 恢复历史)
//!
//! 元信息 (title / created_at / updated_at / agent_alias) 通过 sidecar
//! 文件 `{id}.meta.json` 存储. 旧 session (无 sidecar) load 时元信息字段
//! 全为 None, list_with_metadata 用文件 mtime + jsonl 行数推导.

use crate::kennel::attribution::{Attributable, Role};
use crate::kennel::provider::ChatMessage;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 一个会话 -- 消息历史 + 元信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub messages: Vec<ChatMessage>,
    /// 人类可读的会话标题 (UI 展示用)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 创建时间 (RFC 3339)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// 最后更新时间 (RFC 3339)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// 关联的 agent 别名 (多 agent / 多 profile 时区分)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_alias: Option<String>,
}

/// 会话元信息 -- 用于 list_with_metadata (不加载 messages)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub message_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_alias: Option<String>,
}

/// 会话存储 trait
///
/// 用于跨进程重启恢复对话.
/// 通过 [`Attributable`] 参与归因 (Role::Session), alias 取后端类型.
#[async_trait]
pub trait SessionStore: Attributable {
    /// 加载会话; 不存在返回 None
    async fn load(&self, id: &str) -> Result<Option<Session>>;

    /// 追加单条消息 -- 运行时对话流推荐用此 API.
    ///
    /// 第一次调用自动创建 session (生成元信息 sidecar), 后续调用更新
    /// `updated_at` 与 `message_count`.
    async fn append_message(&self, id: &str, message: &ChatMessage) -> Result<()>;

    /// 全量覆盖保存 -- 初始化 / 修复 / 恢复历史用.
    ///
    /// 注意: 此方法 truncate 现有 messages 文件并重写元信息 sidecar,
    /// 行为与 [`Self::append_message`] 不同.
    async fn save(&self, session: &Session) -> Result<()>;

    /// 删除会话; 不存在视为成功
    async fn delete(&self, id: &str) -> Result<()>;

    /// 列出所有会话 ID (按修改时间降序)
    async fn list(&self) -> Result<Vec<String>>;

    /// 列出所有会话含元信息 (不加载 messages, UI 友好).
    ///
    /// 旧 session (无 `.meta.json` sidecar) 用文件 mtime + jsonl 行数推导元信息.
    async fn list_with_metadata(&self) -> Result<Vec<SessionMetadata>>;
}

/// JSONL + sidecar metadata 文件会话存储
///
/// 文件布局:
/// - `{workspace}/sessions/{id}.jsonl`      -- 消息 (一行一条 JSON)
/// - `{workspace}/sessions/{id}.meta.json`  -- 元信息 (单 JSON, 可选)
///
/// 向后兼容: 旧 session 无 `.meta.json`, [`Self::load`] 时元信息字段全 None,
/// [`Self::list_with_metadata`] 用 mtime + 行数推导.
pub struct JsonlSessionStore {
    /// 工作区根目录 (如 ~/.shadow/)
    workspace: PathBuf,
}

impl JsonlSessionStore {
    /// 创建 JsonlSessionStore, workspace 为根目录
    #[must_use]
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// sessions 目录路径: `{workspace}/sessions/`
    fn sessions_dir(&self) -> PathBuf {
        self.workspace.join("sessions")
    }

    /// 单个会话文件路径: `{workspace}/sessions/{id}.jsonl`
    fn session_file(&self, id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{id}.jsonl"))
    }

    /// 元信息 sidecar 路径: `{workspace}/sessions/{id}.meta.json`
    fn meta_file(&self, id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{id}.meta.json"))
    }

    /// 返回最近修改的会话 ID (通过文件修改时间动态计算)
    #[must_use]
    pub fn current_session_id(&self) -> Option<String> {
        let dir = self.sessions_dir();
        let entries = std::fs::read_dir(&dir).ok()?;
        let mut latest: Option<(std::time::SystemTime, String)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
            // 跳过 .meta.json sidecar, 只看 .jsonl
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let modified = entry.metadata().ok()?.modified().ok();
            let id = path.file_stem()?.to_string_lossy().to_string();
            match (&latest, modified) {
                (Some((prev_time, _)), Some(m)) if &m <= prev_time => {}
                (Some(_), None) => {}
                _ => {
                    latest = Some((
                        modified.unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                        id,
                    ));
                }
            }
        }
        latest.map(|(_, id)| id)
    }

    /// 读 meta sidecar; 不存在或解析失败返回 None
    fn read_meta(&self, id: &str) -> Option<SessionMetadata> {
        let path = self.meta_file(id);
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// 写 meta sidecar (覆盖)
    fn write_meta(&self, id: &str, meta: &SessionMetadata) -> Result<()> {
        let dir = self.sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.meta_file(id);
        let content = serde_json::to_string_pretty(meta)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// 数 jsonl 文件消息条数 (非空行)
    fn count_messages(&self, id: &str) -> usize {
        let path = self.session_file(id);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return 0;
        };
        content.lines().filter(|l| !l.trim().is_empty()).count()
    }

    /// 取 jsonl 文件 mtime 转 RFC 3339 (旧 session 推导用)
    fn file_mtime_rfc3339(&self, id: &str) -> Option<String> {
        let path = self.session_file(id);
        let mtime = std::fs::metadata(&path).ok()?.modified().ok()?;
        let dt: chrono::DateTime<chrono::Utc> = mtime.into();
        Some(dt.to_rfc3339())
    }
}

impl Attributable for JsonlSessionStore {
    fn role(&self) -> Role {
        Role::Session
    }
    fn alias(&self) -> &str {
        "jsonl"
    }
}

#[async_trait]
impl SessionStore for JsonlSessionStore {
    async fn load(&self, id: &str) -> Result<Option<Session>> {
        let path = self.session_file(id);
        if !path.exists() {
            return Ok(None);
        }
        // 读消息
        let content = std::fs::read_to_string(&path)?;
        let mut messages = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let msg: ChatMessage = serde_json::from_str(line)?;
            messages.push(msg);
        }
        // 读 meta sidecar (向后兼容: 不存在全 None)
        let meta = self.read_meta(id);
        Ok(Some(Session {
            id: id.to_string(),
            messages,
            title: meta.as_ref().and_then(|m| m.title.clone()),
            created_at: meta.as_ref().and_then(|m| m.created_at.clone()),
            updated_at: meta.as_ref().and_then(|m| m.updated_at.clone()),
            agent_alias: meta.as_ref().and_then(|m| m.agent_alias.clone()),
        }))
    }

    async fn append_message(&self, id: &str, message: &ChatMessage) -> Result<()> {
        let dir = self.sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.session_file(id);
        // 追加消息
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let line = serde_json::to_string(message)?;
        writeln!(file, "{line}")?;
        drop(file);
        // 更新 meta sidecar (第一次自动创建)
        let now = chrono::Utc::now().to_rfc3339();
        let existing_meta = self.read_meta(id);
        let updated_meta = match existing_meta {
            Some(mut m) => {
                m.updated_at = Some(now);
                m.message_count = self.count_messages(id);
                m
            }
            None => SessionMetadata {
                id: id.to_string(),
                title: None,
                created_at: Some(now.clone()),
                updated_at: Some(now),
                message_count: 1,
                agent_alias: None,
            },
        };
        self.write_meta(id, &updated_meta)?;
        Ok(())
    }

    async fn save(&self, session: &Session) -> Result<()> {
        let dir = self.sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.session_file(&session.id);
        // truncate + write messages
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;
        for msg in &session.messages {
            let line = serde_json::to_string(msg)?;
            writeln!(file, "{line}")?;
        }
        drop(file);
        // 写 meta sidecar (created_at 缺失时用当前时间兜底)
        let now = chrono::Utc::now().to_rfc3339();
        let meta = SessionMetadata {
            id: session.id.clone(),
            title: session.title.clone(),
            created_at: session.created_at.clone().or_else(|| Some(now.clone())),
            updated_at: Some(now),
            message_count: session.messages.len(),
            agent_alias: session.agent_alias.clone(),
        };
        self.write_meta(&session.id, &meta)?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let jsonl = self.session_file(id);
        if jsonl.exists() {
            std::fs::remove_file(&jsonl)?;
        }
        let meta = self.meta_file(id);
        if meta.exists() {
            std::fs::remove_file(&meta)?;
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<String>> {
        let dir = self.sessions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions: Vec<(std::time::SystemTime, String)> = Vec::new();
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            // 只看 .jsonl, 跳过 .meta.json
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            sessions.push((modified, id));
        }
        // 按修改时间降序 (最近修改的在前)
        sessions.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(sessions.into_iter().map(|(_, id)| id).collect())
    }

    async fn list_with_metadata(&self) -> Result<Vec<SessionMetadata>> {
        let dir = self.sessions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out: Vec<(std::time::SystemTime, SessionMetadata)> = Vec::new();
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            // 有 sidecar 用 sidecar, 无则推导
            let meta = self.read_meta(&id).unwrap_or_else(|| SessionMetadata {
                id: id.clone(),
                title: None,
                created_at: self.file_mtime_rfc3339(&id),
                updated_at: self.file_mtime_rfc3339(&id),
                message_count: self.count_messages(&id),
                agent_alias: None,
            });
            out.push((modified, meta));
        }
        // 按修改时间降序
        out.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(out.into_iter().map(|(_, m)| m).collect())
    }
}

