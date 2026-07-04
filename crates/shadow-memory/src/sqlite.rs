//! SQLite 记忆后端 -- 使用 SQLite + FTS5 全文检索
//!
//! 表结构:
//! - memory_entries: 主表, 存储记忆条目 (id TEXT PK)
//! - memory_fts: FTS5 虚拟表, trigram 分词, 用于全文搜索 (外部内容表)
//!
//! FTS 索引通过触发器自动同步, 业务代码只需操作主表

use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, Role};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{params, Connection, Row};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::embedding::EmbeddingProvider;

/// SQLite 记忆后端
///
/// 使用 SQLite 存储记忆条目, FTS5 提供全文搜索能力。
/// 采用 WAL 模式提升并发读写性能, trigram 分词支持中英文混合搜索。
/// 可选注入 EmbeddingProvider 实现语义搜索 (FTS5 + 向量混合检索)。
pub struct SqliteMemory {
    conn: Mutex<Connection>,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
}

impl SqliteMemory {
    /// 创建不带 embedding 的实例 (纯 FTS5 检索)
    pub fn new(workspace_dir: &Path) -> Result<Self> {
        Self::with_embedding(workspace_dir, None)
    }

    /// 创建带 embedding provider 的实例 (混合检索)
    pub fn with_embedding(
        workspace_dir: &Path,
        embedder: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Self> {
        let _ = std::fs::create_dir_all(workspace_dir);
        let db_path = workspace_dir.join("memory.db");

        let conn = Connection::open(&db_path)
            .with_context(|| format!("无法打开记忆数据库: {}", db_path.display()))?;

        Self::init_db(&conn)?;
        Ok(Self { conn: Mutex::new(conn), embedder })
    }

    fn init_db(conn: &Connection) -> Result<()> {
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("无法设置 WAL 模式")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_entries (
                id           TEXT PRIMARY KEY,
                key          TEXT,
                content      TEXT,
                category     TEXT,
                timestamp    TEXT,
                session_id   TEXT,
                agent_alias  TEXT
            );",
        )
        .context("无法创建 memory_entries 表")?;

        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                content,
                content='memory_entries',
                content_rowid='rowid',
                tokenize='trigram'
            );",
        )
        .context("无法创建 memory_fts 虚拟表")?;

        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS memory_fts_ai AFTER INSERT ON memory_entries BEGIN
                INSERT INTO memory_fts(rowid, content) VALUES (new.rowid, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_fts_ad AFTER DELETE ON memory_entries BEGIN
                INSERT INTO memory_fts(memory_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_fts_au AFTER UPDATE ON memory_entries BEGIN
                INSERT INTO memory_fts(memory_fts, rowid, content) VALUES('delete', old.rowid, old.content);
                INSERT INTO memory_fts(rowid, content) VALUES (new.rowid, new.content);
            END;",
        )
        .context("无法创建 FTS 同步触发器")?;

        Ok(())
    }

    /// 将数据库行解析为 MemoryEntry
    fn row_to_entry(row: &Row) -> rusqlite::Result<MemoryEntry> {
        let category_str: String = row.get(3)?;
        Ok(MemoryEntry {
            id: row.get(0)?,
            key: row.get(1)?,
            content: row.get(2)?,
            category: MemoryCategory::from_str(&category_str),
            timestamp: row.get(4)?,
            session_id: row.get(5)?,
            score: None,
            agent_alias: row.get(6)?,
        })
    }

    /// 将查询字符串转为 FTS5 安全的 MATCH 表达式
    fn fts_query(query: &str) -> String {
        let cleaned = query.replace('"', "");
        format!("\"{cleaned}\"")
    }
}

impl Attributable for SqliteMemory {
    fn role(&self) -> Role {
        Role::Memory
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
    ) -> Result<()> {
        let conn = self.conn.lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT OR REPLACE INTO memory_entries
                (id, key, content, category, timestamp, session_id, agent_alias)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                key,
                content,
                category.as_str(),
                timestamp,
                session_id,
                None::<&str>, // agent_alias 暂不使用
            ],
        )
        .context("存储记忆失败")?;

        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let query = query.trim();

        // trigram 分词器需要至少 3 个字符
        if query.chars().count() < 3 {
            // 短查询: 返回最近记忆
            return self.recent(limit, session_id).await;
        }

        let fts = Self::fts_query(query);
        let conn = self.conn.lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let entries = if let Some(sid) = session_id {
            let mut stmt = conn
                .prepare("SELECT me.id, me.key, me.content, me.category, me.timestamp, me.session_id, me.agent_alias
                 FROM memory_fts
                 JOIN memory_entries me ON me.rowid = memory_fts.rowid
                 WHERE memory_fts MATCH ?1 AND me.session_id = ?2
                 ORDER BY rank
                 LIMIT ?3")
                .context("准备检索查询失败")?;
            stmt.query_map(params![fts, sid, limit as i64], |row| Self::row_to_entry(row))
                .context("执行检索查询失败")?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn
                .prepare("SELECT me.id, me.key, me.content, me.category, me.timestamp, me.session_id, me.agent_alias
                 FROM memory_fts
                 JOIN memory_entries me ON me.rowid = memory_fts.rowid
                 WHERE memory_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2")
                .context("准备检索查询失败")?;
            stmt.query_map(params![fts, limit as i64], |row| Self::row_to_entry(row))
                .context("执行检索查询失败")?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        Ok(entries)
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let conn = self.conn.lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let mut stmt = conn
            .prepare("SELECT id, key, content, category, timestamp, session_id, agent_alias
                 FROM memory_entries WHERE key = ?1")
            .context("准备查询失败")?;

        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(Self::row_to_entry(row)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, category: Option<&MemoryCategory>) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let entries = if let Some(cat) = category {
            let mut stmt = conn
                .prepare("SELECT id, key, content, category, timestamp, session_id, agent_alias
                 FROM memory_entries WHERE category = ?1 ORDER BY timestamp DESC")
                .context("准备列表查询失败")?;
            stmt.query_map(params![cat.as_str()], |row| Self::row_to_entry(row))
                .context("执行列表查询失败")?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn
                .prepare("SELECT id, key, content, category, timestamp, session_id, agent_alias
                 FROM memory_entries ORDER BY timestamp DESC")
                .context("准备列表查询失败")?;
            stmt.query_map([], |row| Self::row_to_entry(row))
                .context("执行列表查询失败")?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        Ok(entries)
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        let conn = self.conn.lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let changed = conn.execute(
            "DELETE FROM memory_entries WHERE key = ?1",
            params![key],
        )
        .context("删除记忆失败")?;

        Ok(changed > 0)
    }

    async fn count(&self) -> Result<usize> {
        let conn = self.conn.lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_entries", [], |row| row.get(0))
            .context("计数查询失败")?;

        Ok(count as usize)
    }

    fn health_check(&self) -> bool {
        let conn = self.conn.lock();
        let Ok(conn) = conn else { return false };
        conn.query_row("SELECT 1", [], |_| Ok(())).is_ok()
    }
}

impl SqliteMemory {
    /// 返回最近的记忆 (短查询时使用)
    async fn recent(&self, limit: usize, session_id: Option<&str>) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let entries = if let Some(sid) = session_id {
            let mut stmt = conn
                .prepare("SELECT id, key, content, category, timestamp, session_id, agent_alias
                 FROM memory_entries WHERE session_id = ?1 ORDER BY timestamp DESC LIMIT ?2")?;
            stmt.query_map(params![sid, limit as i64], |row| Self::row_to_entry(row))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn
                .prepare("SELECT id, key, content, category, timestamp, session_id, agent_alias
                 FROM memory_entries ORDER BY timestamp DESC LIMIT ?1")?;
            stmt.query_map(params![limit as i64], |row| Self::row_to_entry(row))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_test_memory() -> (tempfile::TempDir, SqliteMemory) {
        let dir = tempdir().expect("无法创建临时目录");
        let mem = SqliteMemory::new(dir.path()).expect("无法创建 SqliteMemory");
        (dir, mem)
    }

    #[tokio::test]
    async fn store_and_recall() {
        let (_dir, mem) = make_test_memory();

        mem.store("rust", "Rust 是一门系统编程语言, 注重安全和性能", MemoryCategory::Core, Some("s1"))
            .await.unwrap();
        mem.store("python", "Python 是一门动态脚本语言, 注重简洁和易用", MemoryCategory::Core, Some("s2"))
            .await.unwrap();

        let results = mem.recall("Rust", 10, None).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].key, "rust");

        let results = mem.recall("系统编程", 10, None).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].key, "rust");
    }

    #[tokio::test]
    async fn recall_with_session_filter() {
        let (_dir, mem) = make_test_memory();

        mem.store("k1", "hello world content", MemoryCategory::Core, Some("session-a"))
            .await.unwrap();
        mem.store("k2", "hello rust content", MemoryCategory::Core, Some("session-b"))
            .await.unwrap();

        let results = mem.recall("hello", 10, Some("session-a")).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "k1");
    }

    #[tokio::test]
    async fn get_and_forget() {
        let (_dir, mem) = make_test_memory();

        mem.store("test-key", "测试内容", MemoryCategory::Daily, None)
            .await.unwrap();

        let got = mem.get("test-key").await.unwrap().unwrap();
        assert_eq!(got.key, "test-key");
        assert_eq!(got.content, "测试内容");
        assert_eq!(got.category, MemoryCategory::Daily);

        assert!(mem.forget("test-key").await.unwrap());
        assert!(mem.get("test-key").await.unwrap().is_none());
        assert!(!mem.forget("nonexistent").await.unwrap());
    }

    #[tokio::test]
    async fn list_by_category() {
        let (_dir, mem) = make_test_memory();

        mem.store("k1", "a", MemoryCategory::Core, None).await.unwrap();
        mem.store("k2", "b", MemoryCategory::Daily, None).await.unwrap();

        let core = mem.list(Some(&MemoryCategory::Core)).await.unwrap();
        assert_eq!(core.len(), 1);
        assert_eq!(core[0].key, "k1");

        let all = mem.list(None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn count_and_health() {
        let (_dir, mem) = make_test_memory();

        assert_eq!(mem.count().await.unwrap(), 0);
        assert!(mem.health_check());

        mem.store("k1", "a", MemoryCategory::Core, None).await.unwrap();
        mem.store("k2", "b", MemoryCategory::Core, None).await.unwrap();
        assert_eq!(mem.count().await.unwrap(), 2);
    }
}
