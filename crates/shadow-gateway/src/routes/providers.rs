//! Provider 配置路由: /api/providers/*

use axum::extract::State;
use axum::{Router, routing::{get, put}};
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::GatewayResult;
use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new()
        .route("/api/providers", get(list_providers))
        .route("/api/providers/{type}/{alias}", put(update_provider))
}

#[derive(Serialize)]
struct ProviderItem {
    #[serde(rename = "type")]
    provider_type: String,
    alias: String,
    enabled: bool,
    model: String,
    config: serde_json::Value,
}

async fn list_providers(
    State(state): State<GatewayState>,
    _user: AuthUser,
) -> GatewayResult<axum::Json<Vec<ProviderItem>>> {
    let config = state.config.read().await;
    let mut items = Vec::new();

    // 遍历所有 model provider (custom, openai, anthropic 等)
    for (family, alias, cfg) in config.providers.models.iter_entries() {
        items.push(ProviderItem {
            provider_type: family.to_string(),
            alias: alias.to_string(),
            enabled: true,
            model: cfg.model.clone().unwrap_or_default(),
            config: serde_json::to_value(cfg).unwrap_or_default(),
        });
    }

    Ok(axum::Json(items))
}

async fn update_provider(
    _user: AuthUser,
    _path: axum::extract::Path<(String, String)>,
    _body: axum::Json<serde_json::Value>,
) -> GatewayResult<axum::Json<serde_json::Value>> {
    Err(crate::error::GatewayError::Forbidden(
        "Provider 更新暂不支持，请直接编辑 config.toml".to_string(),
    ))
}
