//! 影子记忆后端实现
//!
//! 当前实现: None (空) + Markdown (文件)

pub mod none;
pub mod markdown;
pub mod sqlite;
pub mod strategy;

use agent_core::Memory;
use anyhow::Result;
use std::sync::Arc;

/// 工厂函数 -- 按类型名创建 memory 后端
pub fn create_memory(backend: &str, workspace_dir: &std::path::Path) -> Result<Arc<dyn Memory>> {
    match backend {
        "none" => Ok(Arc::new(none::NoneMemoryBackend)),
        "markdown" => Ok(Arc::new(markdown::MarkdownMemory::new(workspace_dir))),
        "sqlite" => Ok(Arc::new(sqlite::SqliteMemory::new(workspace_dir)?)),
        _ => anyhow::bail!("未知的 memory 后端: {backend}"),
    }
}
