//! 认证路由: /api/auth/*

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth::jwt::create_token;
use crate::auth::middleware::AuthUser;
use crate::db;
use crate::error::{GatewayError, GatewayResult};
use crate::state::GatewayState;

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
    let initialized = db::is_initialized(&state.data_dir)
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    Ok(Json(SetupStatusResponse { initialized }))
}

/// 首次设置请求
/// 前端会发送 database 字段，后端使用 JSON 文件存储所以忽略它
#[derive(Deserialize)]
struct SetupRequest {
    #[serde(default)]
    database: serde_json::Value,
    admin: AdminSetup,
}

#[derive(Deserialize)]
struct AdminSetup {
    username: String,
    password: String,
}

/// 执行首次设置
async fn do_setup(
    State(state): State<GatewayState>,
    Json(payload): Json<SetupRequest>,
) -> GatewayResult<Json<LoginResponse>> {
    // 检查是否已初始化
    let already_init = db::is_initialized(&state.data_dir)
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    if already_init {
        return Err(GatewayError::BadRequest("系统已初始化".to_string()));
    }

    // 创建管理员
    let user = db::create_admin(&state.data_dir, &payload.admin.username, &payload.admin.password)
        .map_err(|e| GatewayError::BadRequest(e.to_string()))?;

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
    let user = db::verify_password(&state.data_dir, &payload.username, &payload.password)
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
