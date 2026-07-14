//! SQLite memory -- 建表 / 迁移 / 索引
//!
//! `init_schema` 创建 memories + agents + FTS5 虚拟表 + embedding_cache,
//! 然后用 ALTER TABLE 增量补列 (兼容旧库)。
//! `open_connection` 支持超时, 防止 NFS / 网络挂载下的无限等待。

use crate::sqlite::util::SQLITE_OPEN_TIMEOUT_CAP_SECS;
use anyhow::Context;
use rusqlite::{Connection, ErrorCode};
use std::path::Path;
use std::thread;
use std::time::Duration;

pub(super) fn open_connection(
    db_path: &Path,
    open_timeout_secs: Option<u64>,
) -> anyhow::Result<Connection> {
    let path_buf = db_path.to_path_buf();
    let conn = if let Some(secs) = open_timeout_secs {
        let capped = secs.min(SQLITE_OPEN_TIMEOUT_CAP_SECS);
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let result = Connection::open(&path_buf);
            let _ = tx.send(result);
        });

        match rx.recv_timeout(Duration::from_secs(capped)) {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => return Err(e).context("Sqlite failed to open database"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                anyhow::bail!(format!(
                    "Sqlite connection open timeout after {} seconds",
                    capped
                ))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("Sqlite open thread exit unexpectedly")
            }
        }
    } else {
        Connection::open(&path_buf).context("SQLite failed to open database")?
    };

    Ok(conn)
}

pub(super) fn init_schema(conn: &Connection) -> anyhow::Result<()> {
    execute_batch_retry(
        conn,
        "-- Agents table: maps agent alias → stable UUID, referenced as
        -- FK by memories.agent_id. Created here (not by an external
        -- migration) so the backend is self-contained and functional.
        CREATE TABLE IF NOT EXISTS agents (
            id         TEXT PRIMARY KEY,
            alias      TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL
        );

        -- Core memories table. Keys are unique per-agent, not globally:
        -- the store path uses ON CONFLICT(agent_id, key) for upserts.
        CREATE TABLE IF NOT EXISTS memories (
            id          TEXT PRIMARY KEY,
            key         TEXT NOT NULL,
            content     TEXT NOT NULL,
            category    TEXT NOT NULL DEFAULT 'core',
            embedding   BLOB,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL,
            agent_id    TEXT REFERENCES agents(id),
            UNIQUE (agent_id, key)
        );
        CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
        CREATE INDEX IF NOT EXISTS idx_memories_key ON memories(key);
        CREATE INDEX IF NOT EXISTS idx_memories_agent ON memories(agent_id);

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

    // Backfill agent_id on DBs created before it was in the base
    // CREATE TABLE. Fresh DBs already have it; old DBs get the column
    // added.
    add_memories_column_if_missing(
        conn,
        "agent_id",
        "ALTER TABLE memories ADD COLUMN agent_id TEXT REFERENCES agents(id);",
    )?;

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

    Ok(())
}

// ── 内部 helper ────────────────────────────────────────────────

fn is_db_locked_error(e: &rusqlite::Error) -> bool {
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
