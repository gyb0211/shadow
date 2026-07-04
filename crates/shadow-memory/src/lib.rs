//! 影子记忆后端实现
//!
//! 当前实现: None (空) + Markdown (文件) + SQLite (FTS5 + 语义检索)
//!
//! 语义检索模块:
//! - [`embedding`]: EmbeddingProvider trait + Noop/OpenAI 实现
//! - [`vector`]: 余弦相似度 + 混合检索融合

pub mod none;
pub mod markdown;
pub mod sqlite;
pub mod strategy;
pub mod embedding;
pub mod vector;

pub use strategy::{format_entries, DefaultMemoryStrategy};

use embedding::EmbeddingProvider;
use shadow_core::Memory;
use anyhow::Result;
use std::sync::Arc;

/// 工厂函数 -- 按类型名创建 memory 后端
///
/// 默认不带 embedding provider (退化为纯 FTS5)。
/// 如需语义检索, 使用 [`create_memory_with_embedding`]。
pub fn create_memory(backend: &str, workspace_dir: &std::path::Path) -> Result<Arc<dyn Memory>> {
    match backend {
        "none" => Ok(Arc::new(none::NoneMemoryBackend)),
        "markdown" => Ok(Arc::new(markdown::MarkdownMemory::new(workspace_dir))),
        "sqlite" => Ok(Arc::new(sqlite::SqliteMemory::new(workspace_dir)?)),
        _ => anyhow::bail!("未知的 memory 后端: {backend}"),
    }
}

/// 工厂函数 (带 embedding) -- 创建支持语义检索的 memory 后端
///
/// 目前仅 sqlite 后端支持 embedding, 其他后端忽略 embedding provider。
pub fn create_memory_with_embedding(
    backend: &str,
    workspace_dir: &std::path::Path,
    embedding: Arc<dyn EmbeddingProvider>,
) -> Result<Arc<dyn Memory>> {
    match backend {
        "none" => Ok(Arc::new(none::NoneMemoryBackend)),
        "markdown" => Ok(Arc::new(markdown::MarkdownMemory::new(workspace_dir))),
        "sqlite" => Ok(Arc::new(sqlite::SqliteMemory::with_embedding(
            workspace_dir,
            Some(embedding),
        )?)),
        _ => anyhow::bail!("未知的 memory 后端: {backend}"),
    }
}
