//! SessionStore trait -- 会话持久化抽象

use crate::attribution::{Attributable, Role};
use crate::provider::ChatMessage;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 一个会话 -- 消息历史的有序集合
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub messages: Vec<ChatMessage>,
}

/// 会话存储 trait
///
/// 用于跨进程重启恢复对话。
/// 通过 [`Attributable`] 参与归因 (Role::Session), alias 取后端类型。
#[async_trait]
pub trait SessionStore: Attributable {
    /// 加载会话; 不存在返回 None
    async fn load(&self, id: &str) -> Result<Option<Session>>;

    /// 保存或覆盖会话
    async fn save(&self, session: &Session) -> Result<()>;

    /// 删除会话; 不存在视为成功
    async fn delete(&self, id: &str) -> Result<()>;

    /// 列出所有会话 ID
    async fn list(&self) -> Result<Vec<String>>;
}

/// JSONL 文件会话存储
///
/// 每条消息一行 JSON, 追加写入 (append-only).
/// 存储路径: `{workspace}/sessions/{session_id}.jsonl`
///
/// 当前会话 ID 通过最新修改时间动态计算 (不单独存储 current_session 文件).
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

    /// 返回最近修改的会话 ID (通过文件修改时间动态计算)
    /// 如果没有会话, 返回 None
    #[must_use]
    pub fn current_session_id(&self) -> Option<String> {
        let dir = self.sessions_dir();
        let entries = std::fs::read_dir(&dir).ok()?;
        let mut latest: Option<(std::time::SystemTime, String)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
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
    /// 读取整个文件, 每行反序列化为 ChatMessage
    async fn load(&self, id: &str) -> Result<Option<Session>> {
        let path = self.session_file(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        let mut messages = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let msg: ChatMessage = serde_json::from_str(line)?;
            messages.push(msg);
        }
        Ok(Some(Session {
            id: id.to_string(),
            messages,
        }))
    }

    /// 每条 ChatMessage 序列化为一行 JSON, 追加写入文件
    async fn save(&self, session: &Session) -> Result<()> {
        let dir = self.sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.session_file(&session.id);
        // 追加写入: 每条消息一行 JSON
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        for msg in &session.messages {
            let line = serde_json::to_string(msg)?;
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    /// 删除会话文件; 不存在视为成功
    async fn delete(&self, id: &str) -> Result<()> {
        let path = self.session_file(id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// 扫描 sessions/ 目录, 返回所有 .jsonl 文件名 (去掉扩展名)
    /// 按修改时间降序排序 (最近修改的在前)
    async fn list(&self) -> Result<Vec<String>> {
        let dir = self.sessions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions: Vec<(std::time::SystemTime, String)> = Vec::new();
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
            sessions.push((modified, id));
        }
        // 按修改时间降序排序 (最近修改的在前)
        sessions.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(sessions.into_iter().map(|(_, id)| id).collect())
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
            tool_call_id: None,
            tool_calls: vec![],
            reasoning_content: None,
        }
    }

    /// save → load 验证消息一致
    #[tokio::test]
    async fn save_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        let session = Session {
            id: "test-session-1".to_string(),
            messages: vec![
                make_msg("user", "你好"),
                make_msg("assistant", "你好! 有什么可以帮你的?"),
            ],
        };

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

    /// 多会话: save 两个会话, list 返回两个, delete 一个剩一个
    #[tokio::test]
    async fn multiple_sessions_list_and_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        // 保存两个会话
        let s1 = Session {
            id: "session-a".to_string(),
            messages: vec![make_msg("user", "hello a")],
        };
        let s2 = Session {
            id: "session-b".to_string(),
            messages: vec![make_msg("user", "hello b")],
        };
        store.save(&s1).await.unwrap();
        // 小延迟确保修改时间不同
        std::thread::sleep(std::time::Duration::from_millis(20));
        store.save(&s2).await.unwrap();

        // list 应返回两个会话
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&"session-a".to_string()));
        assert!(list.contains(&"session-b".to_string()));

        // 删除一个
        store.delete("session-a").await.unwrap();
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], "session-b");
    }

    /// 追加保存: 多次 save 累积消息
    #[tokio::test]
    async fn append_save_accumulates_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        // 第一次保存
        let s1 = Session {
            id: "append-test".to_string(),
            messages: vec![make_msg("user", "第一条")],
        };
        store.save(&s1).await.unwrap();

        // 第二次追加保存
        let s2 = Session {
            id: "append-test".to_string(),
            messages: vec![
                make_msg("assistant", "回复1"),
                make_msg("user", "第二条"),
            ],
        };
        store.save(&s2).await.unwrap();

        // 加载应包含全部 3 条消息
        let loaded = store.load("append-test").await.unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[0].content, "第一条");
        assert_eq!(loaded.messages[1].content, "回复1");
        assert_eq!(loaded.messages[2].content, "第二条");
    }

    /// current_session_id 返回最近修改的会话
    #[tokio::test]
    async fn current_session_id_returns_latest() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());

        // 没有会话时返回 None
        assert!(store.current_session_id().is_none());

        // 保存第一个会话
        let s1 = Session {
            id: "first".to_string(),
            messages: vec![make_msg("user", "first")],
        };
        store.save(&s1).await.unwrap();
        assert_eq!(store.current_session_id().as_deref(), Some("first"));

        // 延迟后保存第二个会话
        std::thread::sleep(std::time::Duration::from_millis(20));
        let s2 = Session {
            id: "second".to_string(),
            messages: vec![make_msg("user", "second")],
        };
        store.save(&s2).await.unwrap();
        assert_eq!(store.current_session_id().as_deref(), Some("second"));
    }

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

    /// delete 不存在的会话不报错
    #[tokio::test]
    async fn delete_nonexistent_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonlSessionStore::new(tmp.path());
        assert!(store.delete("nonexistent").await.is_ok());
    }
}
