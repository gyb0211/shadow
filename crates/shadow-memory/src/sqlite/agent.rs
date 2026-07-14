//! SQLite memory -- agent 归因函数
//!
//! Mint-or-query a single agent row keyed by alias. Used by the
//! SQLite migration's default-agent backfill and by the `ensure_agent_uuid`
//! trait impl on the memory backend (alias resolution at agent-loop entry).

use rusqlite::{params, Connection};

pub fn sqlite_ensure_default_agent_uuid(conn: &Connection) -> anyhow::Result<String> {
    sqlite_ensure_agent_uuid(conn, "default")
}

pub fn sqlite_ensure_agent_uuid(conn: &Connection, alias: &str) -> anyhow::Result<String> {
    let new_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO agents (id, alias, created_at) VALUES (?1, ?2, ?3)",
        params![new_id, alias, now],
    )?;
    let final_id: String = conn.query_row(
        "SELECT id FROM agents WHERE alias = ?1 LIMIT 1",
        params![alias],
        |row| row.get(0),
    )?;
    Ok(final_id)
}
