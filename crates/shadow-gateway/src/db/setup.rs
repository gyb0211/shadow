//! 数据库初始化与用户 CRUD
//!
//! 双后端实现:
//!   - SQLite: rusqlite (同步, 用 tokio::task::spawn_blocking 包装)
//!   - MySQL:  mysql_async (纯 Rust 异步, 无原生库依赖)
//!
//! 由 Gateway setup 时选择，连接信息持久化到 config.toml。

use anyhow::{Context, Result};
use async_trait::async_trait;
use bcrypt::{hash, verify, DEFAULT_COST};
use chrono::{NaiveDateTime, TimeZone, Utc};
use mysql_async::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::entities::{User, UserRole};
use super::DbConn;

// ── 从配置连接 ────────────────────────────────────────

/// 从已有配置加载连接 (Gateway 启动时调用)
pub async fn connect_from_config(
    config: &shadow_config::Config,
) -> Result<Arc<dyn DbConn>> {
    // 尝试 MySQL
    if let Some(mysql_cfg) = config.storage.mysql.get("default") {
        let conn = MysqlConn::connect(
            &mysql_cfg.host,
            mysql_cfg.port.unwrap_or(3306),
            &mysql_cfg.user,
            &mysql_cfg.password,
            &mysql_cfg.database,
        )
        .await?;
        return Ok(Arc::new(conn));
    }

    // 尝试 SQLite (配置中指定)
    if let Some(sqlite_cfg) = config.storage.sqlite.get("default") {
        let path = sqlite_cfg.path.as_deref().unwrap_or("gateway.db");
        let full = expand_db_path(&config.data_dir, path);
        let conn = SqliteConn::connect(&full)?;
        return Ok(Arc::new(conn));
    }

    // 默认: SQLite ~ data_dir/gateway.db
    let full = config.data_dir.join("gateway.db");
    let conn = SqliteConn::connect(&full)?;
    Ok(Arc::new(conn))
}

/// Setup 时创建 SQLite 连接
pub async fn connect_sqlite(data_dir: &PathBuf, db_path: &str) -> Result<Arc<dyn DbConn>> {
    let full = expand_db_path(data_dir, db_path);
    let conn = SqliteConn::connect(&full)?;
    Ok(Arc::new(conn))
}

/// Setup 时创建 MySQL 连接
pub async fn connect_mysql(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
) -> Result<Arc<dyn DbConn>> {
    let conn = MysqlConn::connect(host, port, user, password, database).await?;
    Ok(Arc::new(conn))
}

// ── SQLite 实现 (rusqlite) ───────────────────────────

struct SqliteConn {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteConn {
    fn connect(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = rusqlite::Connection::open(path)
            .context(format!("打开 SQLite 失败: {}", path.display()))?;
        Self::ensure_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn ensure_schema(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'viewer',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )?;
        Ok(())
    }
}

#[async_trait]
impl DbConn for SqliteConn {
    async fn is_initialized(&self) -> Result<bool> {
        let conn = self.conn.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lock = conn.blocking_lock();
            let count: i64 = lock.query_row(
                "SELECT COUNT(*) FROM users WHERE role = 'admin'",
                [],
                |row| row.get(0),
            )?;
            Ok::<bool, anyhow::Error>(count > 0)
        })
        .await??;
        Ok(result)
    }

    async fn create_admin(&self, username: &str, password: &str) -> Result<User> {
        let password_hash = hash(password, DEFAULT_COST)?;
        let now = Utc::now();
        let conn = self.conn.clone();
        let username = username.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let lock = conn.blocking_lock();
            lock.execute(
                "INSERT INTO users (username, password_hash, role, created_at, updated_at) VALUES (?1, ?2, 'admin', ?3, ?4)",
                rusqlite::params![username, password_hash, now, now],
            )?;
            let id = lock.last_insert_rowid() as i32;
            Ok::<User, anyhow::Error>(User {
                id,
                username,
                password_hash,
                role: UserRole::Admin,
                created_at: now,
                updated_at: now,
            })
        })
        .await??;
        Ok(result)
    }

    async fn find_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let conn = self.conn.clone();
        let username = username.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let lock = conn.blocking_lock();
            let mut stmt = lock.prepare(
                "SELECT id, username, password_hash, role, created_at, updated_at FROM users WHERE username = ?1",
            )?;
            let mut rows = stmt.query(rusqlite::params![username])?;
            if let Some(row) = rows.next()? {
                let role_str: String = row.get::<_, String>(3)?;
                Ok::<Option<User>, anyhow::Error>(Some(User {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    password_hash: row.get(2)?,
                    role: UserRole::from_str(&role_str).unwrap_or(UserRole::Viewer),
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await??;
        Ok(result)
    }

    async fn verify_password(&self, username: &str, password: &str) -> Result<Option<User>> {
        let user = self.find_user_by_username(username).await?;
        if let Some(user) = user {
            if verify(password, &user.password_hash).unwrap_or(false) {
                return Ok(Some(user));
            }
        }
        Ok(None)
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let conn = self.conn.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lock = conn.blocking_lock();
            let mut stmt = lock.prepare(
                "SELECT id, username, password_hash, role, created_at, updated_at FROM users ORDER BY id",
            )?;
            let users = stmt
                .query_map([], |row| {
                    let role_str: String = row.get(3)?;
                    Ok(User {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        password_hash: row.get(2)?,
                        role: UserRole::from_str(&role_str).unwrap_or(UserRole::Viewer),
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok::<Vec<User>, anyhow::Error>(users)
        })
        .await??;
        Ok(result)
    }

    async fn create_user(&self, username: &str, password: &str, role: UserRole) -> Result<User> {
        let password_hash = hash(password, DEFAULT_COST)?;
        let now = Utc::now();
        let role_str = role.as_str().to_string();
        let conn = self.conn.clone();
        let username = username.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let lock = conn.blocking_lock();
            lock.execute(
                "INSERT INTO users (username, password_hash, role, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![username, password_hash, role_str, now, now],
            )?;
            let id = lock.last_insert_rowid() as i32;
            Ok::<User, anyhow::Error>(User {
                id,
                username,
                password_hash,
                role,
                created_at: now,
                updated_at: now,
            })
        })
        .await??;
        Ok(result)
    }

    async fn delete_user(&self, id: i32) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let lock = conn.blocking_lock();
            lock.execute("DELETE FROM users WHERE id = ?1", rusqlite::params![id])?;
            Ok::<(), anyhow::Error>(())
        })
        .await??;
        Ok(())
    }
}

// ── MySQL 实现 (mysql_async) ─────────────────────────

struct MysqlConn {
    pool: Arc<Mutex<mysql_async::Pool>>,
}

impl MysqlConn {
    async fn connect(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
        database: &str,
    ) -> Result<Self> {
        let url = format!("mysql://{user}:***@{host}:{port}/{database}");
        let opts = mysql_async::Opts::from_url(&url)
            .context("MySQL URL 解析失败")?;
        let pool = mysql_async::Pool::new(opts);
        let pool = Arc::new(Mutex::new(pool));
        let conn = Self { pool };
        conn.ensure_schema().await?;
        Ok(conn)
    }

    async fn ensure_schema(&self) -> Result<()> {
        let pool = self.pool.clone();
        let mut pool_lock = pool.lock().await;
        let mut conn = pool_lock.get_conn().await?;
        conn.exec_drop(
            "CREATE TABLE IF NOT EXISTS users (
                id INT AUTO_INCREMENT PRIMARY KEY,
                username VARCHAR(64) NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                role VARCHAR(16) NOT NULL DEFAULT 'viewer',
                created_at DATETIME NOT NULL,
                updated_at DATETIME NOT NULL
            )",
            (),
        )
        .await?;
        drop(conn);
        Ok(())
    }
}

#[async_trait]
impl DbConn for MysqlConn {
    async fn is_initialized(&self) -> Result<bool> {
        let pool = self.pool.clone();
        let mut pool_lock = pool.lock().await;
        let mut conn = pool_lock.get_conn().await?;
        let row: Option<(i64,)> = conn
            .exec_first("SELECT COUNT(*) FROM users WHERE role = 'admin'", ())
            .await?;
        drop(conn);
        Ok(row.map(|(c,)| c > 0).unwrap_or(false))
    }

    async fn create_admin(&self, username: &str, password: &str) -> Result<User> {
        let password_hash = hash(password, DEFAULT_COST)?;
        let now = Utc::now();
        let now_naive = now.naive_utc();
        let pool = self.pool.clone();
        let mut pool_lock = pool.lock().await;
        let mut conn = pool_lock.get_conn().await?;
        conn.exec_drop(
            "INSERT INTO users (username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'admin', ?, ?)",
            (username, &password_hash, now_naive, now_naive),
        )
        .await?;
        let id = conn.last_insert_id().unwrap_or(0) as i32;
        drop(conn);
        Ok(User {
            id,
            username: username.to_string(),
            password_hash,
            role: UserRole::Admin,
            created_at: now,
            updated_at: now,
        })
    }

    async fn find_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let pool = self.pool.clone();
        let mut pool_lock = pool.lock().await;
        let mut conn = pool_lock.get_conn().await?;
        let row: Option<(i32, String, String, String, NaiveDateTime, NaiveDateTime)> = conn
            .exec_first(
                "SELECT id, username, password_hash, role, created_at, updated_at FROM users WHERE username = ?",
                (username,),
            )
            .await?;
        drop(conn);
        Ok(row.map(|(id, username, password_hash, role, created_at, updated_at)| User {
            id,
            username,
            password_hash,
            role: UserRole::from_str(&role).unwrap_or(UserRole::Viewer),
            created_at: Utc.from_utc_datetime(&created_at),
            updated_at: Utc.from_utc_datetime(&updated_at),
        }))
    }

    async fn verify_password(&self, username: &str, password: &str) -> Result<Option<User>> {
        let user = self.find_user_by_username(username).await?;
        if let Some(user) = user {
            if verify(password, &user.password_hash).unwrap_or(false) {
                return Ok(Some(user));
            }
        }
        Ok(None)
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let pool = self.pool.clone();
        let mut pool_lock = pool.lock().await;
        let mut conn = pool_lock.get_conn().await?;
        let rows: Vec<(i32, String, String, String, NaiveDateTime, NaiveDateTime)> = conn
            .exec(
                "SELECT id, username, password_hash, role, created_at, updated_at FROM users ORDER BY id",
                (),
            )
            .await?;
        drop(conn);
        Ok(rows
            .into_iter()
            .map(|(id, username, password_hash, role, created_at, updated_at)| User {
                id,
                username,
                password_hash,
                role: UserRole::from_str(&role).unwrap_or(UserRole::Viewer),
                created_at: Utc.from_utc_datetime(&created_at),
                updated_at: Utc.from_utc_datetime(&updated_at),
            })
            .collect())
    }

    async fn create_user(&self, username: &str, password: &str, role: UserRole) -> Result<User> {
        let password_hash = hash(password, DEFAULT_COST)?;
        let now = Utc::now();
        let now_naive = now.naive_utc();
        let role_str = role.as_str().to_string();
        let pool = self.pool.clone();
        let mut pool_lock = pool.lock().await;
        let mut conn = pool_lock.get_conn().await?;
        conn.exec_drop(
            "INSERT INTO users (username, password_hash, role, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
            (username, &password_hash, &role_str, now_naive, now_naive),
        )
        .await?;
        let id = conn.last_insert_id().unwrap_or(0) as i32;
        drop(conn);
        Ok(User {
            id,
            username: username.to_string(),
            password_hash,
            role,
            created_at: now,
            updated_at: now,
        })
    }

    async fn delete_user(&self, id: i32) -> Result<()> {
        let pool = self.pool.clone();
        let mut pool_lock = pool.lock().await;
        let mut conn = pool_lock.get_conn().await?;
        conn.exec_drop("DELETE FROM users WHERE id = ?", (id,)).await?;
        drop(conn);
        Ok(())
    }
}

// ── 辅助 ─────────────────────────────────────────────

fn expand_db_path(data_dir: &PathBuf, path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        data_dir.join(path)
    }
}

/// 返回当前配置使用的数据库类型描述 (用于日志)
pub fn db_kind(config: &shadow_config::Config) -> String {
    if let Some(_mysql) = config.storage.mysql.get("default") {
        "MySQL".to_string()
    } else if let Some(sqlite) = config.storage.sqlite.get("default") {
        let path = sqlite.path.as_deref().unwrap_or("gateway.db");
        format!("SQLite ({path})")
    } else {
        "SQLite (默认 gateway.db)".to_string()
    }
}
