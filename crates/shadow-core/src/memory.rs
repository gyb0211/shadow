//! 记忆 trait -- 对话存储与检索
//!
//! 参考 ZeroClaw 设计, 重构为分参数 store + 枚举 category + session 过滤。

use crate::attribution::{Attributable, Role};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// 记忆分类枚举
///
/// - [`MemoryCategory::Core`]: 长期事实/偏好
/// - [`MemoryCategory::Daily`]: 日常会话
/// - [`MemoryCategory::Conversation`]: 对话上下文
/// - [`MemoryCategory::Custom`]: 自定义分类
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryCategory {
    /// 长期事实/偏好
    Core,
    /// 日常会话
    Daily,
    /// 对话上下文
    Conversation,
    /// 自定义分类 (传入任意字符串)
    Custom(String),
}

impl MemoryCategory {
    /// 返回分类的字符串标识
    ///
    /// Core -> "core", Daily -> "daily", Conversation -> "conversation",
    /// Custom(s) -> s
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Core => "core",
            Self::Daily => "daily",
            Self::Conversation => "conversation",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// 从字符串解析为 MemoryCategory
    ///
    /// "core" -> Core, "daily" -> Daily, "conversation" -> Conversation,
    /// 其他 -> Custom(s)
    #[must_use]
    pub fn from_str(s: &str) -> Self {
        match s {
            "core" => Self::Core,
            "daily" => Self::Daily,
            "conversation" => Self::Conversation,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// 自定义序列化: 序列化为字符串
// Core -> "core", Daily -> "daily", Conversation -> "conversation", Custom(s) -> s
impl Serialize for MemoryCategory {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

// 自定义反序列化: 从字符串解析
impl<'de> Deserialize<'de> for MemoryCategory {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_str(&s))
    }
}

/// 记忆条目
///
/// 一条记忆的完整数据。timestamp 使用 RFC 3339 字符串 (序列化友好),
/// score 为检索相关度 (仅 recall 结果有意义)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// 唯一标识 (uuid)
    pub id: String,
    /// 业务键 (用于 get/forget)
    pub key: String,
    /// 记忆内容
    pub content: String,
    /// 分类
    pub category: MemoryCategory,
    /// 时间戳 (RFC 3339 字符串)
    pub timestamp: String,
    /// 会话 ID (可选, 用于会话级记忆隔离)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 检索相关度 (仅 recall 结果有意义)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Agent 别名 (归因)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_alias: Option<String>,
}

/// 记忆后端 trait
///
/// 每个存储后端实现此 trait (SQLite/Markdown/None...)。
/// 参考 ZeroClaw 设计: store 使用分参数, 调用方无需构造完整 MemoryEntry。
#[async_trait]
pub trait Memory: Attributable {
    /// 后端名称 (如 "sqlite" / "markdown" / "none")
    fn name(&self) -> &str;

    /// 存储记忆 -- 分参数, 后端负责生成 id/timestamp
    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()>;

    /// 检索记忆 -- 支持关键词 + session 过滤
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>>;

    /// 获取单条记忆 (按 key)
    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>>;

    /// 列出记忆 (可选按 category 过滤)
    async fn list(&self, category: Option<&MemoryCategory>) -> Result<Vec<MemoryEntry>>;

    /// 删除记忆, 返回是否删除成功
    async fn forget(&self, key: &str) -> Result<bool>;

    /// 记忆总数
    async fn count(&self) -> Result<usize>;

    /// 健康检查
    fn health_check(&self) -> bool;
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
    fn name(&self) -> &str {
        "none"
    }

    async fn store(
        &self,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }

    async fn get(&self, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(&self, _category: Option<&MemoryCategory>) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }

    async fn forget(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    fn health_check(&self) -> bool {
        true
    }
}

// ── 记忆策略 trait ──

/// 记忆策略 -- 控制记忆的加载/存储
///
/// 由 Agent runtime 在对话前后调用:
/// - [`MemoryStrategy::before_chat`]: 检索相关记忆, 返回原始 entries (调用方负责格式化注入)
/// - [`MemoryStrategy::after_chat`]: 提取并存储本轮重要事实
///
/// 实现方放在 `shadow-memory` 等 crate (如 `DefaultMemoryStrategy`).
#[async_trait]
pub trait MemoryStrategy: Send + Sync {
    /// 对话前: 检索相关记忆, 返回原始 entries.
    ///
    /// 调用方负责格式化 (如拼接成 `[memory_context]...[/memory_context]`) 并注入 system prompt.
    async fn before_chat(
        &self,
        user_message: &str,
        session_id: Option<&str>,
    ) -> Vec<MemoryEntry>;

    /// 对话后: 从对话中提取并存储重要事实.
    ///
    /// 实现方可自行决定过滤策略 (如跳过寒暄/确认), 失败应返回 Err 但不应中断主流程.
    async fn after_chat(
        &self,
        user_message: &str,
        assistant_response: &str,
        session_id: Option<&str>,
    ) -> Result<()>;
}

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;

    /// 测试: MemoryCategory 序列化为字符串
    #[test]
    fn test_category_serialize() {
        let json = serde_json::to_string(&MemoryCategory::Core).unwrap();
        assert_eq!(json, "\"core\"");
        let json = serde_json::to_string(&MemoryCategory::Daily).unwrap();
        assert_eq!(json, "\"daily\"");
        let json = serde_json::to_string(&MemoryCategory::Conversation).unwrap();
        assert_eq!(json, "\"conversation\"");
        let json = serde_json::to_string(&MemoryCategory::Custom("skill".to_string())).unwrap();
        assert_eq!(json, "\"skill\"");
    }

    /// 测试: MemoryCategory 反序列化
    #[test]
    fn test_category_deserialize() {
        let cat: MemoryCategory = serde_json::from_str("\"core\"").unwrap();
        assert_eq!(cat, MemoryCategory::Core);
        let cat: MemoryCategory = serde_json::from_str("\"daily\"").unwrap();
        assert_eq!(cat, MemoryCategory::Daily);
        let cat: MemoryCategory = serde_json::from_str("\"conversation\"").unwrap();
        assert_eq!(cat, MemoryCategory::Conversation);
        let cat: MemoryCategory = serde_json::from_str("\"custom-thing\"").unwrap();
        assert_eq!(cat, MemoryCategory::Custom("custom-thing".to_string()));
    }

    /// 测试: as_str / Display / from_str 往返
    #[test]
    fn test_category_roundtrip() {
        for cat in [
            MemoryCategory::Core,
            MemoryCategory::Daily,
            MemoryCategory::Conversation,
            MemoryCategory::Custom("xyz".to_string()),
        ] {
            let s = cat.as_str();
            assert_eq!(cat.to_string(), s);
            assert_eq!(MemoryCategory::from_str(s), cat);
        }
    }

    /// 测试: NoneMemory 所有方法返回空/默认值
    #[tokio::test]
    async fn test_none_memory() {
        let mem = NoneMemory;
        assert_eq!(mem.name(), "none");
        assert!(mem.health_check());

        mem.store("k", "v", MemoryCategory::Core, None)
            .await
            .unwrap();
        assert_eq!(mem.count().await.unwrap(), 0);
        assert!(mem.get("k").await.unwrap().is_none());
        assert!(mem
            .recall("q", 10, None)
            .await
            .unwrap()
            .is_empty());
        assert!(mem
            .list(None)
            .await
            .unwrap()
            .is_empty());
        assert!(!mem.forget("k").await.unwrap());
    }
}
