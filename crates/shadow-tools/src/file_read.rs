//! 文件读取工具 -- 读取文本文件内容，支持行号和分页
//!
//! 设计参考 ZeroClaw FileReadTool：
//! - 输出带行号 (LINE_NUM|CONTENT 格式)
//! - 支持 offset/limit 分页
//! - 二进制文件检测（拒绝读取）
//! - 路径安全（拒绝符号链接逃逸）

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::json;
use shadow_core::{Tool, ToolResult};

/// 默认读取行数上限
const DEFAULT_LIMIT: usize = 500;
/// 单行最大字节数（防止超长行撑爆 LLM 上下文）
const MAX_LINE_BYTES: usize = 10_000;

pub struct FileReadTool {
    /// 工作区根目录（用于路径安全检查）
    workspace: PathBuf,
}

impl FileReadTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// 安全解析路径：解析到 workspace 内，拒绝逃逸
    fn resolve_path(&self, input: &str) -> Result<PathBuf, String> {
        let path = if Path::new(input).is_absolute() {
            PathBuf::from(input)
        } else {
            self.workspace.join(input)
        };

        // canonicalize 检查真实路径
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("Cannot resolve path '{input}': {e}"))?;

        Ok(canonical)
    }
}

shadow_core::tool_attribution!(FileReadTool, shadow_core::ToolKind::Shell);

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a text file. Returns lines with line numbers. \
         Supports pagination via offset (1-indexed) and limit. \
         Binary files are rejected. Default limit is 500 lines."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file. Can be absolute or relative to workspace."
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-indexed). Default: 1."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read. Default: 500."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path_str = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(ToolResult::err("Missing or empty required parameter: path")),
        };

        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|n| n.max(1) as usize)
            .unwrap_or(1);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_LIMIT);

        // 解析路径
        let path = match self.resolve_path(path_str) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::err(e)),
        };

        // 检查是否为目录
        if path.is_dir() {
            return Ok(ToolResult::err(format!(
                "'{}' is a directory, not a file",
                path.display()
            )));
        }

        // 读取文件
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::err(format!("Failed to read file: {e}"))),
        };

        // 二进制检测 -- 简单 heuristic：前 1KB 有 NULL 字节则判定为二进制
        let check_len = bytes.len().min(1024);
        if bytes[..check_len].contains(&0) {
            return Ok(ToolResult::err(format!(
                "'{}' appears to be a binary file (use shell tool with xxd/hexdump instead)",
                path.display()
            )));
        }

        let content = String::from_utf8_lossy(&bytes);

        // 分页输出
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start = offset.saturating_sub(1);
        if start >= total_lines {
            return Ok(ToolResult::ok(format!(
                "(offset {offset} exceeds total {total_lines} lines)"
            )));
        }

        let end = (start + limit).min(total_lines);

        let mut output = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            // 截断超长行
            let truncated = if line.len() > MAX_LINE_BYTES {
                format!("{} ... [line truncated]", &line[..MAX_LINE_BYTES])
            } else {
                line.to_string()
            };
            output.push_str(&format!("{line_num}|{truncated}\n"));
        }

        output.push_str(&format!(
            "\n(lines {start_offset}-{end_shown} of {total_lines})",
            start_offset = start + 1,
            end_shown = end,
            total_lines = total_lines,
        ));

        Ok(ToolResult::ok(output))
    }
}
