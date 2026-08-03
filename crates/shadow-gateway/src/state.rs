//! Gateway 共享状态

use std::sync::Arc;
use tokio::sync::RwLock;

use crate::db::DbConn;

/// Gateway 共享状态，注入到所有 axum handler
#[derive(Clone)]
pub struct GatewayState {
    /// JWT 密钥
    pub jwt_secret: String,
    /// Shadow 配置（读写锁，支持热更新）
    pub config: Arc<RwLock<shadow_config::Config>>,
    /// 配置文件路径
    pub config_path: std::path::PathBuf,
    /// 数据目录
    pub data_dir: std::path::PathBuf,
    /// Daemon 是否运行中
    pub daemon_running: bool,
    /// 数据库连接 (SQLite/MySQL 双后端)
    pub db: Arc<dyn DbConn>,
}

// axum 0.8 已有 blanket impl: FromRef<T> for T，无需手动实现
