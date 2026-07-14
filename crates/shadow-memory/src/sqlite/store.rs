//! SQLite memory -- 写路径
//!
//! `store_row_with_metadata`: 核心写入 (含 embedding + upsert)
//! `get_or_compute_embedding`: embedding 缓存查询 / 按需计算
//! `swap_embedder`: 运行时替换 embedder

use crate::sqlite::agent::sqlite_ensure_default_agent_uuid;
use crate::sqlite::util::{category_to_str, content_hash};
use crate::vector;
use chrono::Local;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use shadow_core::kennel::memory::StoreOptions;
use shadow_core::MemoryCategory;
use std::sync::Arc;
use uuid::Uuid;

use crate::sqlite::SqliteMemory;
use crate::embedding::EmbeddingProvider;

impl SqliteMemory {
    pub(super) async fn store_row_with_metadata(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        options: StoreOptions,
        agent_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let embedding_bytes = match self.get_or_compute_embedding(content).await {
            Ok(emb) => emb.map(|emb| vector::vec_to_bytes(&emb)),
            Err(_e) => {
                // todo log
                None
            }
        };

        let conn = self.conn.clone();
        let key = key.to_string();
        let content = content.to_string();
        let sid = session_id.map(String::from);
        let ns = options.namespace.unwrap_or_else(|| "default".to_string());
        let imp = options.importance.unwrap_or(0.5);
        let kind = options
            .kind
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let pinned = i64::from(options.pinned);
        let tenant_id = options.tenant_id;
        let aid_in = agent_id.map(String::from);

        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn.lock();
            let now = Local::now().to_rfc3339();
            let cat = category_to_str(&category);
            let id = Uuid::new_v4().to_string();

            // When no explicit agent_id is given, ensure the default agent
            // row exists and resolve to its UUID. Without this, COALESCE
            // yields NULL and SQLite treats NULL agent_ids as distinct in
            // UNIQUE(agent_id, key) — so repeated stores of the same key
            // silently create duplicate rows instead of upserting.
            let aid = match aid_in {
                Some(id) => Some(id),
                None => Some(sqlite_ensure_default_agent_uuid(&conn)?),
            };

            conn.execute(
                "INSERT INTO memories (
                    id, key, content, category, embedding, created_at, updated_at,
                    session_id, namespace, importance, agent_id, kind, pinned, tenant_id
                 )
                 VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    COALESCE(?11, (SELECT id FROM agents WHERE alias = 'default' LIMIT 1)),
                    ?12, ?13, ?14
                 )
                 ON CONFLICT(agent_id, key) DO UPDATE SET
                    content = excluded.content,
                    category = excluded.category,
                    embedding = excluded.embedding,
                    updated_at = excluded.updated_at,
                    session_id = excluded.session_id,
                    namespace = excluded.namespace,
                    importance = excluded.importance,
                    kind = excluded.kind,
                    pinned = excluded.pinned,
                    tenant_id = excluded.tenant_id",
                params![
                    id, key, content, cat, embedding_bytes, now, now,
                    sid, ns, imp, aid, kind, pinned, tenant_id
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn get_or_compute_embedding(
        &self,
        query: &str,
    ) -> anyhow::Result<Option<Vec<f32>>> {
        let embedder = self.embedder.read().clone();

        if embedder.dimensions() == 0 {
            return Ok(None);
        }

        let hash = content_hash(query);
        let now = Local::now().to_rfc3339();

        let conn = self.conn.clone();

        let hash_c = hash.clone();
        let now_c = now.clone();
        let cached = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<Vec<f32>>> {
            let conn = conn.lock();
            let mut stmt =
                conn.prepare("SELECT embedding FROM embedding_cache WHERE content_hash = ?1")?;

            let blob: Option<Vec<u8>> = stmt.query_row(params![hash_c], |row| row.get(0)).ok();
            if let Some(bytes) = blob {
                conn.execute(
                    "UPDATE embedding_cache SET accessed_at = ?1 WHERE content_hash = ?2",
                    params![now_c, hash_c],
                )?;

                return Ok(Some(vector::bytes_to_vec(&bytes)));
            }
            Ok(None)
        })
        .await??;

        let embedding = embedder.embed_one(query).await?;
        let bytes = vector::vec_to_bytes(&embedding);
        let conn = self.conn.clone();

        let cache_max = self.cache_max as i64;
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn.lock();
            conn.execute(
                "INSERT OR REPLACE INTO embedding_cache (content_hash, embedding, created_at, accessed_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![hash, bytes, now, now],
            )?;
            conn.execute(
                "DELETE FROM embedding_cache WHERE content_hash IN (
                    SELECT content_hash FROM embedding_cache
                    ORDER BY accessed_at ASC
                    LIMIT MAX(0, (SELECT COUNT(*) FROM embedding_cache) - ?1)
                )",
                params![cache_max],
            )?;
            Ok(())
        })
        .await??;

        Ok(Some(embedding))
    }

    pub(crate) fn swap_embedder(&self, embedder: Arc<dyn EmbeddingProvider>) {
        *self.embedder.write() = embedder;
    }

    /// 仅供 store 路径使用: 在 spawn_blocking 闭包内持有锁
    #[allow(dead_code)]
    pub(super) fn conn_for_test(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }
}
