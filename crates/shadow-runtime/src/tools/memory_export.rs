//! MemoryExport 工具 -- 导出记忆条目

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use shadow_core::{
    Attributable, Memory, MemoryCategory, MemoryEntry, Tool, ToolResult, tool_attribution,
};

/// MemoryExport 工具 -- 将记忆导出为指定格式
///
/// 支持按 category 过滤, 支持三种输出格式: json / markdown / text.
/// 此为只读操作, requires_approval = false.
pub struct MemoryExportTool {
    memory: Arc<dyn Memory>,
}

impl MemoryExportTool {
    /// 创建 MemoryExportTool, 需要传入 Memory 后端
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    /// 将记忆条目格式化为 JSON 字符串
    fn format_json(entries: &[MemoryEntry]) -> String {
        serde_json::to_string_pretty(entries).unwrap_or_else(|e| format!("序列化失败: {e}"))
    }

    /// 将记忆条目格式化为 Markdown 表格
    fn format_markdown(entries: &[MemoryEntry]) -> String {
        let mut out = String::new();
        out.push_str("| Key | Category | Content | Timestamp |\n");
        out.push_str("| --- | --- | --- | --- |\n");
        for e in entries {
            // 转义 Markdown 中的管道符, 防止破坏表格
            let content = e.content.replace('|', "\\|");
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                e.key, e.category, content, e.timestamp
            ));
        }
        out
    }

    /// 将记忆条目格式化为纯文本
    fn format_text(entries: &[MemoryEntry]) -> String {
        entries
            .iter()
            .map(|e| {
                format!(
                    "[{}] {} ({}): {}",
                    e.category, e.key, e.timestamp, e.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}



#[async_trait]
impl Tool for MemoryExportTool {
    fn name(&self) -> &str {
        "memory_export"
    }

    fn description(&self) -> &str {
        "导出记忆条目。支持按分类过滤, 支持json/markdown/text格式。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "要导出的记忆分类 (如 core/daily/conversation). 不指定则导出全部."
                },
                "format": {
                    "type": "string",
                    "description": "输出格式: json (默认) / markdown / text",
                    "enum": ["json", "markdown", "text"],
                    "default": "json"
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 解析可选的 category 过滤
        let category = args.get("category").and_then(|v| v.as_str());
        let category_filter = category.map(MemoryCategory::from_name);

        // 解析输出格式, 默认 json
        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("json");

        // 列出记忆条目
        let entries = match self.memory.list(category_filter.as_ref()).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(ToolResult::err(format!("列出记忆失败: {e}")));
            }
        };

        if entries.is_empty() {
            let scope = category.unwrap_or("全部");
            return Ok(ToolResult::ok(format!(
                "没有可导出的记忆 (category={scope})"
            )));
        }

        let count = entries.len();
        let output = match format {
            "json" => Self::format_json(&entries),
            "markdown" => Self::format_markdown(&entries),
            "text" => Self::format_text(&entries),
            other => {
                return Ok(ToolResult::err(format!(
                    "不支持的格式: '{other}', 可选: json / markdown / text"
                )));
            }
        };

        Ok(ToolResult::ok(format!(
            "已导出 {count} 条记忆 (format={format}):\n{output}"
        )))
    }
}
