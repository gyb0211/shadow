//! 工具 trait -- agent 可调用的能力

use crate::attribution::{Attributable, Role};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// 工具执行结果
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolResult {
    /// 成功结果
    #[must_use]
    pub fn ok(output: impl Into<String>) -> Self {
        Self { success: true, output: output.into(), error: None }
    }

    /// 失败结果
    #[must_use]
    pub fn err(error: impl Into<String>) -> Self {
        Self { success: false, output: String::new(), error: Some(error.into()) }
    }
}

/// 工具规格 -- 描述给 LLM 的工具元信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// 工具 trait
///
/// 每个工具实现此 trait, agent 通过 execute() 调用
#[async_trait]
pub trait Tool: Attributable {
    /// 工具名称
    fn name(&self) -> &str;

    /// 工具描述 (给 LLM 看)
    fn description(&self) -> &str;

    /// 参数 JSON Schema
    fn parameters_schema(&self) -> Value;

    /// 执行工具
    async fn execute(&self, args: Value) -> Result<ToolResult>;

    /// 工具超时 -- None 表示不限制, Agent 用 tokio::time::timeout 包装
    /// 默认 30 秒
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    /// 是否需要审批 -- Supervised 模式下执行前请求用户确认
    /// 默认 false, 敏感工具 (Shell/FileWrite) 覆盖为 true
    fn requires_approval(&self) -> bool {
        false
    }

    /// 工具规格 (name + description + parameters)
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

/// 工具归属 -- 所有工具的 Role 都是 Tool
pub struct ToolAttribution {
    name: String,
}

impl ToolAttribution {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}

impl Attributable for ToolAttribution {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        &self.name
    }
}

/// 宏: 为工具 struct 快速实现 Attributable
#[macro_export]
macro_rules! tool_attribution {
    ($name:expr) => {
        fn role(&self) -> $crate::attribution::Role {
            $crate::attribution::Role::Tool
        }
        fn alias(&self) -> &str {
            $name
        }
    };
}
