//! 配置管理路由: /api/config

use axum::extract::State;
use axum::{Router, routing::{get, put}};

use crate::auth::middleware::AuthUser;
use crate::error::GatewayResult;
use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new()
        .route("/api/config", get(get_config).put(update_config))
}

/// 获取完整配置 (序列化为 JSON，敏感字段脱敏)
async fn get_config(
    State(state): State<GatewayState>,
    _user: AuthUser,
) -> GatewayResult<axum::Json<serde_json::Value>> {
    let config = state.config.read().await;
    // 序列化配置为 JSON (serde 会自动跳过 #[serde(skip)] 的字段如 config_path/data_dir)
    let mut value = serde_json::to_value(&*config).unwrap_or_default();
    // 脱敏: 隐藏 api_key
    if let Some(obj) = value.as_object_mut() {
        mask_sensitive_fields(obj);
    }
    Ok(axum::Json(value))
}

/// 递归脱敏敏感字段
fn mask_sensitive_fields(value: &mut serde_json::Map<String, serde_json::Value>) {
    for (key, val) in value.iter_mut() {
        if key.contains("api_key") || key.contains("app_secret") || key.contains("password") {
            if let Some(s) = val.as_str() {
                if !s.is_empty() {
                    *val = serde_json::Value::String(format!("{}***", &s[..s.len().min(3)]));
                }
            }
        }
        // 递归处理嵌套对象
        if let Some(obj) = val.as_object_mut() {
            mask_sensitive_fields(obj);
        }
    }
}

async fn update_config(
    _user: AuthUser,
    _body: axum::Json<serde_json::Value>,
) -> GatewayResult<axum::Json<serde_json::Value>> {
    Err(crate::error::GatewayError::Forbidden(
        "配置更新暂不支持，请直接编辑 ~/.shadow/config.toml".to_string(),
    ))
}
