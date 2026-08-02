//! 状态路由: /api/status

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::AuthUser;
use crate::error::GatewayResult;
use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new().route("/api/status", get(status))
}

#[derive(Serialize)]
struct StatusResponse {
    version: String,
    config_path: String,
    data_dir: String,
    daemon_running: bool,
}

async fn status(
    State(state): State<GatewayState>,
    user: AuthUser,
) -> GatewayResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "config_path": state.config_path.to_string_lossy(),
        "data_dir": state.data_dir.to_string_lossy(),
        "daemon_running": state.daemon_running,
        "user": {
            "username": user.username,
            "role": user.role,
        }
    })))
}
