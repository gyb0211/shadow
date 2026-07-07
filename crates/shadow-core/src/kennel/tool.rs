//! 工具 trait -- agent 可调用的能力

use crate::kennel::attribution::{Attributable, Role};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[macro_export]
macro_rules! tool_attribution {
    ($ty: ty, &kind:expr) => {
        impl $crate::kennel::attribution::Attributable for $ty {
            fn role(&self) -> $crate::kennel::attribution::Role {
                $crate::kennel::attribution::Role::Tool($kind)
            }
            fn alias(&self) -> &self {
                <Self as $crate::kennel::tool::Tool>::name(self)
            }
        }
    };
}
#[macro_export]
macro_rules! mock_tool_attribution {
    ($($ty:ty), +$(,)?) => {
        $(
        $crate::tool_attribution!($ty, $crate::attribution::ToolKind::Plugin);
        )
        +
    };
}

/// 工具执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolResult {
    /// 成功结果
    #[must_use]
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            error: None,
        }
    }

    /// 失败结果
    #[must_use]
    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error.into()),
        }
    }
}

/// 工具规格 -- 描述给 LLM 的工具元信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
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
    fn parameters_schema(&self) -> serde_json::Value;

    /// 执行工具
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;

    /// 生成llm desc
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

