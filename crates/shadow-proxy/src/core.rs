//! Proxy Core -- 协议无关的业务逻辑层
//!
//! 所有 transport (HTTP / stdio / embedded) 共用这一层.
//! TaskRouter 已经包含了全部业务逻辑, 这里只是封装一层便捷 API.

use std::sync::Arc;
use anyhow::Result;
use serde_json::{json, Value};

use crate::card::AgentCard;
use crate::router::TaskRouter;
use crate::task::TaskStatus;

/// Proxy 核心业务逻辑 -- 协议无关
#[derive(Clone)]
pub struct ProxyCore {
    pub router: Arc<TaskRouter>,
}

impl ProxyCore {
    pub fn new(router: Arc<TaskRouter>) -> Self {
        Self { router }
    }

    // ── Agent 注册/发现 ──

    pub fn register_agent(&self, card: AgentCard) -> Result<Value> {
        let name = card.name.clone();
        self.router.registry.register(card)?;
        Ok(json!({"registered": true, "name": name}))
    }

    pub fn list_agents(&self) -> Value {
        Value::Array(
            self.router
                .registry
                .list()
                .into_iter()
                .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
                .collect(),
        )
    }

    pub fn get_agent(&self, name: &str) -> Option<Value> {
        self.router
            .registry
            .get(name)
            .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
    }

    pub fn deregister_agent(&self, name: &str) -> bool {
        self.router.registry.deregister(name)
    }

    // ── 任务管理 ──

    pub async fn create_task(
        &self,
        from: &str,
        to: &str,
        prompt: &str,
        capability: Option<&str>,
    ) -> Result<Value> {
        let task = if let Some(cap) = capability {
            self.router
                .dispatch_by_capability(from, cap, prompt)
                .await?
        } else {
            self.router.dispatch(from, to, prompt).await?
        };
        Ok(serde_json::to_value(task)?)
    }

    pub async fn get_task(&self, task_id: &str) -> Option<Value> {
        self.router
            .get_task(task_id)
            .await
            .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
    }

    pub async fn list_tasks(&self, status: Option<TaskStatus>) -> Value {
        let tasks = self.router.list_tasks(status).await;
        Value::Array(
            tasks
                .into_iter()
                .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
                .collect(),
        )
    }

    pub async fn cancel_task(&self, task_id: &str) -> Option<Value> {
        self.router
            .cancel_task(task_id)
            .await
            .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
    }

    // ── 健康检查 ──

    pub fn health(&self) -> Value {
        json!({
            "status": "ok",
            "agents": self.router.registry.count(),
        })
    }

    // ── A2A 发现 ──

    pub fn catalog(&self) -> Value {
        let agents = self.router.registry.list();
        let interfaces: Vec<Value> = agents
            .iter()
            .map(|card| {
                json!({
                    "name": card.name,
                    "transport": match card.transport {
                        crate::card::TransportKind::Local => "local",
                        crate::card::TransportKind::Acp => "acp",
                        crate::card::TransportKind::A2a => "a2a",
                    },
                    "endpoint": card.endpoint,
                    "capabilities": card.capabilities,
                })
            })
            .collect();

        json!({
            "name": "shadow-proxy",
            "description": "Shadow agent discovery catalog",
            "agents": interfaces,
            "version": env!("CARGO_PKG_VERSION"),
        })
    }
}
