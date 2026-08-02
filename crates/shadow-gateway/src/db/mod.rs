//! 数据库管理模块（JSON 文件存储）
//!
//! 用户数据存储在 JSON 文件中，避免数据库依赖和 SQLite 链接冲突

mod entities;
mod setup;

pub use entities::*;
pub use setup::*;

/// 用户存储结构
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserStore {
    pub users: Vec<User>,
}

impl Default for UserStore {
    fn default() -> Self {
        Self { users: Vec::new() }
    }
}
