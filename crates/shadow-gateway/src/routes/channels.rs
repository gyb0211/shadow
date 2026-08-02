//! Channel 配置路由: /api/channels/*

use axum::extract::State;
use axum::{Router, routing::{get, put}};
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::GatewayResult;
use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new()
        .route("/api/channels", get(list_channels))
        .route("/api/channels/{type}/{alias}", put(update_channel))
}

#[derive(Serialize)]
struct ChannelItem {
    #[serde(rename = "type")]
    channel_type: String,
    alias: String,
    enabled: bool,
    config: serde_json::Value,
}

async fn list_channels(
    State(state): State<GatewayState>,
    _user: AuthUser,
) -> GatewayResult<axum::Json<Vec<ChannelItem>>> {
    let config = state.config.read().await;
    let mut items = Vec::new();

    // Lark channels
    for (alias, lark_cfg) in &config.channels.lark {
        items.push(ChannelItem {
            channel_type: "lark".to_string(),
            alias: alias.to_string(),
            enabled: lark_cfg.enabled,
            config: serde_json::to_value(lark_cfg).unwrap_or_default(),
        });
    }

    // CLI channel (布尔开关，不是 HashMap)
    if config.channels.cli {
        items.push(ChannelItem {
            channel_type: "cli".to_string(),
            alias: "default".to_string(),
            enabled: true,
            config: serde_json::json!({}),
        });
    }

    Ok(axum::Json(items))
}

async fn update_channel(
    _user: AuthUser,
    _path: axum::extract::Path<(String, String)>,
    _body: axum::Json<serde_json::Value>,
) -> GatewayResult<axum::Json<serde_json::Value>> {
    Err(crate::error::GatewayError::Forbidden(
        "Channel 更新暂不支持，请直接编辑 config.toml".to_string(),
    ))
}
