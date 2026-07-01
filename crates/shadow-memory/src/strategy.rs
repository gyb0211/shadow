//! 记忆策略 -- 控制记忆的加载/存储/治理
//!
//! DefaultMemoryStrategy 提供基础实现:
//! - load_context: recall 检索相关记忆, 格式化为上下文文本
//! - consolidate_turn: 把对话轮次存为一条 MemoryEntry

use shadow_core::{Memory, MemoryEntry};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;

/// 记忆策略 -- 控制记忆的加载/存储/治理
#[async_trait]
pub trait MemoryStrategy: Send + Sync {
    /// 加载与用户消息相关的记忆上下文
    async fn load_context(&self, user_message: &str) -> String;

    /// 从对话中提取并存储记忆
    async fn consolidate_turn(&self, user_msg: &str, assistant_msg: &str) -> Result<()>;
}

/// 默认记忆策略
///
/// 使用 Memory trait 的 recall 检索记忆,
/// consolidate_turn 把整轮对话存为一条 MemoryEntry。
pub struct DefaultMemoryStrategy {
    /// 记忆后端
    memory: Arc<dyn Memory>,
}

impl DefaultMemoryStrategy {
    /// 创建默认记忆策略, 绑定一个 Memory 后端
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl MemoryStrategy for DefaultMemoryStrategy {
    /// 加载记忆上下文: recall(query, limit=5), 格式化为 [memory_context]...[/memory_context]
    async fn load_context(&self, user_message: &str) -> String {
        // 检索相关记忆 (最多 5 条)
        let entries = match self.memory.recall(user_message, 5).await {
            Ok(e) => e,
            Err(_) => return String::new(),
        };

        // 无记忆时返回空字符串
        if entries.is_empty() {
            return String::new();
        }

        // 格式化为带标签的上下文文本
        let body = entries
            .iter()
            .map(|e| format!("- {}", e.content))
            .collect::<Vec<_>>()
            .join("\n");

        format!("[memory_context]\n{body}\n[/memory_context]")
    }

    /// 存储对话轮次: 把 user+assistant 合并为一条 MemoryEntry, key 用时间戳
    async fn consolidate_turn(&self, user_msg: &str, assistant_msg: &str) -> Result<()> {
        let now = Utc::now();
        // key 用毫秒级时间戳, 保证唯一性
        let key = format!("turn_{}", now.timestamp_millis());
        // 内容合并用户消息和助手回复
        let content = format!("[用户] {user_msg}\n[助手] {assistant_msg}");

        let entry = MemoryEntry {
            id: key.clone(),
            key,
            content,
            category: "conversation".to_string(),
            timestamp: now,
            session_id: None,
            agent_alias: None,
        };

        self.memory.store(&entry).await
    }
}

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqliteMemory;

    /// 测试: load_context 检索记忆并格式化为 [memory_context] 标签
    #[tokio::test]
    async fn test_load_context() {
        let dir = tempfile::tempdir().unwrap();
        let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        // 先存储一条记忆
        let entry = MemoryEntry {
            id: "1".to_string(),
            key: "rust".to_string(),
            content: "Rust 是一门系统编程语言".to_string(),
            category: "fact".to_string(),
            timestamp: Utc::now(),
            session_id: None,
            agent_alias: None,
        };
        mem.store(&entry).await.unwrap();

        let strategy = DefaultMemoryStrategy::new(mem);
        let ctx = strategy.load_context("Rust").await;

        assert!(ctx.contains("[memory_context]"), "应包含 memory_context 开始标签");
        assert!(
            ctx.contains("Rust 是一门系统编程语言"),
            "应包含记忆内容"
        );
        assert!(
            ctx.contains("[/memory_context]"),
            "应包含 memory_context 结束标签"
        );
    }

    /// 测试: consolidate_turn 存储对话轮次
    #[tokio::test]
    async fn test_consolidate_turn() {
        let dir = tempfile::tempdir().unwrap();
        let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        let strategy = DefaultMemoryStrategy::new(mem.clone());

        // 存储一轮对话
        strategy
            .consolidate_turn("什么是 Rust?", "Rust 是一门系统编程语言")
            .await
            .unwrap();

        // 验证: 通过 list 应能看到存储的记忆
        let list = mem.list().await.unwrap();
        assert_eq!(list.len(), 1, "应存储了 1 条记忆");
        assert!(
            list[0].content.contains("什么是 Rust?"),
            "内容应包含用户消息"
        );
        assert!(
            list[0].content.contains("系统编程语言"),
            "内容应包含助手消息"
        );
        assert_eq!(list[0].category, "conversation", "分类应为 conversation");
        assert!(list[0].key.starts_with("turn_"), "key 应以 turn_ 开头");
    }
}
