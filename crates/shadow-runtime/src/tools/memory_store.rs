//! MemoryStore 工具 -- 存储一条记忆

use shadow_core::{tool_attribution, Attributable, Memory, MemoryCategory, Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// MemoryStore 工具 -- 存储一条记忆供后续检索
///
/// 让 LLM 能主动保存重要信息 (用户偏好、任务上下文等),
/// 后续可通过 memory_recall 检索.
pub struct MemoryStoreTool {
    memory: Arc<dyn Memory>,
}

impl MemoryStoreTool {
    /// 创建 MemoryStoreTool, 需要传入 Memory 后端
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

impl Attributable for MemoryStoreTool {
    tool_attribution!("memory_store");
}

#[async_trait]
impl Tool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "存储一条记忆。用于保存重要信息供后续检索。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "记忆键名 (唯一标识)"
                },
                "content": {
                    "type": "string",
                    "description": "记忆内容"
                },
                "category": {
                    "type": "string",
                    "description": "记忆分类",
                    "default": "core"
                }
            },
            "required": ["key", "content"]
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 key 参数"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 content 参数"))?;

        let category_str = args
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("core");

        let category = MemoryCategory::from_name(category_str);

        match self.memory.store(key, content, category, None).await {
            Ok(()) => Ok(ToolResult::ok(format!(
                "已存储记忆: key='{key}', category='{category_str}'"
            ))),
            Err(e) => Ok(ToolResult::err(format!("存储记忆失败: {e}"))),
        }
    }
}
