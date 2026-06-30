//! SQLite 记忆后端 -- 使用 SQLite + FTS5 全文检索
//!
//! 表结构:
//! - memory_entries: 主表, 存储记忆条目 (id TEXT PK)
//! - memory_fts: FTS5 虚拟表, trigram 分词, 用于全文搜索 (外部内容表)
//!
//! FTS 索引通过触发器自动同步, 业务代码只需操作主表

use agent_core::{Attributable, Memory, MemoryEntry, Role};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Row};
use std::path::Path;
use std::sync::Mutex;

/// SQLite 记忆后端
///
/// 使用 SQLite 存储记忆条目, FTS5 提供全文搜索能力。
/// 采用 WAL 模式提升并发读写性能, trigram 分词支持中英文混合搜索。
pub struct SqliteMemory {
    /// 数据库连接 (Mutex 保证线程安全, Connection 是 Send 但非 Sync)
    conn: Mutex<Connection>,
}

impl SqliteMemory {
    /// 创建新的 SQLite 记忆后端
    ///
    /// # 参数
    /// - `workspace_dir`: 工作目录, 数据库文件存为 `workspace_dir/memory.db`
    ///
    /// 自动开启 WAL 模式并建表, 失败返回错误
    pub fn new(workspace_dir: &Path) -> Result<Self> {
        // 确保工作目录存在
        let _ = std::fs::create_dir_all(workspace_dir);
        let db_path = workspace_dir.join("memory.db");

        let conn = Connection::open(&db_path)
            .with_context(|| format!("无法打开记忆数据库: {}", db_path.display()))?;

        // 初始化数据库结构
        Self::init_db(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 初始化数据库: WAL 模式 + 建表 + FTS 同步触发器
    fn init_db(conn: &Connection) -> Result<()> {
        // 开启 WAL 模式 (提升并发读写性能)
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("无法设置 WAL 模式")?;

        // 主表: 记忆条目
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

        // FTS5 虚拟表: 全文搜索
        // 使用 trigram 分词器, 支持中英文混合搜索
        // 外部内容表模式: content 关联 memory_entries, 通过 rowid 链接
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                content,
                content='memory_entries',
                content_rowid='rowid',
                tokenize='trigram'
            );",
        )
        .context("无法创建 memory_fts 虚拟表")?;

        // 触发器: 保持 FTS 索引与主表自动同步
        // - AFTER INSERT: 新增条目时, 插入 FTS 索引
        // - AFTER DELETE: 删除条目时, 清理 FTS 索引
        // - AFTER UPDATE: 更新条目时, 重建 FTS 索引
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
        let timestamp_str: String = row.get(4)?;
        // 解析 RFC3339 时间戳, 失败则用当前时间兜底
        let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(MemoryEntry {
            id: row.get(0)?,
            key: row.get(1)?,
            content: row.get(2)?,
            category: row.get(3)?,
            timestamp,
            session_id: row.get(5)?,
            agent_alias: row.get(6)?,
        })
    }

    /// 将查询字符串转为 FTS5 安全的 MATCH 表达式
    ///
    /// 用双引号包裹整个查询, 使其成为短语搜索, 避免特殊字符破坏 FTS5 语法
    fn fts_query(query: &str) -> String {
        // 移除内部双引号, 避免破坏短语匹配语法
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
    /// 存储记忆: INSERT OR REPLACE 主表, 触发器自动更新 FTS 索引
    async fn store(&self, entry: &MemoryEntry) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        conn.execute(
            "INSERT OR REPLACE INTO memory_entries
                (id, key, content, category, timestamp, session_id, agent_alias)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id,
                entry.key,
                entry.content,
                entry.category,
                entry.timestamp.to_rfc3339(),
                entry.session_id,
                entry.agent_alias,
            ],
        )
        .context("存储记忆失败")?;

        Ok(())
    }

    /// 检索记忆: FTS5 MATCH 全文搜索, 按 rank (BM25 相关性) 排序, 限制返回数量
    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let query = query.trim();

        // trigram 分词器需要至少 3 个字符才能生成三元组
        if query.chars().count() < 3 {
            return Ok(vec![]);
        }

        let fts = Self::fts_query(query);

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let mut stmt = conn
            .prepare(
                "SELECT me.id, me.key, me.content, me.category, me.timestamp, me.session_id, me.agent_alias
                 FROM memory_fts
                 JOIN memory_entries me ON me.rowid = memory_fts.rowid
                 WHERE memory_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .context("准备检索查询失败")?;

        let entries = stmt
            .query_map(params![fts, limit as i64], |row| Self::row_to_entry(row))
            .context("执行检索查询失败")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("解析检索结果失败")?;

        Ok(entries)
    }

    /// 获取单条记忆: 按 key 查询 (返回第一条匹配)
    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, key, content, category, timestamp, session_id, agent_alias
                 FROM memory_entries
                 WHERE key = ?1",
            )
            .context("准备查询失败")?;

        let mut rows = stmt.query(params![key])?;

        match rows.next()? {
            Some(row) => {
                let entry = Self::row_to_entry(row)?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    /// 列出全部记忆: 按时间戳倒序排列 (最新的在前)
    async fn list(&self) -> Result<Vec<MemoryEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, key, content, category, timestamp, session_id, agent_alias
                 FROM memory_entries
                 ORDER BY timestamp DESC",
            )
            .context("准备列表查询失败")?;

        let entries = stmt
            .query_map([], |row| Self::row_to_entry(row))
            .context("执行列表查询失败")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("解析列表结果失败")?;

        Ok(entries)
    }

    /// 删除记忆: DELETE 主表, 触发器自动清理 FTS 索引
    async fn forget(&self, key: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("数据库锁错误: {e}"))?;

        conn.execute("DELETE FROM memory_entries WHERE key = ?1", params![key])
            .context("删除记忆失败")?;

        Ok(())
    }
}

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// 创建临时目录中的测试用 SqliteMemory
    fn make_test_memory() -> (tempfile::TempDir, SqliteMemory) {
        let dir = tempdir().expect("无法创建临时目录");
        let mem = SqliteMemory::new(dir.path()).expect("无法创建 SqliteMemory");
        (dir, mem)
    }

    /// 构造测试用记忆条目
    fn make_entry(key: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            key: key.to_string(),
            content: content.to_string(),
            category: "test".to_string(),
            timestamp: Utc::now(),
            session_id: Some("session-1".to_string()),
            agent_alias: Some("test-agent".to_string()),
        }
    }

    /// 测试: store + recall (存储与全文检索)
    #[tokio::test]
    async fn test_store_and_recall() {
        let (_dir, mem) = make_test_memory();

        // 存储两条记忆
        let e1 = make_entry("rust", "Rust 是一门系统编程语言, 注重安全和性能");
        let e2 = make_entry("python", "Python 是一门动态脚本语言, 注重简洁和易用");
        mem.store(&e1).await.unwrap();
        mem.store(&e2).await.unwrap();

        // 检索 "Rust": 应能找到包含 Rust 的记忆
        let results = mem.recall("Rust", 10).await.unwrap();
        assert!(!results.is_empty(), "检索 Rust 结果不应为空");
        assert_eq!(results[0].key, "rust");
        assert!(results[0].content.contains("Rust"));

        // 检索 "Python": 应能找到包含 Python 的记忆
        let results = mem.recall("Python", 10).await.unwrap();
        assert!(!results.is_empty(), "检索 Python 结果不应为空");
        assert_eq!(results[0].key, "python");

        // 检索中文: "系统编程" 应匹配 rust 条目
        let results = mem.recall("系统编程", 10).await.unwrap();
        assert!(!results.is_empty(), "检索中文结果不应为空");
        assert_eq!(results[0].key, "rust");

        // limit 限制: "是一门" 同时匹配两条, 但只返回 1 条
        let results = mem.recall("是一门", 1).await.unwrap();
        assert_eq!(results.len(), 1, "limit=1 应只返回 1 条");
    }

    /// 测试: get (按 key 获取单条记忆)
    #[tokio::test]
    async fn test_get() {
        let (_dir, mem) = make_test_memory();

        let entry = make_entry("test-key", "这是一条测试记忆内容");
        mem.store(&entry).await.unwrap();

        // 按 key 获取 -- 应存在
        let got = mem.get("test-key").await.unwrap();
        assert!(got.is_some(), "get 应返回存在的记忆");
        let got = got.unwrap();
        assert_eq!(got.key, "test-key");
        assert_eq!(got.content, "这是一条测试记忆内容");
        assert_eq!(got.category, "test");
        assert_eq!(got.session_id.as_deref(), Some("session-1"));
        assert_eq!(got.agent_alias.as_deref(), Some("test-agent"));

        // 不存在的 key -- 应返回 None
        let missing = mem.get("nonexistent").await.unwrap();
        assert!(missing.is_none(), "不存在的 key 应返回 None");
    }

    /// 测试: list (列出全部记忆, 按时间倒序)
    #[tokio::test]
    async fn test_list() {
        let (_dir, mem) = make_test_memory();

        // 空库: list 返回空
        let empty = mem.list().await.unwrap();
        assert!(empty.is_empty(), "空库 list 应为空");

        // 存储多条记忆
        let e1 = make_entry("key-1", "内容一");
        let e2 = make_entry("key-2", "内容二");
        let e3 = make_entry("key-3", "内容三");
        mem.store(&e1).await.unwrap();
        mem.store(&e2).await.unwrap();
        mem.store(&e3).await.unwrap();

        // list 应返回全部 3 条
        let list = mem.list().await.unwrap();
        assert_eq!(list.len(), 3, "list 应返回 3 条记忆");
    }

    /// 测试: forget (删除记忆, 含 FTS 索引清理)
    #[tokio::test]
    async fn test_forget() {
        let (_dir, mem) = make_test_memory();

        let entry = make_entry("to-forget", "这条记忆将被删除 soon");
        mem.store(&entry).await.unwrap();

        // 确认存在
        assert!(mem.get("to-forget").await.unwrap().is_some());

        // 删除
        mem.forget("to-forget").await.unwrap();

        // 确认主表已删除
        assert!(
            mem.get("to-forget").await.unwrap().is_none(),
            "forget 后 get 应返回 None"
        );

        // 确认 FTS 索引也已被清理 (检索不应返回已删除的记忆)
        let results = mem.recall("被删除", 10).await.unwrap();
        assert!(results.is_empty(), "删除后 FTS 检索应为空");
    }
}
