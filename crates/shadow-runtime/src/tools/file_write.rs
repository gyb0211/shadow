//! FileWrite 工具 -- 写入文件内容

use agent_core::{tool_attribution, Attributable, Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

/// FileWrite 工具 -- 将内容写入指定路径的文件
///
/// 如果父目录不存在会自动创建. 覆盖已有文件.
pub struct FileWriteTool;

impl Attributable for FileWriteTool {
    tool_attribution!("file_write");
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "写入文件内容. 参数: path (文件路径), content (文件内容). 覆盖已有文件."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "要写入的文件路径"
                },
                "content": {
                    "type": "string",
                    "description": "要写入的文件内容"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 content 参数"))?;

        // 确保父目录存在
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
        }

        tokio::fs::write(path, content)
            .await
            .map_err(|e| anyhow::anyhow!("写入文件失败 '{path}': {e}"))?;

        Ok(ToolResult::ok(format!(
            "已写入 {path} ({} 字节)",
            content.len()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_and_verify() {
        let tool = FileWriteTool;
        let path = "/tmp/shadow_test_filewrite.txt";

        // 写入
        let result = tool
            .execute(json!({"path": path, "content": "hello shadow"}))
            .await
            .unwrap();
        assert!(result.success);

        // 验证
        let content = tokio::fs::read_to_string(path).await.unwrap();
        assert_eq!(content, "hello shadow");

        // 清理
        tokio::fs::remove_file(path).await.ok();
    }
}
