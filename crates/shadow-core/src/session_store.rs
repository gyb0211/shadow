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

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造测试用 ChatMessage
    fn make_msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    /// 构造测试用 Session (元信息字段全 None)
    fn make_session(id: &str, messages: Vec<ChatMessage>) -> Session {
        Session {
            id: id.to_string(),
            messages,
            title: None,
            created_at: None,
            updated_at: None,
            agent_alias: None,
        }
    }

    // ── load / save 基础 ──

    /// save → load 验证消息一致
    #[tokio::test]
    async fn save_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        let session = make_session(
            "test-session-1",
            vec![
                make_msg("user", "你好"),
                make_msg("assistant", "你好! 有什么可以帮你的?"),
            ],
        );

        store.save(&session).await.unwrap();

        let loaded = store.load("test-session-1").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, "test-session-1");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, "user");
        assert_eq!(loaded.messages[0].content, "你好");
        assert_eq!(loaded.messages[1].role, "assistant");
        assert_eq!(loaded.messages[1].content, "你好! 有什么可以帮你的?");
    }

    /// 空会话 (文件不存在) load 返回 None
    #[tokio::test]
    async fn load_nonexistent_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());
        let result = store.load("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    /// save 写元信息 sidecar, load 时元信息回填
    #[tokio::test]
    async fn save_then_load_returns_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        let mut session = make_session("s1", vec![make_msg("user", "hi")]);
        session.title = Some("测试标题".to_string());
        session.agent_alias = Some("agent-A".to_string());

        store.save(&session).await.unwrap();
        let loaded = store.load("s1").await.unwrap().unwrap();

        assert_eq!(loaded.title.as_deref(), Some("测试标题"));
        assert_eq!(loaded.agent_alias.as_deref(), Some("agent-A"));
        assert!(loaded.created_at.is_some());
        assert!(loaded.updated_at.is_some());
    }

    /// save 是 truncate+write, 同 id 多次 save 后只保留最后一次内容
    #[tokio::test]
    async fn save_truncates_and_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        let s1 = make_session("dup", vec![make_msg("user", "第一条")]);
        store.save(&s1).await.unwrap();

        let s2 = make_session("dup", vec![make_msg("user", "第二条")]);
        store.save(&s2).await.unwrap();

        let loaded = store.load("dup").await.unwrap().unwrap();
        // 只看到第二次的内容 (truncate+write)
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content, "第二条");
    }

    // ── append_message ──

    /// 第一次 append 自动创建 session + meta sidecar
    #[tokio::test]
    async fn append_message_creates_session_with_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        store
            .append_message("s-new", &make_msg("user", "hello"))
            .await
            .unwrap();

        let loaded = store.load("s-new").await.unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content, "hello");
        // meta 自动生成
        assert!(loaded.created_at.is_some());
        assert!(loaded.updated_at.is_some());
    }

    /// 多次 append 累积消息 + 更新 message_count
    #[tokio::test]
    async fn append_message_accumulates() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        store
            .append_message("s-acc", &make_msg("user", "第一条"))
            .await
            .unwrap();
        store
            .append_message("s-acc", &make_msg("assistant", "回复"))
            .await
            .unwrap();
        store
            .append_message("s-acc", &make_msg("user", "第二条"))
            .await
            .unwrap();

        let loaded = store.load("s-acc").await.unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[2].content, "第二条");
    }

    // ── list / list_with_metadata ──

    /// 多会话: save 两个会话, list 返回两个, delete 一个剩一个
    #[tokio::test]
    async fn multiple_sessions_list_and_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        let s1 = make_session("session-a", vec![make_msg("user", "hello a")]);
        let s2 = make_session("session-b", vec![make_msg("user", "hello b")]);
        store.save(&s1).await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        store.save(&s2).await.unwrap();

        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&"session-a".to_string()));
        assert!(list.contains(&"session-b".to_string()));

        store.delete("session-a").await.unwrap();
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], "session-b");
    }

    /// list_with_metadata 返回所有会话的元信息
    #[tokio::test]
    async fn list_with_metadata_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        let mut s1 = make_session("a", vec![make_msg("user", "x")]);
        s1.title = Some("标题-A".to_string());
        let s2 = make_session(
            "b",
            vec![make_msg("user", "y"), make_msg("assistant", "z")],
        );
        store.save(&s1).await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        store.save(&s2).await.unwrap();

        let metas = store.list_with_metadata().await.unwrap();
        assert_eq!(metas.len(), 2);

        let by_id: std::collections::HashMap<&str, &SessionMetadata> =
            metas.iter().map(|m| (m.id.as_str(), m)).collect();
        assert_eq!(by_id["a"].title.as_deref(), Some("标题-A"));
        assert_eq!(by_id["a"].message_count, 1);
        assert_eq!(by_id["b"].message_count, 2);
        assert!(by_id["b"].title.is_none());
    }

    /// 旧 session (无 .meta.json sidecar) 仍可被 list_with_metadata 推导出元信息
    #[tokio::test]
    async fn list_with_metadata_legacy_session() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        // 直接写 .jsonl, 不写 sidecar (模拟旧 session)
        let dir = store.sessions_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("legacy.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&make_msg("user", "a")).unwrap(),
                serde_json::to_string(&make_msg("assistant", "b")).unwrap()
            ),
        )
        .unwrap();

        let metas = store.list_with_metadata().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, "legacy");
        assert_eq!(metas[0].message_count, 2); // 从 jsonl 行数推导
        assert!(metas[0].created_at.is_some()); // 从 mtime 推导
        assert!(metas[0].title.is_none());
        assert!(metas[0].agent_alias.is_none());
    }

    /// 旧 session load 时元信息字段全 None (无 sidecar)
    #[tokio::test]
    async fn legacy_session_load_returns_none_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        let dir = store.sessions_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("old.jsonl"),
            format!("{}\n", serde_json::to_string(&make_msg("user", "x")).unwrap()),
        )
        .unwrap();

        let loaded = store.load("old").await.unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 1);
        assert!(loaded.title.is_none());
        assert!(loaded.created_at.is_none());
        assert!(loaded.updated_at.is_none());
        assert!(loaded.agent_alias.is_none());
    }

    // ── current_session_id ──

    /// current_session_id 返回最近修改的会话
    #[tokio::test]
    async fn current_session_id_returns_latest() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        assert!(store.current_session_id().is_none());

        let s1 = make_session("first", vec![make_msg("user", "first")]);
        store.save(&s1).await.unwrap();
        assert_eq!(store.current_session_id().as_deref(), Some("first"));

        std::thread::sleep(std::time::Duration::from_millis(20));
        let s2 = make_session("second", vec![make_msg("user", "second")]);
        store.save(&s2).await.unwrap();
        assert_eq!(store.current_session_id().as_deref(), Some("second"));
    }

    /// current_session_id 忽略 .meta.json sidecar
    #[tokio::test]
    async fn current_session_id_ignores_meta_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        let dir = store.sessions_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // 写一个 jsonl 和它的 sidecar
        std::fs::write(dir.join("only.jsonl"), "{}\n").ok();
        std::fs::write(dir.join("only.meta.json"), "{}\n").ok();

        // 应该返回 "only" 而不是 "only.meta"
        assert_eq!(store.current_session_id().as_deref(), Some("only"));
    }

    // ── 杂项 ──

    /// Attributable 实现验证
    #[test]
    fn attributable_implementation() {
        let store = JsonlSessionStore::new("/tmp/test");
        assert_eq!(store.role(), Role::Session);
        assert_eq!(store.alias(), "jsonl");
    }

    /// list 空目录返回空列表
    #[tokio::test]
    async fn list_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());
        let list = store.list().await.unwrap();
        assert!(list.is_empty());
    }

    /// delete 不存在的会话不报错 (并清掉 sidecar)
    #[tokio::test]
    async fn delete_nonexistent_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());
        assert!(store.delete("nonexistent").await.is_ok());
    }

    /// delete 同时清理 .jsonl 和 .meta.json
    #[tokio::test]
    async fn delete_removes_both_files() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        store
            .append_message("to-del", &make_msg("user", "x"))
            .await
            .unwrap();
        assert!(store.session_file("to-del").exists());
        assert!(store.meta_file("to-del").exists());

        store.delete("to-del").await.unwrap();
        assert!(!store.session_file("to-del").exists());
        assert!(!store.meta_file("to-del").exists());
    }
}
