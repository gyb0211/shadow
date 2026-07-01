//! SessionStore trait -- 会话持久化抽象

use crate::attribution::Attributable;
use crate::provider::ChatMessage;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
