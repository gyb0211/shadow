//! 记忆策略 -- DefaultMemoryStrategy 实现 + 工具函数
//!
//! Trait 定义在 `shadow_core::MemoryStrategy` (与 Memory trait 同层).
//!
//! DefaultMemoryStrategy 提供:
//! - extract_queries: 从 user message 提取搜索关键词 (空白分词 + 长度过滤 + 去重)
//! - before_chat: 多关键词 recall + 去重 + score 排序, 返回原始 entries
//! - after_chat: importance filter 过滤寒暄, 只存有意义的轮次
//!
//! 格式化为 system prompt 文本的工作交给调用方 (format_entries),
//! 这样上层可以二次过滤 / 重排 / 自定义渲染.

use shadow_core::{Memory, MemoryCategory, MemoryEntry, MemoryStrategy};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;

/// 从 user message 提取搜索关键词.
///
/// 当前实现: 简单按空白分词 + 长度 >= 2 字符过滤 + 大小写不敏感去重.
/// 中文等无空白分隔的文本会作为整句传入 -- 未来可替换为 LLM-based 提取器.
pub fn extract_queries(message: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for word in message.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if trimmed.chars().count() < 2 {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if seen.insert(lower) {
            out.push(trimmed.to_string());
        }
    }
    out
}

/// 格式化记忆条目为 system prompt 注入文本.
///
/// 空切片返回空字符串 (调用方据此判断是否注入).
pub fn format_entries(entries: &[MemoryEntry]) -> String {
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

/// 默认记忆策略
///
/// - before_chat: 多关键词 recall + 去重 + score 排序, 截断到 limit=5
/// - after_chat: [`is_important_turn`] 过滤后, 存一条 Conversation 记忆
pub struct DefaultMemoryStrategy {
    memory: Arc<dyn Memory>,
}

impl DefaultMemoryStrategy {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}


/// 简单的重要性过滤器 -- 决定本轮对话是否值得存储.
///
/// 当前规则 (跳过寒暄/确认):
/// - assistant 回复 < 10 字符 -> 跳过 (过滤 "ok"/"好的"/"嗯" 等)
/// - user message < 3 字符 -> 跳过
///
/// 未来可替换为 LLM-based 判定或被 recall 命中过的标记.
fn is_important_turn(user_message: &str, assistant_response: &str) -> bool {
    let u = user_message.trim();
    let a = assistant_response.trim();
    if a.chars().count() < 10 {
        return false;
    }
    if u.chars().count() < 3 {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqliteMemory;

    #[test]
    fn extract_queries_dedups_case_insensitive() {
        let qs = extract_queries("Rust 是什么 async rust");
        // "rust" 大小写不敏感去重, 只出现一次
        let rust_count = qs
            .iter()
            .filter(|s| s.to_lowercase() == "rust")
            .count();
        assert_eq!(rust_count, 1);
    }

    #[test]
    fn extract_queries_skips_short_tokens() {
        let qs = extract_queries("a b cd ef");
        // 长度 < 2 字符的词被过滤
        assert!(qs.iter().all(|s| s.chars().count() >= 2));
    }

    #[test]
    fn format_entries_empty_returns_empty_string() {
        assert_eq!(format_entries(&[]), "");
    }

    #[test]
    fn format_entries_wraps_in_tags() {
        let entries = vec![MemoryEntry {
            id: "1".into(),
            key: "k".into(),
            content: "hello".into(),
            category: MemoryCategory::Core,
            timestamp: "t".into(),
            session_id: None,
            score: None,
            agent_alias: None,
        }];
        let s = format_entries(&entries);
        assert!(s.contains("[memory_context]"));
        assert!(s.contains("hello"));
    }

    #[test]
    fn is_important_turn_rejects_short_response() {
        assert!(!is_important_turn("hello", "ok"));
        assert!(!is_important_turn("hi", "短的回复"));
    }

    #[test]
    fn is_important_turn_accepts_meaningful() {
        assert!(is_important_turn(
            "什么是 Rust?",
            "Rust 是一门系统编程语言"
        ));
    }

    #[tokio::test]
    async fn before_chat_returns_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        mem.store("rust", "Rust 是一门系统编程语言", MemoryCategory::Core, None)
            .await
            .unwrap();

        let strategy = DefaultMemoryStrategy::new(mem);
        let entries = strategy.before_chat("Rust", None).await;

        assert!(entries.iter().any(|e| e.content.contains("Rust")));
    }

    #[tokio::test]
    async fn before_chat_falls_back_to_full_message_when_no_keywords() {
        // 纯中文无空白 -> extract_queries 返回空 -> before_chat 用原消息兜底
        let dir = tempfile::tempdir().unwrap();
        let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        mem.store("k", "记忆内容在这里", MemoryCategory::Core, None)
            .await
            .unwrap();

        let strategy = DefaultMemoryStrategy::new(mem);
        let entries = strategy.before_chat("记忆", None).await;
        assert!(!entries.is_empty());
    }

    #[tokio::test]
    async fn after_chat_skips_short_response() {
        let dir = tempfile::tempdir().unwrap();
        let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        let strategy = DefaultMemoryStrategy::new(mem.clone());

        // assistant 回复 < 20 字符 -> 不存储
        strategy.after_chat("hello", "ok", None).await.unwrap();

        let list = mem.list(None).await.unwrap();
        assert_eq!(list.len(), 0);
    }

    #[tokio::test]
    async fn after_chat_stores_meaningful_turn() {
        let dir = tempfile::tempdir().unwrap();
        let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        let strategy = DefaultMemoryStrategy::new(mem.clone());

        strategy
            .after_chat("什么是 Rust?", "Rust 是一门系统编程语言", None)
            .await
            .unwrap();

        let list = mem.list(None).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].content.contains("什么是 Rust?"));
        assert_eq!(list[0].category, MemoryCategory::Conversation);
        assert!(list[0].key.starts_with("turn_"));
    }
}
