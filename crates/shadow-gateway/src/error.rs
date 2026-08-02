//! 统一错误处理

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub type GatewayResult<T> = Result<T, GatewayError>;

#[derive(Debug)]
pub enum GatewayError {
    /// 未认证
    Unauthorized(String),
    /// 权限不足
    Forbidden(String),
    /// 资源不存在
    NotFound(String),
    /// 参数错误
    BadRequest(String),
    /// 数据库错误
    DbError(String),
    /// 配置错误
    ConfigError(String),
    /// 内部错误
    Internal(String),
}

impl From<anyhow::Error> for GatewayError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            Self::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::DbError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            Self::ConfigError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            Self::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };

        let body = Json(json!({
            "error": self.error_code(),
            "message": message,
        }));

        (status, body).into_response()
    }
}

impl GatewayError {
    fn error_code(&self) -> &'static str {
        match self {
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::BadRequest(_) => "bad_request",
            Self::DbError(_) => "db_error",
            Self::ConfigError(_) => "config_error",
            Self::Internal(_) => "internal_error",
        }
    }
}
