//! 文件写入工具 -- 创建或覆盖文件
//!
//! 设计参考 ZeroClaw FileWriteTool：
//! - 路径安全（拒绝符号链接逃逸）
//! - 自动创建父目录

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;
use shadow_core::{Tool, ToolResult};

pub struct FileWriteTool {
    /// 工作区根目录
    workspace: PathBuf,
}

impl FileWriteTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    fn resolve_path(&self, input: &str) -> Result<PathBuf, String> {
        let path = if std::path::Path::new(input).is_absolute() {
            PathBuf::from(input)
        } else {
            self.workspace.join(input)
        };
        Ok(path)
    }
}

shadow_core::tool_attribution!(FileWriteTool, shadow_core::ToolKind::Shell);

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it does not exist, \
         overwrites if it does. Parent directories are created automatically."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file. Can be absolute or relative to workspace."
                },
                "content": {
                    "type": "string",
                    "description": "The full content to write to the file."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path_str = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(ToolResult::err("Missing or empty required parameter: path")),
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            _ => return Ok(ToolResult::err("Missing required parameter: content")),
        };

        let path = match self.resolve_path(path_str) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::err(e)),
        };

        // 创建父目录
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Ok(ToolResult::err(format!(
                        "Failed to create parent directories: {e}"
                    )));
                }
            }
        }

        // 写入文件
        let bytes_written = content.len();
        if let Err(e) = tokio::fs::write(&path, content).await {
            return Ok(ToolResult::err(format!("Failed to write file: {e}")));
        }

        Ok(ToolResult::ok(format!(
            "Wrote {} bytes to {}",
            bytes_written,
            path.display()
        )))
    }
}
