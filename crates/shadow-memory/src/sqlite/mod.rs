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
//! - [`recall`]: 读路径 (fts5_search / vector_search / recall_by_time_only)
//! - [`agent`]: agent UUID 管理
//! - [`util`]: 纯函数 + 常量

pub mod agent;
pub mod recall;
pub mod schema;
pub mod store;
pub mod util;

use crate::embedding::{create_embedding_provider, EmbeddingProvider, NoopEmbedding};
use crate::sqlite::schema::{init_schema, open_connection};
use crate::sqlite::util::{
    acquire_sqlite_startup_lock, category_to_str, decode_kind, is_prefix_wildcard_term,
    like_fallback_matches, like_search_pattern, str_to_category,
};
use crate::vector;
use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use rusqlite::{params, Connection};
use shadow_config::schema::SearchMode;
use shadow_core::kennel::attribution;
use shadow_core::kennel::memory::{
    is_recent_recall_query, ExportFilter, MemoryStats, StoreOptions,
};
use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, Role};
use std::collections::HashSet;
use std::fmt::Write as _;
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

#[async_trait]
impl Memory for SqliteMemory {
    fn name(&self) -> &str {
        "sqlite"
    }

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

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        if is_recent_recall_query(query) {
            return self
                .recall_by_time_only(limit, session_id, since, until)
                .await;
        }

        let query_embedding = if self.search_mode == SearchMode::Bm25 {
            None
        } else {
            self.get_or_compute_embedding(query).await?
        };
        let conn = self.conn.clone();
        let query = query.to_string();
        let sid = session_id.map(String::from);

        let since_owned = since.map(String::from);
        let until_owned = until.map(String::from);

        let vector_weight = self.vector_weight;
        let keyword_weight = self.keyword_weight;
        let search_mode = self.search_mode.clone();

        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<MemoryEntry>> {
            let conn = conn.lock();
            let session_ref = sid.as_deref();
            let since_ref = since_owned.as_deref();
            let until_ref = until_owned.as_deref();

            let kw_result = if search_mode == SearchMode::Bm25 {
                Vec::new()
            } else {
                Self::fts5_search(&conn, &query, limit * 2).unwrap_or_default()
            };

            let vector_result = if search_mode == SearchMode::Bm25 {
                Vec::new()
            } else if let Some(ref qe) = query_embedding {
                Self::vector_search(&conn, qe, limit * 2, None, session_ref).unwrap_or_default()
            } else {
                Vec::new()
            };

            let merged = if vector_result.is_empty() {
                kw_result
                    .iter()
                    .map(|(id, score)| vector::ScoredResult {
                        id: id.clone(),
                        vector_score: None,
                        keyword_score: Some(*score),
                        final_score: *score,
                    })
                    .collect::<Vec<_>>()
            } else if kw_result.is_empty() {
                vector_result
                    .iter()
                    .map(|(id, score)| vector::ScoredResult {
                        id: id.clone(),
                        vector_score: None,
                        keyword_score: Some(*score),
                        final_score: *score,
                    })
                    .collect::<Vec<_>>()
            } else {
                vector::hybrid_merge(&vector_result, &kw_result, vector_weight, keyword_weight, limit)
            };

            let mut results = Vec::new();
            if merged.is_empty() {
                const MAX_LIKE_KEYWORDS: usize = 8;
                let raw_keywords: Vec<String> = query
                    .split_whitespace()
                    .take(MAX_LIKE_KEYWORDS)
                    .map(str::to_string)
                    .collect();
                if !raw_keywords.is_empty() {
                    let needs_prefix_filter =
                        raw_keywords.iter().any(|keyword| is_prefix_wildcard_term(keyword));
                    let sql_limit = if needs_prefix_filter {
                        limit.saturating_mul(8).min(limit.saturating_add(512))
                    } else {
                        limit
                    };

                    let patterns: Vec<String> =
                        raw_keywords.iter().map(|kw| like_search_pattern(kw)).collect();
                    let conditions: Vec<String> = patterns
                        .iter()
                        .enumerate()
                        .map(|(i, _)| {
                            format!(
                                "(m.content like ?{} ESCAPE '\\' OR m.key like ?{} ESCAPE '\\')",
                                i * 2 + 1,
                                i * 2 + 2,
                            )
                        })
                        .collect();

                    let where_clause = conditions.join(" OR ");
                    let mut param_idx = patterns.len() * 2 + 1;
                    let mut time_conditions = String::new();
                    if since_ref.is_some() {
                        let _ = write!(time_conditions, " AND m.created_at >= ?{param_idx}");
                        param_idx += 1;
                    }
                    if until_ref.is_some() {
                        let _ = write!(time_conditions, " AND m.created_at <= ?{param_idx}");
                        param_idx += 1;
                    }
                    let sql = format!(
                        "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id
                         FROM memories m LEFT JOIN agents a ON a.id = m.agent_id
                         WHERE m.superseded_by IS NULL AND ({where_clause}){time_conditions}
                         ORDER BY m.updated_at DESC
                         LIMIT ?{param_idx}"
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                    for kw in &patterns {
                        param_values.push(Box::new(kw.clone()));
                        param_values.push(Box::new(kw.clone()));
                    }
                    if let Some(s) = since_ref {
                        param_values.push(Box::new(s.to_string()));
                    }
                    if let Some(u) = until_ref {
                        param_values.push(Box::new(u.to_string()));
                    }
                    #[allow(clippy::cast_possible_wrap)]
                    param_values.push(Box::new(sql_limit as i64));
                    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                        param_values.iter().map(AsRef::as_ref).collect();
                    let rows = stmt.query_map(params_ref.as_slice(), |row| {
                        Ok(MemoryEntry {
                            id: row.get(0)?,
                            key: row.get(1)?,
                            content: row.get(2)?,
                            category: str_to_category(&row.get::<_, String>(3)?),
                            timestamp: row.get(4)?,
                            session_id: row.get(5)?,
                            score: Some(1.0),
                            namespace: row.get::<_, Option<String>>(6)?
                                .unwrap_or_else(|| "default".into()),
                            importance: row.get(7)?,
                            superseded_by: row.get(8)?,
                            kind: decode_kind(row.get(9)?),
                            pinned: row.get::<_, i64>(10)? != 0,
                            tenant_id: row.get(13)?,
                            agent_alias: row.get(11)?,
                            agent_id: row.get(12)?,
                        })
                    })?;
                    for row in rows {
                        let entry = row?;
                        if let Some(sid) = session_ref
                            && entry.session_id.as_deref() != Some(sid)
                        {
                            continue;
                        }
                        if needs_prefix_filter
                            && !raw_keywords.iter().any(|keyword| {
                                like_fallback_matches(&entry.key, keyword)
                                    || like_fallback_matches(&entry.content, keyword)
                            })
                        {
                            continue;
                        }
                        results.push(entry);
                        if results.len() >= limit {
                            break;
                        }
                    }
                }
            } else {
                // merged is non-empty: fetch full MemoryEntry rows for the
                // ranked IDs, preserving the hybrid-merge order.
                let ordered_ids: Vec<String> = merged.iter().map(|s| s.id.clone()).collect();
                let id_placeholders: Vec<String> =
                    (0..ordered_ids.len()).map(|i| format!("?{}", i + 1)).collect();

                let mut sql = format!(
                    "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, \
                     m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, \
                     a.alias, m.agent_id, m.tenant_id \
                     FROM memories m LEFT JOIN agents a ON a.id = m.agent_id \
                     WHERE m.superseded_by IS NULL AND m.id IN ({})",
                    id_placeholders.join(", ")
                );

                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ordered_ids
                    .iter()
                    .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                let mut idx = ordered_ids.len() + 1;

                if let Some(sid) = session_ref {
                    let _ = write!(sql, " AND m.session_id = ?{idx}");
                    param_values.push(Box::new(sid.to_string()));
                    idx += 1;
                }
                if let Some(s) = since_ref {
                    let _ = write!(sql, " AND m.created_at >= ?{idx}");
                    param_values.push(Box::new(s.to_string()));
                    idx += 1;
                }
                if let Some(u) = until_ref {
                    let _ = write!(sql, " AND m.created_at <= ?{idx}");
                    param_values.push(Box::new(u.to_string()));
                }

                let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(AsRef::as_ref).collect();
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params_ref.as_slice(), |row| {
                    Ok(MemoryEntry {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        content: row.get(2)?,
                        category: str_to_category(&row.get::<_, String>(3)?),
                        timestamp: row.get(4)?,
                        session_id: row.get(5)?,
                        score: None,
                        namespace: row.get::<_, Option<String>>(6)?
                            .unwrap_or_else(|| "default".into()),
                        importance: row.get(7)?,
                        superseded_by: row.get(8)?,
                        kind: decode_kind(row.get(9)?),
                        pinned: row.get::<_, i64>(10)? != 0,
                        tenant_id: row.get(13)?,
                        agent_alias: row.get(11)?,
                        agent_id: row.get(12)?,
                    })
                })?;

                let mut by_id: std::collections::HashMap<String, MemoryEntry> =
                    std::collections::HashMap::new();
                for row in rows {
                    let entry = row?;
                    by_id.insert(entry.id.clone(), entry);
                }

                for scored in &merged {
                    if let Some(mut entry) = by_id.get(&scored.id).cloned() {
                        entry.score = Some(scored.final_score as f64);
                        results.push(entry);
                        if results.len() >= limit {
                            break;
                        }
                    }
                }
            }
            results.truncate(limit);
            Ok(results)
        })
        .await?
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        let conn = self.conn.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<Option<MemoryEntry>> {
            let conn = conn.lock();
            let mut stmt = conn.prepare(
                "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id \
                 FROM memories m LEFT JOIN agents a ON a.id = m.agent_id \
                 WHERE m.key = ?1",
            )?;

            let mut rows = stmt.query_map(params![key], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    category: str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: decode_kind(row.get(9)?),
                    pinned: row.get::<_, i64>(10)? != 0,
                    tenant_id: row.get(13)?,
                    agent_alias: row.get(11)?,
                    agent_id: row.get(12)?,
                })
            })?;

            match rows.next() {
                Some(Ok(entry)) => Ok(Some(entry)),
                _ => Ok(None),
            }
        })
        .await?
    }

    async fn get_for_agent(
        &self,
        key: &str,
        agent_id: &str,
    ) -> anyhow::Result<Option<MemoryEntry>> {
        let conn = self.conn.clone();
        let key = key.to_string();
        let agent_id = agent_id.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<Option<MemoryEntry>> {
            let conn = conn.lock();
            let mut stmt = conn.prepare(
                "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id \
                 FROM memories m LEFT JOIN agents a ON a.id = m.agent_id \
                 WHERE m.key = ?1 AND m.agent_id = ?2",
            )?;

            let mut rows = stmt.query_map(params![key, agent_id], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    category: str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: decode_kind(row.get(9)?),
                    pinned: row.get::<_, i64>(10)? != 0,
                    tenant_id: row.get(13)?,
                    agent_alias: row.get(11)?,
                    agent_id: row.get(12)?,
                })
            })?;

            match rows.next() {
                Some(Ok(entry)) => Ok(Some(entry)),
                _ => Ok(None),
            }
        })
        .await?
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        const DEFAULT_LIST_LIMIT: i64 = 1000;

        let conn = self.conn.clone();
        let category = category.cloned();
        let sid = session_id.map(String::from);

        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<MemoryEntry>> {
            let conn = conn.lock();
            let session_ref = sid.as_deref();
            let mut results = Vec::new();

            let row_mapper = |row: &rusqlite::Row| -> rusqlite::Result<MemoryEntry> {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    category: str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: decode_kind(row.get(9)?),
                    pinned: row.get::<_, i64>(10)? != 0,
                    tenant_id: row.get(13)?,
                    agent_alias: row.get(11)?,
                    agent_id: row.get(12)?,
                })
            };

            if let Some(ref cat) = category {
                let cat_str = category_to_str(cat);
                let mut stmt = conn.prepare(
                    "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id
                     FROM memories m LEFT JOIN agents a ON a.id = m.agent_id
                     WHERE m.superseded_by IS NULL AND m.category = ?1 ORDER BY m.updated_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![cat_str, DEFAULT_LIST_LIMIT], row_mapper)?;
                for row in rows {
                    let entry = row?;
                    if let Some(sid) = session_ref
                        && entry.session_id.as_deref() != Some(sid)
                    {
                        continue;
                    }
                    results.push(entry);
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id
                     FROM memories m LEFT JOIN agents a ON a.id = m.agent_id
                     WHERE m.superseded_by IS NULL ORDER BY m.updated_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![DEFAULT_LIST_LIMIT], row_mapper)?;
                for row in rows {
                    let entry = row?;
                    if let Some(sid) = session_ref
                        && entry.session_id.as_deref() != Some(sid)
                    {
                        continue;
                    }
                    results.push(entry);
                }
            }

            Ok(results)
        })
        .await?
    }

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = conn.lock();
            let affected = conn.execute("DELETE FROM memories WHERE key = ?1", params![key])?;
            Ok(affected > 0)
        })
        .await?
    }

    async fn forget_for_agent(&self, key: &str, agent_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let key = key.to_string();
        let agent_id = agent_id.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = conn.lock();
            let affected = conn.execute(
                "DELETE FROM memories WHERE key = ?1 AND agent_id = ?2",
                params![key, agent_id],
            )?;
            Ok(affected > 0)
        })
        .await?
    }

    async fn purge_namespace(&self, namespace: &str) -> anyhow::Result<usize> {
        let conn = self.conn.clone();
        let namespace = namespace.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
            let conn = conn.lock();
            let affected = conn.execute(
                "DELETE FROM memories WHERE namespace = ?1",
                params![namespace],
            )?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(affected)
        })
        .await?
    }

    async fn purge_session(&self, session_id: &str) -> anyhow::Result<usize> {
        let conn = self.conn.clone();
        let session_id = session_id.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
            let conn = conn.lock();
            let affected = conn.execute(
                "DELETE FROM memories WHERE session_id = ?1",
                params![session_id],
            )?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(affected)
        })
        .await?
    }

    async fn purge_session_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> anyhow::Result<usize> {
        let conn = self.conn.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
            let conn = conn.lock();
            let affected = conn.execute(
                "DELETE FROM memories WHERE session_id = ?1 AND agent_id = ?2",
                params![session_id, agent_id],
            )?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(affected)
        })
        .await?
    }

    async fn purge_agent(&self, agent_alias: &str) -> anyhow::Result<usize> {
        let conn = self.conn.clone();
        let agent_alias = agent_alias.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
            let conn = conn.lock();
            let affected = conn.execute(
                "DELETE FROM memories WHERE agent_id = (SELECT id FROM agents WHERE alias = ?1 LIMIT 1)",
                params![agent_alias],
            )?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(affected)
        })
        .await?
    }

    async fn rename_agent(&self, from: &str, to: &str) -> anyhow::Result<usize> {
        let conn = self.conn.clone();
        let from = from.to_string();
        let to = to.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
            let conn = conn.lock();
            let to_rows: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE agent_id = (SELECT id FROM agents WHERE alias = ?1 LIMIT 1)",
                params![to],
                |row| row.get(0),
            )?;
            if to_rows > 0 {
                anyhow::bail!(
                    "cannot rename agent memory to `{to}`: an existing memory store under that alias has {to_rows} row(s); refusing to merge"
                );
            }
            conn.execute("DELETE FROM agents WHERE alias = ?1", params![to])?;
            let affected = conn.execute(
                "UPDATE agents SET alias = ?2 WHERE alias = ?1",
                params![from, to],
            )?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(affected)
        })
        .await?
    }

    async fn count_agent(&self, agent_alias: &str) -> anyhow::Result<usize> {
        let conn = self.conn.clone();
        let agent_alias = agent_alias.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
            let conn = conn.lock();
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM agents WHERE alias = ?1",
                params![agent_alias],
                |row| row.get(0),
            )?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(count as usize)
        })
        .await?
    }

    async fn count(&self) -> anyhow::Result<usize> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
            let conn = conn.lock();
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(count as usize)
        })
        .await?
    }

    async fn health_check(&self) -> bool {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || conn.lock().execute_batch("SELECT 1").is_ok())
            .await
            .unwrap_or(false)
    }

    async fn reindex(&self) -> anyhow::Result<usize> {
        // Step 1: Rebuild FTS5 (always safe, cheap)
        {
            let conn = self.conn.clone();
            tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let conn = conn.lock();
                conn.execute_batch("INSERT INTO memories_fts(memories_fts) VALUES('rebuild');")?;
                Ok(())
            })
            .await??;
        }

        // Step 2: Re-embed memories with NULL vectors, if embedder is configured
        if self.embedder.read().dimensions() == 0 {
            return Ok(0);
        }

        let conn = self.conn.clone();
        let entries: Vec<(String, String)> = tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt =
                conn.prepare("SELECT id, content FROM memories WHERE embedding IS NULL")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            Ok::<_, anyhow::Error>(rows.filter_map(std::result::Result::ok).collect())
        })
        .await??;

        let mut count = 0;
        for (id, content) in &entries {
            if let Ok(Some(emb)) = self.get_or_compute_embedding(content).await {
                let bytes = vector::vec_to_bytes(&emb);
                let conn = self.conn.clone();
                let id = id.clone();
                tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    let conn = conn.lock();
                    conn.execute(
                        "UPDATE memories SET embedding = ?1 WHERE id = ?2",
                        params![bytes, id],
                    )?;
                    Ok(())
                })
                .await??;
                count += 1;
            }
        }

        Ok(count)
    }

    async fn export(&self, filter: &ExportFilter) -> anyhow::Result<Vec<MemoryEntry>> {
        let conn = self.conn.clone();
        let filter = filter.clone();

        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<MemoryEntry>> {
            let conn = conn.lock();
            let mut sql =
                "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id \
                 FROM memories m LEFT JOIN agents a ON a.id = m.agent_id \
                 WHERE 1=1"
                    .to_string();
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut idx = 1;

            if let Some(ref ns) = filter.namespace {
                let _ = write!(sql, " AND m.namespace = ?{idx}");
                param_values.push(Box::new(ns.clone()));
                idx += 1;
            }
            if let Some(ref sid) = filter.session_id {
                let _ = write!(sql, " AND m.session_id = ?{idx}");
                param_values.push(Box::new(sid.clone()));
                idx += 1;
            }
            if let Some(ref cat) = filter.category {
                let _ = write!(sql, " AND m.category = ?{idx}");
                param_values.push(Box::new(category_to_str(cat)));
                idx += 1;
            }
            if let Some(ref since) = filter.since {
                let _ = write!(sql, " AND m.created_at >= ?{idx}");
                param_values.push(Box::new(since.clone()));
                idx += 1;
            }
            if let Some(ref until) = filter.until {
                let _ = write!(sql, " AND m.created_at <= ?{idx}");
                param_values.push(Box::new(until.clone()));
                let _ = idx;
            }
            sql.push_str(" ORDER BY m.created_at ASC");

            let mut stmt = conn.prepare(&sql)?;
            let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            let rows = stmt.query_map(params_ref.as_slice(), |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    category: str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: decode_kind(row.get(9)?),
                    pinned: row.get::<_, i64>(10)? != 0,
                    tenant_id: row.get(13)?,
                    agent_alias: row.get(11)?,
                    agent_id: row.get(12)?,
                })
            })?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
        .await?
    }

    async fn export_agent(&self, agent_alias: &str) -> anyhow::Result<Vec<MemoryEntry>> {
        let conn = self.conn.clone();
        let agent_alias = agent_alias.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<MemoryEntry>> {
            let conn = conn.lock();
            let mut stmt = conn.prepare(
                "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id \
                 FROM memories m LEFT JOIN agents a ON a.id = m.agent_id \
                 WHERE m.agent_id = (SELECT id FROM agents WHERE alias = ?1 LIMIT 1) \
                 ORDER BY m.created_at ASC",
            )?;
            let rows = stmt.query_map(params![agent_alias], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    category: str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: decode_kind(row.get(9)?),
                    pinned: row.get::<_, i64>(10)? != 0,
                    tenant_id: row.get(13)?,
                    agent_alias: row.get(11)?,
                    agent_id: row.get(12)?,
                })
            })?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
        .await?
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
            .recall(query, limit * 2, session_id, since, until)
            .await?;
        let filtered: Vec<MemoryEntry> = entries
            .into_iter()
            .filter(|e| e.namespace == namespace)
            .take(limit)
            .collect();
        Ok(filtered)
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

    async fn supersede(&self, superseded_ids: &[String], new_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        let ids = superseded_ids.to_vec();
        let new_id = new_id.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn.lock();
            crate::conflict::mark_superseded(&conn, &ids, &new_id)
        })
        .await?
    }

    async fn count_in_scope(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
    ) -> anyhow::Result<u64> {
        let conn = self.conn.clone();
        let namespace = namespace.map(str::to_string);
        let category = category.map(category_to_str);
        tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
            let conn = conn.lock();
            let count = match (namespace, category) {
                (Some(ns), Some(cat)) => conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND category = ?2 AND superseded_by IS NULL",
                    params![ns, cat],
                    |row| row.get::<_, u64>(0),
                )?,
                (Some(ns), None) => conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND superseded_by IS NULL",
                    params![ns],
                    |row| row.get::<_, u64>(0),
                )?,
                (None, Some(cat)) => conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE category = ?1 AND superseded_by IS NULL",
                    params![cat],
                    |row| row.get::<_, u64>(0),
                )?,
                (None, None) => conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE superseded_by IS NULL",
                    [],
                    |row| row.get::<_, u64>(0),
                )?,
            };
            Ok(count)
        })
        .await?
    }

    async fn stats(&self) -> anyhow::Result<MemoryStats> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<MemoryStats> {
            let conn = conn.lock();
            let total_rows = conn.query_row("SELECT COUNT(*) FROM memories", [], |row| {
                row.get::<_, u64>(0)
            })?;
            let superseded_rows = conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE superseded_by IS NOT NULL",
                [],
                |row| row.get::<_, u64>(0),
            )?;
            let pinned_rows = conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE pinned = 1",
                [],
                |row| row.get::<_, u64>(0),
            )?;
            let bytes = conn.query_row(
                "SELECT COALESCE(SUM(LENGTH(content)), 0) FROM memories",
                [],
                |row| row.get::<_, u64>(0),
            )?;
            let mut stmt =
                conn.prepare("SELECT category, COUNT(*) FROM memories GROUP BY category")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;

            let mut by_category = Vec::new();
            for row in rows {
                by_category.push(row?);
            }

            Ok(MemoryStats {
                total_rows,
                by_category,
                superseded_rows,
                pinned_rows,
                bytes,
            })
        })
        .await?
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
        if allowed_agent_ids.is_empty() {
            return self.recall(query, limit, session_id, since, until).await;
        }

        let full_candidate_limit = self.count().await?.max(limit);
        let raw = self
            .recall(query, full_candidate_limit, session_id, since, until)
            .await?;
        if raw.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.clone();
        let ids: Vec<String> = raw.iter().map(|e| e.id.clone()).collect();
        let allowed: Vec<String> = allowed_agent_ids.iter().map(|s| (*s).to_string()).collect();

        let kept: HashSet<String> =
            tokio::task::spawn_blocking(move || -> anyhow::Result<HashSet<String>> {
                let conn = conn.lock();
                let id_placeholders: String = (1..=ids.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let agent_placeholders: String = (ids.len() + 1..=ids.len() + allowed.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT id FROM memories \
                     WHERE id IN ({id_placeholders}) \
                       AND agent_id IN ({agent_placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
                    Vec::with_capacity(ids.len() + allowed.len());
                for id in &ids {
                    params.push(Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>);
                }
                for aid in &allowed {
                    params.push(Box::new(aid.clone()) as Box<dyn rusqlite::types::ToSql>);
                }
                let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(AsRef::as_ref).collect();
                let rows = stmt.query_map(params_ref.as_slice(), |row| row.get::<_, String>(0))?;
                let mut set = HashSet::new();
                for row in rows {
                    set.insert(row?);
                }
                Ok(set)
            })
            .await??;

        Ok(raw
            .into_iter()
            .filter(|e| kept.contains(&e.id))
            .take(limit)
            .collect())
    }

    async fn ensure_agent_uuid(&self, alias: &str) -> anyhow::Result<String> {
        let conn = self.conn.clone();
        let alias = alias.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let conn = conn.lock();
            sqlite_ensure_agent_uuid(&conn, &alias)
        })
        .await?
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
