//! SQLite 记忆后端 -- 使用 SQLite + FTS5 全文检索
//!
//! 表结构:
//! - agents: agent 别名 → UUID 映射
//! - memories: 主表, 存储记忆条目 (id TEXT PK), UNIQUE(agent_id, key)
//! - memories_fts: FTS5 虚拟表, BM25 全文搜索 (外部内容表, 触发器同步)
//! - embedding_cache: 向量缓存 (LRU)
//!
//! FTS 索引通过触发器自动同步, 业务代码只需操作主表。
//!
//! 子模块:
//! - [`schema`]: 建表 / 迁移 / open_connection
//! - [`store`]: 写路径 (store_row_with_metadata / embedding 缓存)
//! - [`recall`]: 检索底层 (fts5_search / vector_search / recall_by_time_only)
//! - [`memory_impl`]: `impl Memory for SqliteMemory` 完整 trait 实现
//! - [`agent`]: agent UUID 管理
//! - [`util`]: 纯函数 + 常量

pub mod agent;
pub mod memory_impl;
pub mod recall;
pub mod schema;
pub mod store;
pub mod util;

use crate::embedding::{EmbeddingProvider, NoopEmbedding};
use crate::sqlite::schema::{init_schema, open_connection};
use crate::sqlite::util::acquire_sqlite_startup_lock;
use parking_lot::{Mutex, RwLock};
use rusqlite::Connection;
use shadow_config::schema::SearchMode;
use shadow_core::kennel::attribution;
use shadow_core::{Attributable, Role};
use std::path::Path;
use std::sync::Arc;

pub use agent::{sqlite_ensure_agent_uuid, sqlite_ensure_default_agent_uuid};

/// SQLite 记忆后端
///
/// 使用 SQLite 存储记忆条目, FTS5 提供全文搜索能力。
/// 采用 WAL 模式提升并发读写性能。
/// 可选注入 EmbeddingProvider 实现语义搜索 (FTS5 + 向量混合检索)。
pub struct SqliteMemory {
    alias: String,
    pub(super) conn: Arc<Mutex<Connection>>,
    pub(super) embedder: RwLock<Arc<dyn EmbeddingProvider>>,
    pub(super) vector_weight: f32,
    pub(super) keyword_weight: f32,
    pub(super) cache_max: usize,
    pub(super) search_mode: SearchMode,
}

impl SqliteMemory {
    pub fn with_embedder(
        alias: &str,
        workspace_dir: &Path,
        embedder: Arc<dyn EmbeddingProvider>,
        vector_weight: f32,
        keyword_weight: f32,
        cache_max: usize,
        open_timeout_secs: Option<u64>,
        search_mode: SearchMode,
    ) -> anyhow::Result<Self> {
        let db_path = workspace_dir.join("memory").join("brain.db");

        let _startup_guard = acquire_sqlite_startup_lock();

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = open_connection(&db_path, open_timeout_secs)?;

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA mmap_size    = 8388608;
             PRAGMA cache_size   = -2000;
             PRAGMA temp_store   = MEMORY;",
        )?;

        init_schema(&conn)?;

        Ok(Self {
            alias: alias.to_string(),
            conn: Arc::new(Mutex::new(conn)),
            embedder: RwLock::new(embedder),
            vector_weight,
            keyword_weight,
            cache_max,
            search_mode,
        })
    }

    pub fn new(alias: &str, workspace_dir: &Path) -> anyhow::Result<Self> {
        Self::with_embedder(
            alias,
            workspace_dir,
            Arc::new(NoopEmbedding),
            0.7,
            0.3,
            10_000,
            None,
            SearchMode::default(),
        )
    }

    pub fn new_named(db_name: &str, alias: &str, workspace_dir: &Path) -> anyhow::Result<Self> {
        let db_path = workspace_dir.join("memory").join(format!("{db_name}.db"));
        let _startup_guard = acquire_sqlite_startup_lock();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = open_connection(&db_path, None)?;
        let _ = conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA mmap_size    = 8388608;
             PRAGMA cache_size   = -2000;
             PRAGMA temp_store   = MEMORY;",
        );

        init_schema(&conn)?;

        Ok(Self {
            alias: alias.to_string(),
            conn: Arc::new(Mutex::new(conn)),
            embedder: RwLock::new(Arc::new(NoopEmbedding)),
            vector_weight: 0.7,
            keyword_weight: 0.3,
            cache_max: 10_000,
            search_mode: Default::default(),
        })
    }
}

impl Attributable for SqliteMemory {
    fn role(&self) -> Role {
        Role::Memory(attribution::MemoryKind::Sqlite)
    }
    fn alias(&self) -> &str {
        "sqlite"
    }
}
