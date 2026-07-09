//! 影子记忆后端实现
//!
//! 当前实现: None (空) + Markdown (文件) + SQLite (FTS5 + 语义检索)
//!
//! 语义检索模块:
//! - [`embedding`]: EmbeddingProvider trait + Noop/OpenAI 实现
//! - [`vector`]: 余弦相似度 + 混合检索融合

pub mod none;
// pub mod markdown;
// pub mod sqlite;
pub mod strategy;
pub mod embedding;
pub mod vector;

pub use strategy::{format_entries, DefaultMemoryStrategy};

use embedding::EmbeddingProvider;
use shadow_core::Memory;
use anyhow::Result;
use std::sync::Arc;

