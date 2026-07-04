//! 子代理委派工具 -- 将任务委派给子代理执行
//!
//! 通过 shadow-proxy 的 LocalAgent 或直接 spawn 子进程,
//! 将任务委派给另一个 agent 执行, 实现多 Agent 协作

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// 子代理委派工具 (无 proxy 模式)
///
/// 无 router 模式仅支持 list_agents 等基本操作。
/// 启用 `proxy` feature 后支持 delegate/delegate_by_capability。
#[cfg(not(feature = "proxy"))]
pub struct SpawnSubagentTool;

#[cfg(not(feature = "proxy"))]
impl SpawnSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(feature = "proxy"))]
impl Default for SpawnSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "proxy"))]
impl Attributable for SpawnSubagentTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        "spawn_subagent"
    }
}

#[cfg(not(feature = "proxy"))]
#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "将任务委派给另一个 agent 执行。当前未配置 proxy, 仅支持 list_agents。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["delegate", "delegate_by_capability", "list_agents", "get_agent"],
                    "description": "操作类型"
                },
                "to": { "type": "string", "description": "目标 agent 名称" },
                "capability": { "type": "string", "description": "能力标签" },
                "prompt": { "type": "string", "description": "任务描述" },
                "name": { "type": "string", "description": "agent 名称 (get_agent)" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, _args: Value) -> Result<ToolResult> {
        Ok(ToolResult::err("未配置 TaskRouter。请启用 proxy feature 并初始化 shadow-proxy。"))
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(120))
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

#[cfg(not(feature = "proxy"))]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_metadata() {
        let tool = SpawnSubagentTool::new();
        assert_eq!(tool.name(), "spawn_subagent");
        assert_eq!(tool.timeout(), Some(Duration::from_secs(120)));
    }

    #[tokio::test]
    async fn test_no_router_error() {
        let tool = SpawnSubagentTool::new();
        let result = tool.execute(json!({"action": "list_agents"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("TaskRouter"));
    }
}
