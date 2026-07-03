//! MemoryRecall 工具 -- 检索相关记忆

use shadow_core::{tool_attribution, Attributable, Memory, Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// MemoryRecall 工具 -- 通过关键词检索记忆条目
///
/// 将 agent 的 Memory 后端暴露为 LLM 可调用的工具,
/// 让 LLM 能主动回忆之前存储的知识.
pub struct MemoryRecallTool {
    memory: Arc<dyn Memory>,
}

impl MemoryRecallTool {
    /// 创建 MemoryRecallTool, 需要传入 Memory 后端
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

impl Attributable for MemoryRecallTool {
    tool_attribution!("memory_recall");
}

#[async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "检索相关记忆。输入关键词，返回匹配的记忆条目。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索关键词"
                },
                "limit": {
                    "type": "integer",
                    "description": "返回结果上限",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 query 参数"))?;

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(5);

        let entries = match self.memory.recall(query, limit, None).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(ToolResult::err(format!("检索记忆失败: {e}")));
            }
        };

        if entries.is_empty() {
            return Ok(ToolResult::ok(format!("未找到与 '{query}' 相关的记忆")));
        }

        // 格式化为 "key: content\ntimestamp" 列表
        let output = entries
            .iter()
            .map(|e| format!("{}: {}\n{}", e.key, e.content, e.timestamp))
            .collect::<Vec<_>>()
            .join("\n---\n");

        Ok(ToolResult::ok(output))
    }
}
