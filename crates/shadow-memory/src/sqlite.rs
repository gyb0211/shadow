//! SQLite 记忆后端 -- 使用 SQLite + FTS5 全文检索
//!
//! 表结构:
//! - memory_entries: 主表, 存储记忆条目 (id TEXT PK)
//! - memory_fts: FTS5 虚拟表, trigram 分词, 用于全文搜索 (外部内容表)
//!
//! FTS 索引通过触发器自动同步, 业务代码只需操作主表

use crate::embedding::{EmbeddingProvider, NoopEmbedding, create_embedding_provider};
use anyhow::Context;
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, ErrorCode, Row, params};
use std::fmt::Write as _;

use shadow_config::schema::SearchMode;
use shadow_core::kennel::memory::{is_recent_recall_query, MemoryKind};
use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, Role};
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, RwLock, mpsc};
use std::thread;
use std::time::Duration;

const SQLITE_OPEN_TIMEOUT_CAP_SECS: u64 = 300;
static SQLITE_MEMORY_STARTUP_LOCK: StdMutex<()> = StdMutex::new(());

fn acquire_sqlite_startup_lock() -> MutexGuard<'static, ()> {
    SQLITE_MEMORY_STARTUP_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// SQLite 记忆后端
///
/// 使用 SQLite 存储记忆条目, FTS5 提供全文搜索能力。
/// 采用 WAL 模式提升并发读写性能, trigram 分词支持中英文混合搜索。
/// 可选注入 EmbeddingProvider 实现语义搜索 (FTS5 + FTS5向量混合检索)。
pub struct SqliteMemory {
    alias: String,
    conn: Arc<Mutex<Connection>>,
    /// 保留 embedding provider 引用 (预留给后续语义搜索功能, 当前仅使用 FTS5)
    #[allow(dead_code)]
    embedder: RwLock<Arc<dyn EmbeddingProvider>>,
    vector_weight: f32,
    keyword_weight: f32,
    cache_max: usize,
    search_mode: SearchMode,
}

impl SqliteMemory {
    fn open_connection(
        db_path: &Path,
        open_timeout_secs: Option<u64>,
    ) -> anyhow::Result<Connection> {
        let path_buf = db_path.to_path_buf();
        let conn = if let Some(secs) = open_timeout_secs {
            let capped = secs.min(SQLITE_OPEN_TIMEOUT_CAP_SECS);
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let result = Connection::open(&path_buf);
                let _ = tx.send(result);
            });

            match rx.recv_timeout(Duration::from_secs(capped)) {
                Ok(Ok(c)) => c,
                Ok(Err(e)) => return Err(e).context("Sqlite failed to open database"),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    anyhow::bail!(format!(
                        "Sqlite connection open timeout after {} seconds",
                        capped
                    ))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("Sqlite open thread exit unexpectedly")
                }
            }
        } else {
            Connection::open(&path_buf).context("SQLite failed to open database")?
        };

        Ok(conn)
    }

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

        let conn = Self::open_connection(&db_path, open_timeout_secs)?;

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA mmap_size    = 8388608;
             PRAGMA cache_size   = -2000;
             PRAGMA temp_store   = MEMORY;",
        )?;

        Self::init_schema(&conn)?;

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
            10_1000,
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

        let conn = Self::open_connection(&db_path, None)?;
        conn.execute_batch(
            // foreign_keys is OFF by default in SQLite and is a
            // per-connection PRAGMA, so the multi-agent migration's
            // `REFERENCES agents(id)` constraint would be unenforced
            // without this. Set it before any writes flow through.
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA mmap_size    = 8388608;
             PRAGMA cache_size   = -2000;
             PRAGMA temp_store   = MEMORY;",
        );

        Self::init_schema(&conn)?;

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

    fn init_schema(conn: &Connection) -> anyhow::Result<()> {
        fn is_db_locked_error(e: &rusqlite::Error) -> bool {
            use rusqlite::ffi::Error;
            matches!(e, rusqlite::Error::SqliteFailure(err, _)
            if matches!(err.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked))
        }

        fn execute_batch_retry(conn: &Connection, sql: &str) -> Result<(), rusqlite::Error> {
            let mut backoff = Duration::from_millis(10);
            let max_backoff = Duration::from_millis(200);
            let max_attempts: usize = 24;

            for attempt in 1..=max_attempts {
                match conn.execute_batch(sql) {
                    Ok(()) => return Ok(()),
                    Err(e) if is_db_locked_error(&e) && attempt < max_attempts => {
                        std::thread::sleep(backoff);
                        backoff = (backoff * 2).min(max_backoff)
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        }

        fn memories_has_column(conn: &Connection, name: &str) -> anyhow::Result<bool> {
            let mut stmt = conn.prepare("PRAGMA table_info(memories)")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let col_name: String = row.get(1)?;
                if col_name == name {
                    return Ok(true);
                }
            }
            Ok(false)
        }

        fn is_duplicate_column_error(e: &rusqlite::Error) -> bool {
            matches!(
                e,
                rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.contains("duplicate column name")
            )
        }

        fn add_memories_column_if_missing(
            conn: &Connection,
            name: &str,
            alter_sql: &str,
        ) -> anyhow::Result<()> {
            if memories_has_column(conn, name)? {
                return Ok(());
            }

            match execute_batch_retry(conn, alter_sql) {
                Ok(()) => Ok(()),
                Err(e) if is_duplicate_column_error(&e) => Ok(()),
                Err(e) => Err(e)
                    .with_context(|| format!("SQLite migration failed adding memories.{name}")),
            }
        }
        execute_batch_retry(
            conn,
            "-- Core memories table. This is an intermediate shape; the V3
            -- migration in `zeroclaw_config::schema::v2::migrate_sqlite_memory_to_v3`
            -- rebuilds it with the `agent_id` column and a composite
            -- `UNIQUE (agent_id, key)` constraint immediately after init.
            CREATE TABLE IF NOT EXISTS memories (
                id          TEXT PRIMARY KEY,
                key         TEXT NOT NULL UNIQUE,
                content     TEXT NOT NULL,
                category    TEXT NOT NULL DEFAULT 'core',
                embedding   BLOB,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
            CREATE INDEX IF NOT EXISTS idx_memories_key ON memories(key);

            -- FTS5 full-text search (BM25 scoring)
            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                key, content, content=memories, content_rowid=rowid
            );

            -- FTS5 triggers: keep in sync with memories table
            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, key, content)
                VALUES (new.rowid, new.key, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, key, content)
                VALUES ('delete', old.rowid, old.key, old.content);
            END;
            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, key, content)
                VALUES ('delete', old.rowid, old.key, old.content);
                INSERT INTO memories_fts(rowid, key, content)
                VALUES (new.rowid, new.key, new.content);
            END;

            -- Embedding cache with LRU eviction
            CREATE TABLE IF NOT EXISTS embedding_cache (
                content_hash TEXT PRIMARY KEY,
                embedding    BLOB NOT NULL,
                created_at   TEXT NOT NULL,
                accessed_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cache_accessed ON embedding_cache(accessed_at);",
        )
        .with_context(|| "SQLite init_schema failed: CREATE base schema")?;

        add_memories_column_if_missing(
            conn,
            "session_id",
            "ALTER TABLE memories ADD COLUMN session_id TEXT;",
        )?;
        execute_batch_retry(
            conn,
            "CREATE INDEX IF NOT EXISTS idx_memories_session ON memories(session_id);",
        )
        .with_context(|| "SQLite init_schema failed: CREATE INDEX idx_memories_session")?;

        add_memories_column_if_missing(
            conn,
            "namespace",
            "ALTER TABLE memories ADD COLUMN namespace TEXT DEFAULT 'default';",
        )?;
        execute_batch_retry(
            conn,
            "CREATE INDEX IF NOT EXISTS idx_memories_namespace ON memories(namespace);",
        )
        .with_context(|| "SQLite init_schema failed: CREATE INDEX idx_memories_namespace")?;

        add_memories_column_if_missing(
            conn,
            "importance",
            "ALTER TABLE memories ADD COLUMN importance REAL DEFAULT 0.5;",
        )?;

        add_memories_column_if_missing(
            conn,
            "superseded_by",
            "ALTER TABLE memories ADD COLUMN superseded_by TEXT;",
        )?;
        add_memories_column_if_missing(conn, "kind", "ALTER TABLE memories ADD COLUMN kind TEXT;")?;
        add_memories_column_if_missing(
            conn,
            "pinned",
            "ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;",
        )?;
        add_memories_column_if_missing(
            conn,
            "tenant_id",
            "ALTER TABLE memories ADD COLUMN tenant_id TEXT;",
        )?;
        execute_batch_retry(
            conn,
            "CREATE INDEX IF NOT EXISTS idx_memories_namespace_category ON memories(namespace, category);",
        )
            .with_context(|| "SQLite init_schema failed: CREATE INDEX idx_memories_namespace_category")?;

        // Self::migrate_session_ids_to_sanitized(conn)?;

        Ok(())
    }

    fn str_to_category(s: &str) -> MemoryCategory {
        match s {
            "core" => MemoryCategory::Core,
            "daily" => MemoryCategory::Daily,
            "conversation" => MemoryCategory::Conversation,
            other => MemoryCategory::Custom(other.to_string()),
        }
    }
    fn decode_kind(raw: Option<String>) -> Option<MemoryKind> {
        raw.and_then(|kind| serde_json::from_str(&kind).ok())
    }

    async fn recall_by_time_only(
        &self,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let conn = self.conn.clone();

        let sid = session_id.map(String::from);
        let since_owned = since.map(String::from);
        let until_owned = until.map(String::from);

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let since_ref = since_owned.as_deref();
            let until_ref = until_owned.as_deref();


            let mut sql =
                "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id \
                 FROM memories m LEFT JOIN agents a ON a.id = m.agent_id \
                 WHERE m.superseded_by IS NULL AND 1=1"
                    .to_string();

            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut idx = 1;

            if let Some(sid) = sid.as_deref() {
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
                idx += 1;
            }

            let _ = write!(sql, "ORDER BY m.updated_at DESC LIMIT ?{idx}");
            param_values.push(Box::new(limit as i64));

            let mut stmt = conn.prepare(&sql)?;
            let params_ref : Vec<&dyn rusqlite::types::ToSql>=
            param_values.iter().map(AsRef::as_ref).collect();
            let rows = stmt.query_map(params_ref.as_slice(), |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    category: Self::str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: Self::decode_kind(row.get(9)?),
                    pinned: row.get::<_, i64>(10)? != 0,
                    tenant_id: row.get(13)?,
                    agent_alias: row.get(11)?,
                    agent_id: row.get(12)?,
                })
            } )?;

            let mut results = Vec::new();

            for row in rows {
                results.push(row?);
            }

            Ok(results)

        }).await?
    }
}
impl Attributable for SqliteMemory {
    fn role(&self) -> Role {
        Role::Memory(MemoryKind::Sqlite)
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
        self.store_with_agent(key, content, category, session_id, None, None, None)
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

        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<MemoryEntry>>{
            let conn = conn.lock();
            let session_ref = sid.as_deref();
            let since_ref = since_owned.as_deref();
            let until_ref = until_owned.as_deref();

            let kw_result = if search_mode == SearchMode::Bm25 { Vec::new() } else {
                Self::fts5_search(&conn, &query, limit * 2).unwrap_or_default()
            };

            let vector_result = if search_mode == SearchMode::Bm25 { Vec::new() } else if let Some(ref qe) = query_embedding {
                Self::vector_search(&conn, &query, limit * 2, None, session_ref).unwrap_or_default()
            } else {
                Vec::new()
            };

            let merged = if vector_result.is_empty() {
                kw_result.iter().map(|(id, score)| vector::ScoredResult {
                    id: id.clone(),
                    vector_score: None,
                    keyword_score: Some(*score),
                    final_score: *score,
                }).collect::<Vec<_>>()
            } else if kw_result.is_empty() {
                vector_result.iter().map(|(id, score)| vector::ScoredResult {
                    id: id.clone(),
                    vector_score: None,
                    keyword_score: Some(*score),
                    final_score: *score,
                }).collect::<Vec<_>>()
            } else {
                vector::hybrid_merge(&vector_result, &kw_result, vector_weight, keyword_weight)
            };

            let mut results = Vec::new();
            if merged.is_empty() {
                const MAX_LIKE_KEYWORDS: usize = 8;
                let raw_keywords: Vec<String> = query.split_whitespace().take(MAX_LIKE_KEYWORDS).map(str::to_string).collect();
                if !raw_keywords.is_empty() {
                    let needs_prefix_filter = raw_keywords.iter().any(|keyword| Self::is_prefix_wildcard_term(keyword));
                    let sql_limit = if needs_prefix_filter {
                        limit.saturating_mul(8).min(limit.saturating_add(512))
                    } else {
                        limit
                    };

                    let patterns: Vec<String> = raw_keywords.inter().map(|kw| Self::like_search_pattern(kw)).collect();
                    let conditions: Vec<String> = patterns.iter().enumerate()
                        .map(|(i, _)| {
                            format!("(m.content like ?{} ESCAPE '\\' OR m.ley like ?{} ESCAPE '\\'}})",
                                    i * 2 + 1,
                                    i * 2 + 2, )
                        }).collect();

                    let were_clause = conditions.join(" OR ");
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
                            category: Self::str_to_category(&row.get::<_, String>(3)?),
                            timestamp: row.get(4)?,
                            session_id: row.get(5)?,
                            score: Some(1.0),
                            namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                            importance: row.get(7)?,
                            superseded_by: row.get(8)?,
                            kind: Self::decode_kind(row.get(9)?),
                            pinned: row.get::<_, i64>(10)? != 0,
                            tenant_id: row.get(13)?,
                            agent_alias: row.get(11)?,
                            agent_id: row.get(12)?,
                        })
                    })?;
                    for row in rows {
                        let entry = row?;
                        if let Some(sid) = session_ref
                            && entry.session_id.as_deref() != Some(sid) {
                            continue;
                        }
                        if needs_prefix_filter
                            && !raw_keywords.iter().any(|keyword| {
                            Self::like_fallback_matches(&entry.key, keyword)
                                || Self::like_fallback_matches(&entry.content, keyword)
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
            }
            results.truncate(limit);
                Ok(results)
            }).await?
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
                    category: Self::str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: Self::decode_kind(row.get(9)?),
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
                    category: Self::str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: Self::decode_kind(row.get(9)?),
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
                    category: Self::str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: Self::decode_kind(row.get(9)?),
                    pinned: row.get::<_, i64>(10)? != 0,
                    tenant_id: row.get(13)?,
                    agent_alias: row.get(11)?,
                    agent_id: row.get(12)?,
                })
            };

            if let Some(ref cat) = category {
                let cat_str = Self::category_to_str(cat);
                let mut stmt = conn.prepare(
                    "SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id, m.namespace, m.importance, m.superseded_by, m.kind, m.pinned, a.alias, m.agent_id, m.tenant_id
                     FROM memories m LEFT JOIN agents a ON a.id = m.agent_id
                     WHERE m.superseded_by IS NULL AND m.category = ?1 ORDER BY m.updated_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![cat_str, DEFAULT_LIST_LIMIT], row_mapper)?;
                for row in rows {
                    let entry = row?;
                    if let Some(sid) = session_ref
                        && entry.session_id.as_deref() != Some(sid) {
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
                        && entry.session_id.as_deref() != Some(sid) {
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
            // `agent_alias` is the human alias, but `memories.agent_id` holds
            // the agent's UUID (FK → agents.id). Resolve alias → id via the same
            // subselect the insert path uses (`store_with_agent`); binding the
            // alias straight into agent_id matches zero rows and silently
            // no-ops. An unknown alias yields a NULL subselect → matches
            // nothing, which is the correct outcome.
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
            // Memory rows ride `memories.agent_id` (FK → agents.id, a stable
            // UUID); only the human `alias` column moves, so this is a single
            // agents-row update. An unknown `from` matches nothing → Ok(0).
            //
            // Collision-safety: `agents.alias` is UNIQUE, and deleting an agent
            // purges its memories but leaves the `agents` row behind (an orphan
            // holding the alias). A bare UPDATE onto a previously-used-then-
            // deleted `to` alias would hit the UNIQUE constraint and fail. We
            // hold the connection lock across the whole sequence (single writer),
            // so: refuse if `to` still has memory rows (a genuine conflict we
            // won't silently merge), otherwise drop the orphan `to` row and
            // proceed. (`COUNT(*)` over a NULL subselect when no `to` row exists
            // is 0, so the common no-collision path falls straight through.)
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
            // Drop any orphan `to` agents row (verified above to own no memories).
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
            // Mirror `rename_agent`: it moves the `agents` row (alias -> id), not
            // the memory rows, so residue is the presence of that alias row (0 or
            // 1). A memory-row count would miss an agent with an `agents` row but
            // no memories - a real lag `rename_agent` would still re-point.
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

    /// Rebuild backend indexes: FTS tables and missing embedding vectors.
    ///
    /// Step 1 rebuilds the FTS5 index unconditionally (idempotent, cheap).
    /// Step 2 fills in vectors for every row with `embedding IS NULL` using
    /// the configured embedder. If interrupted, re-running is safe — only
    /// rows still missing a vector are re-processed. Intended to be run
    /// after bulk writes that didn't go through `store()` (e.g. `zeroclaw
    /// migrate openclaw`, which uses `NoopEmbedding` for speed). Returns
    /// the number of rows that received a new embedding; returns 0 if the
    /// embedder has no dimensions (Noop) or if everything is already
    /// embedded.
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
                param_values.push(Box::new(Self::category_to_str(cat)));
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
                    category: Self::str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: Self::decode_kind(row.get(9)?),
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
                    category: Self::str_to_category(&row.get::<_, String>(3)?),
                    timestamp: row.get(4)?,
                    session_id: row.get(5)?,
                    score: None,
                    namespace: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "default".into()),
                    importance: row.get(7)?,
                    superseded_by: row.get(8)?,
                    kind: Self::decode_kind(row.get(9)?),
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
        // Same routing rule as `store`: no agent context at the trait
        // boundary, so attribute to the default agent through
        // `store_with_agent`.
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
        let category = category.map(Self::category_to_str);
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
            let by_category = rows.collect::<Result<Vec<_>, _>>()?;
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
        // Empty allowlist means "no agent filter": fall back to plain
        // recall. The wrapper always includes the bound agent's UUID,
        // so a non-empty allowlist is the live-runtime case.
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

        // Single SQL pass that returns only the candidate IDs whose
        // agent_id is on the allowlist. Legacy NULL-agent_id rows do
        // not match (the V3 migration backfills `default`, and the
        // NOT NULL FK rejects new NULLs), so cross-agent leakage of
        // unattributed rows that an earlier post-fetch fall-through
        // would have allowed is closed at the query boundary.
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
            zeroclaw_config::schema::v2::sqlite_ensure_agent_uuid(&conn, &alias)
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
