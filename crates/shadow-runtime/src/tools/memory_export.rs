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

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(15))
    }

    /// 只读操作, 无需审批
    fn requires_approval(&self) -> bool {
        false
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

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;
    use shadow_memory::sqlite::SqliteMemory;

    /// 准备测试数据: 存入 3 条不同分类的记忆
    async fn setup(mem: &Arc<dyn Memory>) {
        mem.store(
            "lang",
            "Rust 是一门系统编程语言",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();
        mem.store("todo", "今天要写测试", MemoryCategory::Daily, None)
            .await
            .unwrap();
        mem.store(
            "ctx",
            "用户在讨论记忆工具",
            MemoryCategory::Conversation,
            None,
        )
        .await
        .unwrap();
    }

    /// 测试: 默认 json 格式导出全部
    #[tokio::test]
    async fn test_export_json_all() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        setup(&mem).await;

        let tool = MemoryExportTool::new(Arc::clone(&mem));
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("3 条记忆"));
        assert!(result.output.contains("lang"));
        assert!(result.output.contains("Rust"));
    }

    /// 测试: markdown 格式导出
    #[tokio::test]
    async fn test_export_markdown() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        setup(&mem).await;

        let tool = MemoryExportTool::new(Arc::clone(&mem));
        let result = tool.execute(json!({"format": "markdown"})).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("| Key |"));
        assert!(result.output.contains("| lang |"));
        assert!(result.output.contains("Rust"));
    }

    /// 测试: text 格式导出
    #[tokio::test]
    async fn test_export_text() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        setup(&mem).await;

        let tool = MemoryExportTool::new(Arc::clone(&mem));
        let result = tool.execute(json!({"format": "text"})).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("[core] lang"));
        assert!(result.output.contains("[daily] todo"));
    }

    /// 测试: 按 category 过滤导出
    #[tokio::test]
    async fn test_export_by_category() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        setup(&mem).await;

        let tool = MemoryExportTool::new(Arc::clone(&mem));
        let result = tool
            .execute(json!({"category": "core", "format": "text"}))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("1 条记忆"));
        assert!(result.output.contains("lang"));
        assert!(!result.output.contains("todo"));
    }

    /// 测试: 空记忆库
    #[tokio::test]
    async fn test_export_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        let tool = MemoryExportTool::new(mem);
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("没有可导出的记忆"));
    }

    /// 测试: 不支持的格式应返回错误
    #[tokio::test]
    async fn test_export_unsupported_format() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        setup(&mem).await;

        let tool = MemoryExportTool::new(Arc::clone(&mem));
        let result = tool.execute(json!({"format": "xml"})).await.unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("不支持的格式"));
    }

    /// 测试: requires_approval 为 false
    #[test]
    fn test_no_approval_required() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        let tool = MemoryExportTool::new(mem);
        assert!(!tool.requires_approval());
    }
}
