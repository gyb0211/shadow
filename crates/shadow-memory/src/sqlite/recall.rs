//! SQLite memory -- 检索路径
//!
//! `recall_by_time_only`: 纯时间排序召回 (空查询 / "*")
//! `fts5_search`: FTS5 BM25 关键词检索
//! `vector_search`: 向量余弦相似度检索
//! `embedder_dimensions`: embedder 维度查询

use crate::sqlite::util::{decode_kind, fts5_term_query, str_to_category};
use crate::sqlite::SqliteMemory;
use crate::vector;
use rusqlite::{params, Connection};
use shadow_core::kennel::memory::MemoryEntry;
use std::fmt::Write as _;

impl SqliteMemory {
    /// 时间排序召回: 跳过 FTS / 向量, 仅按 created_at 过滤 + updated_at 排序。
    pub(super) async fn recall_by_time_only(
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

            let _ = write!(sql, " ORDER BY m.updated_at DESC LIMIT ?{idx}");
            param_values.push(Box::new(limit as i64));

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

            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
        .await?
    }

    /// FTS5 BM25 关键词检索, 返回 (id, score) 列表。
    pub(super) fn fts5_search(
        conn: &Connection,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, f32)>> {
        let fts_query: String = query
            .split_whitespace()
            .map(fts5_term_query)
            .collect::<Vec<_>>()
            .join(" OR ");

        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let sql = "SELECT m.id, bm25(memories_fts) as score
                   FROM memories_fts f
                   JOIN memories m ON m.rowid = f.rowid
                   WHERE memories_fts MATCH ?1
                   ORDER BY score
                   LIMIT ?2";

        let mut stmt = conn.prepare(sql)?;
        #[allow(clippy::cast_possible_wrap)]
        let limit_i64 = limit as i64;

        let rows = stmt.query_map(params![fts_query, limit_i64], |row| {
            let id: String = row.get(0)?;
            let score: f64 = row.get(1)?;
            #[allow(clippy::cast_possible_truncation)]
            Ok((id, (-score) as f32))
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// 向量余弦相似度检索: 全表扫描 embedding 列。
    pub(super) fn vector_search(
        conn: &Connection,
        query_embedding: &[f32],
        limit: usize,
        category: Option<&str>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<(String, f32)>> {
        let mut sql = "SELECT id, embedding FROM memories WHERE embedding IS NOT NULL".to_string();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1;

        if let Some(cat) = category {
            let _ = write!(sql, " AND category = ?{idx}");
            param_values.push(Box::new(cat.to_string()));
            idx += 1;
        }
        if let Some(sid) = session_id {
            let _ = write!(sql, " AND session_id = ?{idx}");
            param_values.push(Box::new(sid.to_string()));
        }

        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();
        let rows = stmt.query_map(params_ref.as_slice(), |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        })?;

        let mut scored: Vec<(String, f32)> = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            let emb = vector::bytes_to_vec(&blob);
            let sim = vector::cosine_similarity(query_embedding, &emb);
            if sim > 0.0 {
                scored.push((id, sim));
            }
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored)
    }

    pub(super) fn embedder_dimensions(&self) -> usize {
        self.embedder.read().dimensions()
    }
}
