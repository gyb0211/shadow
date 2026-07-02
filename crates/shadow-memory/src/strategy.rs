//! 记忆策略 -- 控制记忆的加载/存储/治理
//!
//! DefaultMemoryStrategy 提供基础实现:
//! - before_chat: recall 检索相关记忆, 格式化为上下文文本
//! - after_chat: 把对话轮次存为一条记忆

use shadow_core::{Memory, MemoryCategory};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// 记忆策略 trait -- 控制记忆的加载/存储
#[async_trait]
pub trait MemoryStrategy: Send + Sync {
    /// 对话前: 检索相关记忆, 返回注入 system prompt 的文本
    async fn before_chat(&self, user_message: &str, session_id: Option<&str>) -> String;

    /// 对话后: 从对话中提取并存储记忆
    async fn after_chat(
        &self,
        user_message: &str,
        assistant_response: &str,
        session_id: Option<&str>,
    ) -> Result<()>;
}

/// 默认记忆策略
///
/// - before_chat: recall(query, limit=5), 格式化为 [memory_context]...[/memory_context]
/// - after_chat: 把 user+assistant 合并存为一条 Conversation 记忆
pub struct DefaultMemoryStrategy {
    memory: Arc<dyn Memory>,
}

impl DefaultMemoryStrategy {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl MemoryStrategy for DefaultMemoryStrategy {
    async fn before_chat(&self, user_message: &str, session_id: Option<&str>) -> String {
        let entries = match self.memory.recall(user_message, 5, session_id).await {
            Ok(e) => e,
            Err(_) => return String::new(),
        };

        if entries.is_empty() {
            return String::new();
        }

        let body = entries
            .iter()
            .map(|e| format!("- {}", e.content))
            .collect::<Vec<_>>()
            .join("\n");

        format!("[memory_context]\n{body}\n[/memory_context]")
    }

    async fn after_chat(
        &self,
        user_message: &str,
        assistant_response: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        let key = format!("turn_{}", chrono::Utc::now().timestamp_millis());
        let content = format!("[用户] {user_message}\n[助手] {assistant_response}");

        self.memory
            .store(&key, &content, MemoryCategory::Conversation, session_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqliteMemory;

    #[tokio::test]
    async fn before_chat_returns_context() {
        let dir = tempfile::tempdir().unwrap();
        let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        mem.store("rust", "Rust 是一门系统编程语言", MemoryCategory::Core, None)
            .await.unwrap();

        let strategy = DefaultMemoryStrategy::new(mem);
        let ctx = strategy.before_chat("Rust", None).await;

        assert!(ctx.contains("[memory_context]"));
        assert!(ctx.contains("Rust 是一门系统编程语言"));
    }

    #[tokio::test]
    async fn after_chat_stores_memory() {
        let dir = tempfile::tempdir().unwrap();
        let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        let strategy = DefaultMemoryStrategy::new(mem.clone());

        strategy
            .after_chat("什么是 Rust?", "Rust 是一门系统编程语言", None)
            .await.unwrap();

        let list = mem.list(None).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].content.contains("什么是 Rust?"));
        assert_eq!(list[0].category, MemoryCategory::Conversation);
        assert!(list[0].key.starts_with("turn_"));
    }
}
