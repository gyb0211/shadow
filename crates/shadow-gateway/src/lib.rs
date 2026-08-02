//! Shadow Gateway -- Web 管理 API 服务器
//!
//! 架构:
//! - axum HTTP 服务器 (端口 7975)
//! - sea-orm 数据库 (SQLite/MySQL)
//! - JWT 认证 + 角色权限 (admin/viewer)
//! - 读写 config.toml 管理 Shadow 配置
//! - rust-embed 嵌入前端静态文件
//!
//! 启动: shadow gateway --port 7975

pub mod auth;
pub mod db;
pub mod error;
pub mod routes;
pub mod server;
pub mod state;

pub use server::run_gateway;