//! 文件编辑工具 -- 精确字符串查找替换
//!
//! 设计参考 ZeroClaw FileEditTool：
//! - 查找 old_string 必须唯一匹配（否则报错）
//! - 替换为 new_string
//! - 支持 replace_all 批量替换

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;
use shadow_core::{Tool, ToolResult};

pub struct FileEditTool {
    workspace: PathBuf,
}

impl FileEditTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    fn resolve_path(&self, input: &str) -> PathBuf {
        if std::path::Path::new(input).is_absolute() {
            PathBuf::from(input)
        } else {
            self.workspace.join(input)
        }
    }
}

shadow_core::tool_attribution!(FileEditTool, shadow_core::ToolKind::Shell);

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "Edit a file by finding and replacing text. \
         The old_string must be unique in the file unless replace_all is true. \
         Fails if old_string is not found or is not unique."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit."
                },
                "old_string": {
                    "type": "string",
                    "description": "The text to find in the file. Must match exactly."
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace all occurrences. Default: false."
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path_str = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(ToolResult::err("Missing or empty required parameter: path")),
        };

        let old_string = match args.get("old_string").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            _ => return Ok(ToolResult::err("Missing required parameter: old_string")),
        };

        let new_string = match args.get("new_string").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            _ => return Ok(ToolResult::err("Missing required parameter: new_string")),
        };

        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if old_string == new_string {
            return Ok(ToolResult::err(
                "old_string and new_string are identical -- nothing to change",
            ));
        }

        let path = self.resolve_path(path_str);

        // 读取当前内容
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::err(format!("Failed to read file: {e}"))),
        };

        // 统计匹配数
        let match_count = content.matches(&old_string).count();

        if match_count == 0 {
            return Ok(ToolResult::err(format!(
                "old_string not found in file. \
                 Make sure the text matches exactly, including whitespace and indentation."
            )));
        }

        if match_count > 1 && !replace_all {
            return Ok(ToolResult::err(format!(
                "old_string found {match_count} times in file. \
                 Provide more context to make it unique, or set replace_all=true."
            )));
        }

        // 执行替换
        let new_content = if replace_all {
            content.replace(&old_string, &new_string)
        } else {
            content.replacen(&old_string, &new_string, 1)
        };

        // 写回文件
        if let Err(e) = tokio::fs::write(&path, &new_content).await {
            return Ok(ToolResult::err(format!("Failed to write file: {e}")));
        }

        let replaced_count = if replace_all { match_count } else { 1 };
        Ok(ToolResult::ok(format!(
            "Replaced {replaced_count} occurrence(s) in {}",
            path.display()
        )))
    }
}

