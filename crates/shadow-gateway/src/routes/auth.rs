//! 认证路由: /api/auth/*

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth::jwt::create_token;
use crate::auth::middleware::AuthUser;
use crate::db::DbConn;
use crate::error::{GatewayError, GatewayResult};
use crate::state::GatewayState;
use shadow_config::{MysqlStorageConfig, SqliteStorageConfig};

pub fn routes() -> Router<GatewayState> {
    Router::new()
        .route("/api/auth/setup/status", get(setup_status))
        .route("/api/auth/setup", post(do_setup))
        .route("/api/auth/login", post(login))
        .route("/api/auth/me", get(me))
}

/// 检查初始化状态
#[derive(Serialize)]
struct SetupStatusResponse {
    initialized: bool,
}

async fn setup_status(State(state): State<GatewayState>) -> GatewayResult<Json<SetupStatusResponse>> {
    let initialized = state.db.is_initialized()
        .await
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    Ok(Json(SetupStatusResponse { initialized }))
}

/// 数据库配置 (前端传入)
#[derive(Deserialize)]
#[serde(tag = "type")]
enum DatabaseSetup {
    #[serde(rename = "sqlite")]
    Sqlite {
        #[serde(default)]
        path: Option<String>,
    },
    #[serde(rename = "mysql")]
    Mysql {
        host: String,
        port: Option<u16>,
        user: String,
        password: String,
        database: String,
    },
}

/// 首次设置请求
#[derive(Deserialize)]
struct SetupRequest {
    database: DatabaseSetup,
    admin: AdminSetup,
}

#[derive(Deserialize)]
struct AdminSetup {
    username: String,
    password: String,
}

/// 执行首次设置
///
/// 1. 根据前端传入的 database 配置连接数据库
/// 2. 建 users 表
/// 3. 创建管理员账号
/// 4. 将数据库配置写入 config.toml 持久化
async fn do_setup(
    State(state): State<GatewayState>,
    Json(payload): Json<SetupRequest>,
) -> GatewayResult<Json<LoginResponse>> {
    // 检查是否已初始化
    let already_init = state.db.is_initialized()
        .await
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    if already_init {
        return Err(GatewayError::BadRequest("系统已初始化".to_string()));
    }

    // 根据前端配置连接新数据库
    let db_conn: std::sync::Arc<dyn DbConn> = match &payload.database {
        DatabaseSetup::Sqlite { path } => {
            let path = path.as_deref().unwrap_or("gateway.db");
            crate::db::connect_sqlite(&state.data_dir, path)
                .await
                .map_err(|e| GatewayError::DbError(format!("SQLite 连接失败: {e}")))?
        }
        DatabaseSetup::Mysql {
            host,
            port,
            user,
            password,
            database,
        } => {
            crate::db::connect_mysql(host, port.unwrap_or(3306), user, password, database)
                .await
                .map_err(|e| GatewayError::DbError(format!("MySQL 连接失败: {e}")))?
        }
    };

    // 创建管理员
    let user = db_conn.create_admin(&payload.admin.username, &payload.admin.password)
        .await
        .map_err(|e| GatewayError::BadRequest(e.to_string()))?;

    // 将数据库配置写入 config.toml
    {
        let mut config = state.config.write().await;
        match &payload.database {
            DatabaseSetup::Sqlite { path } => {
                config.storage.sqlite.insert(
                    "default".to_string(),
                    SqliteStorageConfig {
                        path: path.clone(),
                        open_timeout_secs: None,
                    },
                );
            }
            DatabaseSetup::Mysql {
                host,
                port,
                user,
                password,
                database,
            } => {
                config.storage.mysql.insert(
                    "default".to_string(),
                    MysqlStorageConfig {
                        host: host.clone(),
                        port: *port,
                        user: user.clone(),
                        password: password.clone(),
                        database: database.clone(),
                    },
                );
            }
        }
        // 保存配置到 config.toml
        if let Err(e) = config.save().await {
            tracing::warn!("配置保存失败 (不影响本次初始化): {e}");
        }
    }

    let token = create_token(user.id, &user.username, user.role.as_str(), &state.jwt_secret)
        .map_err(|e| GatewayError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        user: UserInfo {
            id: user.id,
            username: user.username,
            role: user.role.as_str().to_string(),
        },
    }))
}

/// 登录请求
#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

/// 登录响应
#[derive(Serialize)]
struct LoginResponse {
    token: String,
    user: UserInfo,
}

#[derive(Serialize)]
struct UserInfo {
    id: i32,
    username: String,
    role: String,
}

/// 登录
async fn login(
    State(state): State<GatewayState>,
    Json(payload): Json<LoginRequest>,
) -> GatewayResult<Json<LoginResponse>> {
    let user = state.db.verify_password(&payload.username, &payload.password)
        .await
        .map_err(|e| GatewayError::Internal(e.to_string()))?
        .ok_or_else(|| GatewayError::Unauthorized("用户名或密码错误".to_string()))?;

    let token = create_token(user.id, &user.username, user.role.as_str(), &state.jwt_secret)
        .map_err(|e| GatewayError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        user: UserInfo {
            id: user.id,
            username: user.username,
            role: user.role.as_str().to_string(),
        },
    }))
}

/// 获取当前用户
async fn me(user: AuthUser) -> GatewayResult<Json<UserInfo>> {
    Ok(Json(UserInfo {
        id: user.user_id,
        username: user.username,
        role: user.role,
    }))
}
