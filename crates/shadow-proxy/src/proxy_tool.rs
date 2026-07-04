//! ProxyTool -- 主 agent 通过此 tool 调用 proxy 的全部功能
//!
//! action:
//!   delegate    -- 派发任务到指定 agent
//!   by_capability -- 按能力查找 agent 并派发
//!   list_agents -- 列出所有已注册 agent
//!   get_agent   -- 查看指定 agent 详情
//!   check_task  -- 查询任务状态
//!   list_tasks  -- 列出任务
//!   cancel_task -- 取消任务

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use shadow_core::{Attributable, Role, Tool, ToolResult};

use crate::router::TaskRouter;
use crate::task::TaskStatus;

/// Proxy Tool -- 主 agent 通过此 tool 与 proxy 交互
pub struct ProxyTool {
    router: Arc<TaskRouter>,
}

impl ProxyTool {
    pub fn new(router: Arc<TaskRouter>) -> Self {
        Self { router }
    }
}

impl Attributable for ProxyTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        "proxy"
    }
}

#[async_trait]
impl Tool for ProxyTool {
    fn name(&self) -> &str {
        "delegate"
    }

    fn description(&self) -> &str {
        "委派任务到其他 agent (本地/ACP子进程/远程A2A). \
         action=delegate: 派发任务; \
         action=by_capability: 按能力查找并派发; \
         action=list_agents: 列出已注册 agent; \
         action=check_task: 查询任务状态; \
         action=cancel_task: 取消任务."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["delegate", "by_capability", "list_agents", "get_agent", "check_task", "list_tasks", "cancel_task"],
                    "default": "delegate"
                },
                "agent": {
                    "type": "string",
                    "description": "目标 agent 名称 (delegate 时必填)"
                },
                "capability": {
                    "type": "string",
                    "description": "能力标签 (by_capability 时必填, 如 'coding', 'review')"
                },
                "prompt": {
                    "type": "string",
                    "description": "任务描述 (delegate/by_capability 时必填)"
                },
                "task_id": {
                    "type": "string",
                    "description": "任务 ID (check_task/cancel_task 时必填)"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "running", "completed", "failed", "cancelled"],
                    "description": "状态过滤 (list_tasks 时可选)"
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("delegate");

        match action {
            "delegate" => self.do_delegate(&args).await,
            "by_capability" => self.do_by_capability(&args).await,
            "list_agents" => self.do_list_agents().await,
            "get_agent" => self.do_get_agent(&args).await,
            "check_task" => self.do_check_task(&args).await,
            "list_tasks" => self.do_list_tasks(&args).await,
            "cancel_task" => self.do_cancel_task(&args).await,
            other => Ok(ToolResult::err(format!("未知 action: {other}"))),
        }
    }
}

impl ProxyTool {
    async fn do_delegate(&self, args: &Value) -> Result<ToolResult> {
        let agent = match args.get("agent").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return Ok(ToolResult::err("缺少 agent 参数")),
        };
        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Ok(ToolResult::err("缺少 prompt 参数")),
        };

        let task = self.router.dispatch("main", agent, prompt).await?;
        Ok(task_to_result(&task))
    }

    async fn do_by_capability(&self, args: &Value) -> Result<ToolResult> {
        let capability = match args.get("capability").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return Ok(ToolResult::err("缺少 capability 参数")),
        };
        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Ok(ToolResult::err("缺少 prompt 参数")),
        };

        let task = self
            .router
            .dispatch_by_capability("main", capability, prompt)
            .await?;
        Ok(task_to_result(&task))
    }

    async fn do_list_agents(&self) -> Result<ToolResult> {
        let agents = self.router.registry.list();
        if agents.is_empty() {
            return Ok(ToolResult::ok("没有已注册的 agent"));
        }
        let lines: Vec<String> = agents
            .iter()
            .map(|card| {
                format!(
                    "- {} [{}] capabilities: [{}]",
                    card.name,
                    match card.transport {
                        crate::card::TransportKind::Local => "local",
                        crate::card::TransportKind::Acp => "acp",
                        crate::card::TransportKind::A2a => "a2a",
                    },
                    card.capabilities.join(", ")
                )
            })
            .collect();
        Ok(ToolResult::ok(lines.join("\n")))
    }

    async fn do_get_agent(&self, args: &Value) -> Result<ToolResult> {
        let name = match args.get("agent").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return Ok(ToolResult::err("缺少 agent 参数")),
        };

        match self.router.registry.get(name) {
            Some(card) => Ok(ToolResult::ok(serde_json::to_string_pretty(&card)?)),
            None => Ok(ToolResult::err(format!("agent '{name}' 不存在"))),
        }
    }

    async fn do_check_task(&self, args: &Value) -> Result<ToolResult> {
        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return Ok(ToolResult::err("缺少 task_id 参数")),
        };

        match self.router.get_task(task_id).await {
            Some(task) => Ok(ToolResult::ok(serde_json::to_string_pretty(&task)?)),
            None => Ok(ToolResult::err(format!("任务 '{task_id}' 不存在"))),
        }
    }

    async fn do_list_tasks(&self, args: &Value) -> Result<ToolResult> {
        let status_filter = args
            .get("status")
            .and_then(|v| v.as_str())
            .and_then(|s| match s {
                "pending" => Some(TaskStatus::Pending),
                "running" => Some(TaskStatus::Running),
                "completed" => Some(TaskStatus::Completed),
                "failed" => Some(TaskStatus::Failed),
                "cancelled" => Some(TaskStatus::Cancelled),
                _ => None,
            });

        let tasks = self.router.list_tasks(status_filter).await;
        if tasks.is_empty() {
            return Ok(ToolResult::ok("没有匹配的任务"));
        }
        Ok(ToolResult::ok(serde_json::to_string_pretty(&tasks)?))
    }

    async fn do_cancel_task(&self, args: &Value) -> Result<ToolResult> {
        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return Ok(ToolResult::err("缺少 task_id 参数")),
        };

        match self.router.cancel_task(task_id).await {
            Some(task) => Ok(ToolResult::ok(format!("任务已取消: {}", task.id))),
            None => Ok(ToolResult::err(format!("任务 '{task_id}' 不存在"))),
        }
    }
}

/// Task → ToolResult 转换
fn task_to_result(task: &crate::task::Task) -> ToolResult {
    match task.status {
        TaskStatus::Completed => {
            ToolResult::ok(task.result.clone().unwrap_or_else(|| "[空响应]".into()))
        }
        TaskStatus::Failed => ToolResult::err(task.error.clone().unwrap_or_else(|| "未知错误".into())),
        TaskStatus::Cancelled => ToolResult::err("任务已取消".to_string()),
        TaskStatus::Running => ToolResult::ok(format!("任务运行中: {}", task.id)),
        TaskStatus::Pending => ToolResult::ok(format!("任务排队中: {}", task.id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalAgent;
    use crate::AgentCard;
    use crate::AgentRegistry;
    use crate::transport::AgentTransport;
    use shadow_core::{Attributable, ChatResponse, ChatRequest, Provider, Role, TokenUsage};

    struct MockProvider;

    impl Attributable for MockProvider {
        fn role(&self) -> Role { Role::Agent }
        fn alias(&self) -> &str { "mock" }
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        fn provider_type(&self) -> &str { "mock" }
        async fn chat(&self, _: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: "done".into(),
                reasoning_content: None,
                tool_calls: vec![],
                usage: TokenUsage::default(),
            })
        }
        // chat_stream 使用默认实现
        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(vec!["mock-model".into()])
        }
    }

    #[tokio::test]
    async fn proxy_tool_delegate() {
        let registry = Arc::new(AgentRegistry::new());
        let router = Arc::new(TaskRouter::new(Arc::clone(&registry)));

        let provider: Arc<dyn Provider> = Arc::new(MockProvider);
        let agent = LocalAgent::new("worker", vec!["coding".into()], provider, "model".into());
        registry.register(agent.card().clone()).unwrap();
        router.register_transport("worker", Arc::new(agent));

        let tool = ProxyTool::new(Arc::clone(&router));
        let result = tool
            .execute(json!({
                "action": "delegate",
                "agent": "worker",
                "prompt": "hello"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "done");
    }

    #[tokio::test]
    async fn proxy_tool_list_agents() {
        let registry = Arc::new(AgentRegistry::new());
        let router = Arc::new(TaskRouter::new(Arc::clone(&registry)));

        registry
            .register(AgentCard::local("a", vec!["coding".into()]))
            .unwrap();
        registry
            .register(AgentCard::acp("b", "claude", vec![], vec!["review".into()]))
            .unwrap();

        let tool = ProxyTool::new(router);
        let result = tool
            .execute(json!({"action": "list_agents"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("a"));
        assert!(result.output.contains("b"));
    }

    #[tokio::test]
    async fn proxy_tool_delegate_missing_agent() {
        let registry = Arc::new(AgentRegistry::new());
        let router = Arc::new(TaskRouter::new(registry));

        let tool = ProxyTool::new(router);
        let result = tool
            .execute(json!({"action": "delegate", "agent": "missing", "prompt": "hi"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("未注册"));
    }

    #[tokio::test]
    async fn proxy_tool_by_capability() {
        let registry = Arc::new(AgentRegistry::new());
        let router = Arc::new(TaskRouter::new(Arc::clone(&registry)));

        let provider: Arc<dyn Provider> = Arc::new(MockProvider);
        let agent = LocalAgent::new("coder", vec!["coding".into()], provider, "model".into());
        registry.register(agent.card().clone()).unwrap();
        router.register_transport("coder", Arc::new(agent));

        let tool = ProxyTool::new(router);
        let result = tool
            .execute(json!({"action": "by_capability", "capability": "coding", "prompt": "write code"}))
            .await
            .unwrap();
        assert!(result.success);
    }
}
