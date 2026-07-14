//! SQLite memory -- 写路径 (forget / purge / count / supersede)
//!
//! 所有方法都是 `impl SqliteMemory` 的 `pub(super)` inherent 方法,
//! 由 `mod.rs` 的 `impl Memory` trait 方法委托调用。
//! (store 路径在 [`store`], 这里只放 delete / count / supersede。)

use crate::sqlite::agent::sqlite_ensure_agent_uuid;
use crate::sqlite::SqliteMemory;
use rusqlite::params;

impl SqliteMemory {
    /// [`Memory::forget`](shadow_core::Memory::forget) 的实现。
    pub(super) async fn forget_inner(&self, key: &str) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = conn.lock();
            let affected = conn.execute("DELETE FROM memories WHERE key = ?1", params![key])?;
            Ok(affected > 0)
        })
        .await?
    }

    /// [`Memory::forget_for_agent`](shadow_core::Memory::forget_for_agent) 的实现。
    pub(super) async fn forget_for_agent_inner(
        &self,
        key: &str,
        agent_id: &str,
    ) -> anyhow::Result<bool> {
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

    /// [`Memory::purge_namespace`](shadow_core::Memory::purge_namespace) 的实现。
    pub(super) async fn purge_namespace_inner(&self, namespace: &str) -> anyhow::Result<usize> {
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

    /// [`Memory::purge_session`](shadow_core::Memory::purge_session) 的实现。
    pub(super) async fn purge_session_inner(&self, session_id: &str) -> anyhow::Result<usize> {
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

    /// [`Memory::purge_session_for_agent`](shadow_core::Memory::purge_session_for_agent) 的实现。
    pub(super) async fn purge_session_for_agent_inner(
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

    /// [`Memory::purge_agent`](shadow_core::Memory::purge_agent) 的实现。
    pub(super) async fn purge_agent_inner(&self, agent_alias: &str) -> anyhow::Result<usize> {
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

    /// [`Memory::rename_agent`](shadow_core::Memory::rename_agent) 的实现。
    pub(super) async fn rename_agent_inner(&self, from: &str, to: &str) -> anyhow::Result<usize> {
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

    /// [`Memory::count_agent`](shadow_core::Memory::count_agent) 的实现。
    pub(super) async fn count_agent_inner(&self, agent_alias: &str) -> anyhow::Result<usize> {
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

    /// [`Memory::count`](shadow_core::Memory::count) 的实现。
    pub(super) async fn count_inner(&self) -> anyhow::Result<usize> {
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

    /// [`Memory::supersede`](shadow_core::Memory::supersede) 的实现。
    pub(super) async fn supersede_inner(
        &self,
        superseded_ids: &[String],
        new_id: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        let ids = superseded_ids.to_vec();
        let new_id = new_id.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn.lock();
            crate::conflict::mark_superseded(&conn, &ids, &new_id)
        })
        .await?
    }

    /// [`Memory::ensure_agent_uuid`](shadow_core::Memory::ensure_agent_uuid) 的实现。
    pub(super) async fn ensure_agent_uuid_inner(&self, alias: &str) -> anyhow::Result<String> {
        let conn = self.conn.clone();
        let alias = alias.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let conn = conn.lock();
            sqlite_ensure_agent_uuid(&conn, &alias)
        })
        .await?
    }
}
