//! MemoryForget 工具 -- 按 key 删除一条记忆

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use shadow_core::{Attributable, Memory, Tool, ToolResult, tool_attribution};

/// MemoryForget 工具 -- 按 key 删除指定记忆条目
///
/// 让 LLM 能主动遗忘不再需要的信息 (如过时的偏好、临时上下文),
/// 删除操作需要审批 (requires_approval = true).
pub struct MemoryForgetTool {
    memory: Arc<dyn Memory>,
}

impl MemoryForgetTool {
    /// 创建 MemoryForgetTool, 需要传入 Memory 后端
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}



#[async_trait]
impl Tool for MemoryForgetTool {
    fn name(&self) -> &str {
        "memory_forget"
    }

    fn description(&self) -> &str {
        "删除一条记忆。按 key 删除指定的记忆条目。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "要删除的记忆键名 (唯一标识)"
                }
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 key 参数"))?;

        match self.memory.forget(key).await {
            Ok(true) => Ok(ToolResult::ok(format!("已删除记忆: key='{key}'"))),
            Ok(false) => Ok(ToolResult::err(format!("未找到记忆: key='{key}'"))),
            Err(e) => Ok(ToolResult::err(format!("删除记忆失败: {e}"))),
        }
    }
}
