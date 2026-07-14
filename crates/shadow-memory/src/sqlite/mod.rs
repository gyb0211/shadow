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
//! - [`store`]: 写路径 (store_row_with_metadata / embedding)
//! - [`recall`]: 检索底层 (fts5_search / vector_search / recall_by_time_only)
//! - [`query`]: 读路径 trait 方法体 (recall / get / list / export / stats ...)
//! - [`mutate`]: 写路径 trait 方法体 (forget / purge / count / supersede ...)
//! - [`agent`]: agent UUID 管理
//! - [`util`]: 纯函数 + 常量

pub mod agent;
pub mod mutate;
pub mod query;
pub mod recall;
pub mod schema;
pub mod store;
pub mod util;

use crate::embedding::{create_embedding_provider, EmbeddingProvider, NoopEmbedding};
use crate::sqlite::schema::{init_schema, open_connection};
use crate::sqlite::util::acquire_sqlite_startup_lock;
use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use rusqlite::Connection;
use shadow_config::schema::SearchMode;
use shadow_core::kennel::attribution;
use shadow_core::kennel::memory::StoreOptions;
use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, Role};
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

/// Trait 方法均为薄委托, 实现体在 [`query`] / [`mutate`] / [`store`] / [`recall`]。
#[async_trait]
impl Memory for SqliteMemory {
    fn name(&self) -> &str {
        "sqlite"
    }

    // ── store 路径 (委托 store.rs) ──────────────────────────────

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.store_row_with_metadata(key, content, category, session_id, StoreOptions::default(), None)
            .await
    }

    async fn store_with_metadata(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        namespace: Option<&str>,
        importance: Option<f64>,
    ) -> anyhow::Result<()> {
        self.store_row_with_metadata(
            key,
            content,
            category,
            session_id,
            StoreOptions {
                namespace: namespace.map(str::to_string),
                importance,
                ..StoreOptions::default()
            },
            None,
        )
        .await
    }

    async fn store_with_options(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        options: StoreOptions,
    ) -> anyhow::Result<()> {
        self.store_row_with_metadata(key, content, category, session_id, options, None)
            .await
    }

    async fn store_with_agent(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        namespace: Option<&str>,
        importance: Option<f64>,
        agent_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.store_row_with_metadata(
            key,
            content,
            category,
            session_id,
            StoreOptions {
                namespace: namespace.map(str::to_string),
                importance,
                ..StoreOptions::default()
            },
            agent_id,
        )
        .await
    }

    // ── query 路径 (委托 query.rs) ──────────────────────────────

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        self.recall_inner(query, limit, session_id, since, until).await
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        self.get_inner(key).await
    }

    async fn get_for_agent(
        &self,
        key: &str,
        agent_id: &str,
    ) -> anyhow::Result<Option<MemoryEntry>> {
        self.get_for_agent_inner(key, agent_id).await
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        self.list_inner(category, session_id).await
    }

    async fn export(
        &self,
        filter: &shadow_core::kennel::memory::ExportFilter,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        self.export_inner(filter).await
    }

    async fn export_agent(&self, agent_alias: &str) -> anyhow::Result<Vec<MemoryEntry>> {
        self.export_agent_inner(agent_alias).await
    }

    async fn count_in_scope(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
    ) -> anyhow::Result<u64> {
        self.count_in_scope_inner(namespace, category).await
    }

    async fn stats(&self) -> anyhow::Result<shadow_core::kennel::memory::MemoryStats> {
        self.stats_inner().await
    }

    async fn health_check(&self) -> bool {
        self.health_check_inner().await
    }

    async fn reindex(&self) -> anyhow::Result<usize> {
        self.reindex_inner().await
    }

    async fn recall_namespaced(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let entries = self
            .recall_inner(query, limit * 2, session_id, since, until)
            .await?;
        let filtered: Vec<MemoryEntry> = entries
            .into_iter()
            .filter(|e| e.namespace == namespace)
            .take(limit)
            .collect();
        Ok(filtered)
    }

    async fn recall_for_agents(
        &self,
        allowed_agent_ids: &[&str],
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        self.recall_for_agents_inner(allowed_agent_ids, query, limit, session_id, since, until)
            .await
    }

    // ── mutate 路径 (委托 mutate.rs) ────────────────────────────

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        self.forget_inner(key).await
    }

    async fn forget_for_agent(&self, key: &str, agent_id: &str) -> anyhow::Result<bool> {
        self.forget_for_agent_inner(key, agent_id).await
    }

    async fn purge_namespace(&self, namespace: &str) -> anyhow::Result<usize> {
        self.purge_namespace_inner(namespace).await
    }

    async fn purge_session(&self, session_id: &str) -> anyhow::Result<usize> {
        self.purge_session_inner(session_id).await
    }

    async fn purge_session_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> anyhow::Result<usize> {
        self.purge_session_for_agent_inner(session_id, agent_id).await
    }

    async fn purge_agent(&self, agent_alias: &str) -> anyhow::Result<usize> {
        self.purge_agent_inner(agent_alias).await
    }

    async fn rename_agent(&self, from: &str, to: &str) -> anyhow::Result<usize> {
        self.rename_agent_inner(from, to).await
    }

    async fn count_agent(&self, agent_alias: &str) -> anyhow::Result<usize> {
        self.count_agent_inner(agent_alias).await
    }

    async fn count(&self) -> anyhow::Result<usize> {
        self.count_inner().await
    }

    async fn supersede(&self, superseded_ids: &[String], new_id: &str) -> anyhow::Result<()> {
        self.supersede_inner(superseded_ids, new_id).await
    }

    async fn ensure_agent_uuid(&self, alias: &str) -> anyhow::Result<String> {
        self.ensure_agent_uuid_inner(alias).await
    }

    async fn refresh_embedder(
        &self,
        model_provider: &str,
        api_key: Option<&str>,
        model: &str,
        dimensions: usize,
    ) {
        let embedder: Arc<dyn EmbeddingProvider> = Arc::from(create_embedding_provider(
            model_provider,
            api_key,
            model,
            dimensions,
        ));
        self.swap_embedder(embedder);
    }
}
