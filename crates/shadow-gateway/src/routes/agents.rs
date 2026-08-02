//! Agent 配置路由: /api/agents/*

use axum::extract::State;
use axum::{Router, routing::{get, put}};
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::GatewayResult;
use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new()
        .route("/api/agents", get(list_agents))
        .route("/api/agents/{alias}", get(get_agent).put(update_agent))
}

/// 前端期望的 Agent 配置格式
#[derive(Serialize)]
struct AgentItem {
    alias: String,
    model_provider: String,
    enabled: bool,
    tts_provider: String,
}

async fn list_agents(
    State(state): State<GatewayState>,
    _user: AuthUser,
) -> GatewayResult<axum::Json<Vec<AgentItem>>> {
    let config = state.config.read().await;
    let agents: Vec<AgentItem> = config
        .agents
        .iter()
        .map(|(alias, cfg)| AgentItem {
            alias: alias.to_string(),
            model_provider: cfg.model_provider.to_string(),
            enabled: cfg.enabled,
            tts_provider: cfg.tts_provider.clone(),
        })
        .collect();
    Ok(axum::Json(agents))
}

async fn get_agent(
    State(state): State<GatewayState>,
    _user: AuthUser,
    axum::extract::Path(alias): axum::extract::Path<String>,
) -> GatewayResult<axum::Json<serde_json::Value>> {
    let config = state.config.read().await;
    let agent = config
        .agents
        .get(&alias)
        .ok_or_else(|| crate::error::GatewayError::NotFound("Agent not found".to_string()))?;
    Ok(axum::Json(serde_json::to_value(agent).unwrap_or_default()))
}

async fn update_agent(
    _user: AuthUser,
    _alias: axum::extract::Path<String>,
    _body: axum::Json<serde_json::Value>,
) -> GatewayResult<axum::Json<serde_json::Value>> {
    Err(crate::error::GatewayError::Forbidden(
        "Agent 更新暂不支持，请直接编辑 config.toml".to_string(),
    ))
}
