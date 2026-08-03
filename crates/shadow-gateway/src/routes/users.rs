//! 用户管理路由: /api/users (admin only)

use axum::{
    extract::{State, Path},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth::middleware::AdminUser;
use crate::db::{User, UserRole};
use crate::error::{GatewayError, GatewayResult};
use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new()
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/{id}", axum::routing::delete(delete_user))
}

#[derive(Serialize)]
struct UserInfo {
    id: i32,
    username: String,
    role: String,
}

impl From<User> for UserInfo {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            role: user.role.as_str().to_string(),
        }
    }
}

/// 列出所有用户 (admin only)
async fn list_users(
    _admin: AdminUser,
    State(state): State<GatewayState>,
) -> GatewayResult<Json<Vec<UserInfo>>> {
    let users = state.db.list_users()
        .await
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    let infos: Vec<UserInfo> = users.into_iter().map(UserInfo::from).collect();
    Ok(Json(infos))
}

/// 创建用户 (admin only)
#[derive(Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    role: String,
}

async fn create_user(
    _admin: AdminUser,
    State(state): State<GatewayState>,
    Json(payload): Json<CreateUserRequest>,
) -> GatewayResult<Json<UserInfo>> {
    let role = UserRole::from_str(&payload.role)
        .ok_or_else(|| GatewayError::BadRequest("无效的角色".to_string()))?;

    let user = state.db.create_user(&payload.username, &payload.password, role)
        .await
        .map_err(|e| GatewayError::BadRequest(e.to_string()))?;

    Ok(Json(UserInfo::from(user)))
}

/// 删除用户 (admin only)
async fn delete_user(
    _admin: AdminUser,
    State(state): State<GatewayState>,
    Path(id): Path<i32>,
) -> GatewayResult<StatusCode> {
    state.db.delete_user(id)
        .await
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}
