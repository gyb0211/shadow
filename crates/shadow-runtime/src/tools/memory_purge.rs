//! MemoryPurge 工具 -- 批量清除记忆

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use shadow_core::{Attributable, Memory, MemoryCategory, Tool, ToolResult, tool_attribution};

/// MemoryPurge 工具 -- 批量清除指定分类的记忆
///
/// 支持按 category 过滤后批量删除, 也可不指定 category 删除全部.
/// 出于安全考虑, 必须传入 confirm=true 才会执行删除.
/// requires_approval = true.
pub struct MemoryPurgeTool {
    memory: Arc<dyn Memory>,
}

impl MemoryPurgeTool {
    /// 创建 MemoryPurgeTool, 需要传入 Memory 后端
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}



#[async_trait]
impl Tool for MemoryPurgeTool {
    fn name(&self) -> &str {
        "memory_purge"
    }

    fn description(&self) -> &str {
        "批量清除记忆。可按分类过滤, 必须设置 confirm=true 才会执行。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "要清除的记忆分类 (如 core/daily/conversation). 不指定则清除全部."
                },
                "confirm": {
                    "type": "boolean",
                    "description": "安全确认, 必须为 true 才执行删除操作",
                    "default": false
                }
            },
            "required": ["confirm"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 安全确认检查
        let confirm = args
            .get("confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !confirm {
            return Ok(ToolResult::err(
                "安全确认未通过: 请设置 confirm=true 以确认批量删除操作",
            ));
        }

        // 解析可选的 category 过滤
        let category = args.get("category").and_then(|v| v.as_str());
        let category_filter = category.map(MemoryCategory::from_name);

        // 列出待删除的条目
        let entries = match self.memory.list(category_filter.as_ref()).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(ToolResult::err(format!("列出记忆失败: {e}")));
            }
        };

        let total = entries.len();
        if total == 0 {
            let scope = category.unwrap_or("全部");
            return Ok(ToolResult::ok(format!(
                "没有需要清除的记忆 (category={scope})"
            )));
        }

        // 逐个删除
        let mut deleted = 0usize;
        let mut errors = Vec::new();
        for entry in &entries {
            match self.memory.forget(&entry.key).await {
                Ok(true) => deleted += 1,
                Ok(false) => {} // key 不存在, 跳过
                Err(e) => errors.push(format!("key='{}': {e}", entry.key)),
            }
        }

        let scope = category.unwrap_or("全部");
        if errors.is_empty() {
            Ok(ToolResult::ok(format!(
                "已清除 {deleted}/{total} 条记忆 (category={scope})"
            )))
        } else {
            Ok(ToolResult::ok(format!(
                "已清除 {deleted}/{total} 条记忆 (category={scope}), 失败 {} 条: {}",
                errors.len(),
                errors.join("; ")
            )))
        }
    }
}
