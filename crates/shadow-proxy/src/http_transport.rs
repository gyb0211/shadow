//! HTTP 传输 -- 基于 axum 的 RESTful API
//!
//! 把 ProxyCore 的方法暴露为 HTTP endpoint:
//!   POST /agents/register    -- 注册 agent
//!   GET  /agents             -- 列出所有 agent
//!   GET  /agents/{name}      -- 查看指定 agent
//!   DELETE /agents/{name}    -- 注销 agent
//!   POST /tasks              -- 派发任务
//!   GET  /tasks/{id}         -- 查询任务状态
//!   GET  /tasks              -- 列出任务
//!   POST /tasks/{id}/cancel  -- 取消任务
//!   GET  /health             -- 健康检查
//!   GET  /.well-known/agent-card.json -- A2A 发现

use axum::{
    Router,
    Json,
    extract::{State, Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;

use crate::card::AgentCard;
use crate::core::ProxyCore;
use crate::task::TaskStatus;

/// HTTP server
pub struct HttpTransport {
    core: ProxyCore,
    bind: String,
    port: u16,
}

impl HttpTransport {
    pub fn new(core: ProxyCore, bind: &str, port: u16) -> Self {
        Self { core, bind: bind.to_string(), port }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/agents/register", post(register_agent))
            .route("/agents", get(list_agents))
            .route("/agents/{name}", get(get_agent).delete(deregister_agent))
            .route("/tasks", post(create_task).get(list_tasks))
            .route("/tasks/{id}", get(get_task))
            .route("/tasks/{id}/cancel", post(cancel_task))
            .route("/health", get(health_check))
            .route("/.well-known/agent-card.json", get(catalog_card))
            .with_state(self.core.clone())
    }

    pub async fn serve(&self) -> anyhow::Result<()> {
        let addr = format!("{}:{}", self.bind, self.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        shadow_log::record!(
            INFO,
            shadow_log::Action::Start,
            format!("Proxy HTTP server 监听 http://{addr}")
        );
        axum::serve(listener, self.router()).await?;
        Ok(())
    }
}

// ── Agent 注册/发现 ──────────────────────────────────────

async fn register_agent(
    State(core): State<ProxyCore>,
    Json(card): Json<AgentCard>,
) -> impl IntoResponse {
    match core.register_agent(card.clone()) {
        Ok(v) => (StatusCode::OK, Json(v)),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))),
    }
}

async fn list_agents(State(core): State<ProxyCore>) -> impl IntoResponse {
    Json(core.list_agents())
}

async fn get_agent(
    State(core): State<ProxyCore>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match core.get_agent(&name) {
        Some(v) => (StatusCode::OK, Json(v)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": format!("agent '{name}' 不存在")}))).into_response(),
    }
}

async fn deregister_agent(
    State(core): State<ProxyCore>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if core.deregister_agent(&name) {
        (StatusCode::OK, Json(json!({"deregistered": true, "name": name})))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": format!("agent '{name}' 不存在")})))
    }
}

// ── 任务管理 ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateTaskRequest {
    to: String,
    prompt: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    capability: Option<String>,
}

async fn create_task(
    State(core): State<ProxyCore>,
    Json(req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    let from = req.from.unwrap_or_else(|| "api".to_string());
    match core.create_task(&from, &req.to, &req.prompt, req.capability.as_deref()).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn get_task(
    State(core): State<ProxyCore>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match core.get_task(&id).await {
        Some(v) => (StatusCode::OK, Json(v)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": format!("任务 '{id}' 不存在")}))).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct TaskQuery {
    status: Option<String>,
}

async fn list_tasks(
    State(core): State<ProxyCore>,
    Query(q): Query<TaskQuery>,
) -> impl IntoResponse {
    let status_filter = q.status.as_deref().and_then(|s| match s {
        "pending" => Some(TaskStatus::Pending),
        "running" => Some(TaskStatus::Running),
        "completed" => Some(TaskStatus::Completed),
        "failed" => Some(TaskStatus::Failed),
        "cancelled" => Some(TaskStatus::Cancelled),
        _ => None,
    });
    Json(core.list_tasks(status_filter).await)
}

async fn cancel_task(
    State(core): State<ProxyCore>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match core.cancel_task(&id).await {
        Some(v) => (StatusCode::OK, Json(v)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": format!("任务 '{id}' 不存在")}))).into_response(),
    }
}

// ── 健康检查 + A2A 发现 ──────────────────────────────────

async fn health_check(State(core): State<ProxyCore>) -> impl IntoResponse {
    Json(core.health())
}

async fn catalog_card(State(core): State<ProxyCore>) -> impl IntoResponse {
    Json(core.catalog())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use serde_json::Value;
    use crate::{AgentRegistry, TaskRouter, AgentCard};

    #[tokio::test]
    async fn health_check_returns_agent_count() {
        let registry = Arc::new(AgentRegistry::new());
        let router = Arc::new(TaskRouter::new(Arc::clone(&registry)));
        registry.register(AgentCard::local("a", vec![])).unwrap();
        let core = ProxyCore::new(router);
        let server = HttpTransport::new(core, "127.0.0.1", 0);
        let app = server.router();

        use tower::ServiceExt;
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["agents"], 1);
    }
}
