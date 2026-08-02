//! 日志路由: /api/logs

use axum::{Router, routing::get, extract::Query};
use serde::Deserialize;

use crate::auth::middleware::AuthUser;
use crate::error::GatewayResult;
use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new().route("/api/logs", get(list_logs))
}

#[derive(Deserialize)]
struct LogsQuery {
    level: Option<String>,
    limit: Option<usize>,
}

async fn list_logs(_user: AuthUser, _query: Query<LogsQuery>) -> GatewayResult<axum::Json<Vec<serde_json::Value>>> {
    Ok(axum::Json(vec![]))
}
