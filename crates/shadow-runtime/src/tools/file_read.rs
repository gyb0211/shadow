//! FileRead 工具 -- 读取文件内容

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_core::{Attributable, Tool, ToolResult, tool_attribution};

/// FileRead 工具 -- 读取指定路径的文件内容
///
/// 支持文本文件读取, 自动截断超大文件 (前 100KB).
pub struct FileReadTool;

impl Attributable for FileReadTool {
    tool_attribution!("file_read");
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "读取文件内容. 参数: path (文件路径). 返回文件文本内容."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "要读取的文件路径 (相对或绝对)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?;

        // 读取文件, 失败时返回 ToolResult::err 而非 Err
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::err(format!("读取文件失败 '{path}': {e}")));
            }
        };

        // 截断超大文件 (前 100KB)
        const MAX_SIZE: usize = 100 * 1024;
        let result = if content.len() > MAX_SIZE {
            format!(
                "{}\n\n[文件已截断, 显示前 100KB / 共 {} 字节]",
                &content[..MAX_SIZE],
                content.len()
            )
        } else {
            content
        };

        Ok(ToolResult::ok(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_existing_file() {
        let tool = FileReadTool;
        // 使用 workspace 根目录的 Cargo.toml (含 [workspace])
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = format!("{}/../../Cargo.toml", manifest_dir);
        let result = tool.execute(json!({"path": path})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("[workspace]"));
    }

    #[tokio::test]
    async fn read_nonexistent_file() {
        let tool = FileReadTool;
        let result = tool
            .execute(json!({"path": "/nonexistent/file.txt"}))
            .await
            .unwrap();
        assert!(!result.success);
    }
}
