//! SQLite memory -- 工具函数与常量
//!
//! 纯函数 + schema 常量 + 启动锁。无 `&self` 依赖, 可被任意子模块直接调用。

use shadow_core::kennel::memory::{MemoryCategory, MemoryKind};
use std::sync::{Mutex as StdMutex, MutexGuard};

pub(super) const SQLITE_OPEN_TIMEOUT_CAP_SECS: u64 = 300;

pub(super) static SQLITE_MEMORY_STARTUP_LOCK: StdMutex<()> = StdMutex::new(());

pub(super) fn acquire_sqlite_startup_lock() -> MutexGuard<'static, ()> {
    SQLITE_MEMORY_STARTUP_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── 分类 / kind 编解码 ─────────────────────────────────────────

pub(super) fn str_to_category(s: &str) -> MemoryCategory {
    match s {
        "core" => MemoryCategory::Core,
        "daily" => MemoryCategory::Daily,
        "conversation" => MemoryCategory::Conversation,
        other => MemoryCategory::Custom(other.to_string()),
    }
}

pub(super) fn category_to_str(cat: &MemoryCategory) -> String {
    match cat {
        MemoryCategory::Core => "core".into(),
        MemoryCategory::Daily => "daily".into(),
        MemoryCategory::Conversation => "conversation".into(),
        MemoryCategory::Custom(name) => name.clone(),
    }
}

pub(super) fn decode_kind(raw: Option<String>) -> Option<MemoryKind> {
    raw.and_then(|kind| serde_json::from_str(&kind).ok())
}

// ── 内容哈希 ───────────────────────────────────────────────────

pub(super) fn content_hash(query: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(query.as_bytes());
    format!(
        "{:016x}",
        u64::from_be_bytes(hash[..8].try_into().expect(
            "SHA-256 always produces >= 8 bytes"
        ))
    )
}

// ── FTS5 / LIKE 查询辅助 ───────────────────────────────────────

pub(super) fn fts5_term_query(term: &str) -> String {
    if let Some(prefix) = term.strip_suffix('*')
        && !prefix.is_empty()
    {
        let escaped = prefix.replace('"', "\"\"");
        format!("\"{escaped}\"*")
    } else {
        let escaped = term.replace('"', "\"\"");
        format!("\"{escaped}\"")
    }
}

pub(super) fn like_search_pattern(term: &str) -> String {
    if let Some(prefix) = term.strip_suffix('*')
        && !prefix.is_empty()
    {
        return format!("%{}%", escape_like_pattern(prefix));
    }
    format!("%{}%", escape_like_pattern(term))
}

pub(super) fn is_prefix_wildcard_term(term: &str) -> bool {
    matches!(term.strip_suffix('*'), Some(prefix) if !prefix.is_empty())
}

pub(super) fn like_fallback_matches(text: &str, term: &str) -> bool {
    let text = text.to_lowercase();
    if let Some(prefix) = term.strip_suffix('*')
        && !prefix.is_empty()
    {
        let prefix = prefix.to_lowercase();
        return text
            .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
            .any(|token| token.starts_with(&prefix));
    }
    text.contains(&term.to_lowercase())
}

pub(super) fn escape_like_pattern(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len());
    for ch in term.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}
