//! 记忆 trait -- 对话存储与检索

use crate::attribution::{Attributable, Role};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub content: String,
    pub category: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_alias: Option<String>,
}

/// 记忆后端 trait
///
/// 每个存储后端实现此 trait (SQLite/Markdown/None...)
#[async_trait]
pub trait Memory: Attributable {
    /// 存储记忆
    async fn store(&self, entry: &MemoryEntry) -> Result<()>;

    /// 检索相关记忆
    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// 获取单条记忆
    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>>;

    /// 列出全部记忆
    async fn list(&self) -> Result<Vec<MemoryEntry>>;

    /// 删除记忆
    async fn forget(&self, key: &str) -> Result<()>;
}

/// 空记忆 -- 未配置时的占位
pub struct NoneMemory;

impl Attributable for NoneMemory {
    fn role(&self) -> Role {
        Role::Memory
    }
    fn alias(&self) -> &str {
        "none"
    }
}

#[async_trait]
impl Memory for NoneMemory {
    async fn store(&self, _entry: &MemoryEntry) -> Result<()> {
        Ok(())
    }
    async fn recall(&self, _query: &str, _limit: usize) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn get(&self, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }
    async fn list(&self) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn forget(&self, _key: &str) -> Result<()> {
        Ok(())
    }
}
