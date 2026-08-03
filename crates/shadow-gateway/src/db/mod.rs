//! 数据库管理模块
//!
//! 支持 SQLite (rusqlite) 和 MySQL (mysql_async)，由 Gateway setup 时选择。
//! 连接信息持久化到 config.toml 的 [storage.sqlite] 或 [storage.mysql] 段。

pub mod entities;
pub mod setup;

pub use entities::{User, UserRole};
pub use setup::{connect_from_config, connect_sqlite, connect_mysql, db_kind};

use anyhow::Result;
use async_trait::async_trait;

/// 统一数据库连接 trait (抽象 SQLite/MySQL 差异)
#[async_trait]
pub trait DbConn: Send + Sync {
    async fn is_initialized(&self) -> Result<bool>;
    async fn create_admin(&self, username: &str, password: &str) -> Result<User>;
    async fn find_user_by_username(&self, username: &str) -> Result<Option<User>>;
    async fn verify_password(&self, username: &str, password: &str) -> Result<Option<User>>;
    async fn list_users(&self) -> Result<Vec<User>>;
    async fn create_user(&self, username: &str, password: &str, role: UserRole) -> Result<User>;
    async fn delete_user(&self, id: i32) -> Result<()>;
}

/// 数据库连接类型别名
pub type DbConnBox = std::sync::Arc<dyn DbConn>;
