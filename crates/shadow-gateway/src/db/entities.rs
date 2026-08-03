//! 数据库实体 -- 用户表
//!
//! users 表存储管理员和普通用户，用于 Gateway Web 管理面板认证。
//! SQLite 用 rusqlite，MySQL 用 mysql_async。两套实现共用相同的 User/UserRole 类型。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 用户角色枚举
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    Viewer,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Admin => "admin",
            UserRole::Viewer => "viewer",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "admin" => Some(UserRole::Admin),
            "viewer" => Some(UserRole::Viewer),
            _ => None,
        }
    }
}

/// 用户实体 (与数据库 users 表对应)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
