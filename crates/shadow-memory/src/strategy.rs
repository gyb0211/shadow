//! 记忆策略 -- DefaultMemoryStrategy 实现 + 工具函数
//!
//! Trait 定义在 `shadow_core::MemoryStrategy`.

use shadow_core::{Memory, MemoryCategory, MemoryEntry, MemoryStrategy};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;

/// 从 user message 提取搜索关键词.
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
pub struct DefaultMemoryStrategy {
    memory: Arc<dyn Memory>,
}

impl DefaultMemoryStrategy {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

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

#[async_trait]
impl MemoryStrategy for DefaultMemoryStrategy {
    async fn load_context(
        &self,
        _observer: &dyn shadow_core::Observer,
        query: &str,
        session_id: Option<&str>,
    ) -> Vec<MemoryEntry> {
        let queries = extract_queries(query);
        let mut all_entries = Vec::new();
        let mut seen_keys = HashSet::new();
        let limit = 5;

        for q in &queries {
            let entries = self
                .memory
                .recall(q, limit, session_id, None, None)
                .await
                .unwrap_or_default();
            for entry in entries {
                if seen_keys.insert(entry.key.clone()) {
                    all_entries.push(entry);
                }
            }
        }

        // 按 score 排序
        all_entries.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_entries.truncate(limit);
        all_entries
    }

    async fn consolidate_turn(
        &self,
        user_message: &str,
        assistant_response: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        if !is_important_turn(user_message, assistant_response) {
            return Ok(());
        }
        let key = format!("turn_{}_{}", chrono::Utc::now().timestamp(), session_id.unwrap_or("default"));
        self.memory
            .store(&key, user_message, MemoryCategory::Conversation, session_id)
            .await
    }

    async fn run_governance(&self) -> Result<()> {
        Ok(())
    }
}
