//! SQLite memory -- 读路径 (query / export / stats)
//!
//! 所有方法都是 `impl SqliteMemory` 的 `pub(super)` inherent 方法,
//! 由 `mod.rs` 的 `impl Memory` trait 方法委托调用。

use crate::sqlite::util::{category_to_str, decode_kind, is_prefix_wildcard_term, like_fallback_matches, like_search_pattern, str_to_category};
use crate::sqlite::SqliteMemory;
use crate::vector;
use rusqlite::params;
use shadow_config::schema::SearchMode;
use shadow_core::kennel::memory::{is_recent_recall_query, ExportFilter, MemoryStats};
use shadow_core::MemoryCategory;
use shadow_core::MemoryEntry;
use std::collections::HashSet;
use std::fmt::Write as _;

impl SqliteMemory {
    /// [`Memory::recall`](shadow_core::Memory::recall) 的实现。
    pub(super) async fn recall_inner(
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

    /// [`Memory::get`](shadow_core::Memory::get) 的实现。
    pub(super) async fn get_inner(
        &self,
        key: &str,
    ) -> anyhow::Result<Option<MemoryEntry>> {
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

    /// [`Memory::get_for_agent`](shadow_core::Memory::get_for_agent) 的实现。
    pub(super) async fn get_for_agent_inner(
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

    /// [`Memory::list`](shadow_core::Memory::list) 的实现。
    pub(super) async fn list_inner(
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

    /// [`Memory::export`](shadow_core::Memory::export) 的实现。
    pub(super) async fn export_inner(
        &self,
        filter: &ExportFilter,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
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

    /// [`Memory::export_agent`](shadow_core::Memory::export_agent) 的实现。
    pub(super) async fn export_agent_inner(
        &self,
        agent_alias: &str,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
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

    /// [`Memory::count_in_scope`](shadow_core::Memory::count_in_scope) 的实现。
    pub(super) async fn count_in_scope_inner(
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

    /// [`Memory::stats`](shadow_core::Memory::stats) 的实现。
    pub(super) async fn stats_inner(&self) -> anyhow::Result<MemoryStats> {
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

    /// [`Memory::health_check`](shadow_core::Memory::health_check) 的实现。
    pub(super) async fn health_check_inner(&self) -> bool {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || conn.lock().execute_batch("SELECT 1").is_ok())
            .await
            .unwrap_or(false)
    }

    /// [`Memory::reindex`](shadow_core::Memory::reindex) 的实现。
    pub(super) async fn reindex_inner(&self) -> anyhow::Result<usize> {
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

    /// [`Memory::recall_for_agents`](shadow_core::Memory::recall_for_agents) 的实现。
    pub(super) async fn recall_for_agents_inner(
        &self,
        allowed_agent_ids: &[&str],
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        if allowed_agent_ids.is_empty() {
            return self.recall_inner(query, limit, session_id, since, until).await;
        }

        let full_candidate_limit = self.count_inner().await?.max(limit);
        let raw = self
            .recall_inner(query, full_candidate_limit, session_id, since, until)
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
}
