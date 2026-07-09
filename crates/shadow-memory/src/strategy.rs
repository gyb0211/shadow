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

