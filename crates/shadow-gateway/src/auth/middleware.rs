//! 认证中间件 -- axum extractor
//!
//! 通过实现 FromRequestParts，在 handler 参数中声明 AuthUser 即可要求登录。
//! AdminUser 额外要求 admin 角色。

use axum::extract::{FromRequestParts, FromRef};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

use crate::error::GatewayError;
use crate::state::GatewayState;
use crate::auth::Claims;

/// 认证用户信息（从 JWT 提取）
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: i32,
    pub username: String,
    pub role: String,
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    pub fn from_claims(claims: Claims) -> Self {
        Self {
            user_id: claims.sub,
            username: claims.username,
            role: claims.role,
        }
    }
}

/// 从请求头提取并验证 JWT
fn extract_user(parts: &Parts, state: &GatewayState) -> Result<AuthUser, GatewayError> {
    let auth_header = parts
        .headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| GatewayError::Unauthorized("Missing Authorization header".into()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| GatewayError::Unauthorized("Invalid auth header format".into()))?;

    let claims = crate::auth::verify_token(token, &state.jwt_secret)
        .map_err(|e| GatewayError::Unauthorized(format!("Invalid token: {e}")))?;

    Ok(AuthUser::from_claims(claims))
}

/// 认证 extractor: 要求登录
/// 用法: `async fn handler(user: AuthUser) -> ...`
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    GatewayState: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = GatewayState::from_ref(state);
        extract_user(parts, &state).map_err(|e| e.into_response())
    }
}

/// 管理员 extractor: 要求 admin 角色
/// 用法: `async fn handler(admin: AdminUser) -> ...`
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthUser);

impl<S> FromRequestParts<S> for AdminUser
where
    S: Send + Sync,
    GatewayState: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = GatewayState::from_ref(state);
        let user = extract_user(parts, &state).map_err(|e| e.into_response())?;

        if !user.is_admin() {
            return Err(GatewayError::Forbidden("Admin access required".into()).into_response());
        }

        Ok(AdminUser(user))
    }
}
