//! PDF 读取工具 -- 从 PDF 文件中提取纯文本
//!
//! 使用 poppler 的 pdftotext 命令行工具提取文本
//! 安装: brew install poppler (macOS) / apt install poppler-utils (Linux)

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use shadow_core::{Tool, ToolResult};
use std::path::PathBuf;

/// 最大 PDF 文件大小 (50 MB)
const MAX_PDF_BYTES: u64 = 50 * 1024 * 1024;
/// 默认返回字符数上限
const DEFAULT_MAX_CHARS: usize = 50_000;
/// 硬性字符上限
const MAX_OUTPUT_CHARS: usize = 200_000;

/// PDF 读取工具
pub struct PdfReadTool {
    /// 工作区根目录
    workspace: PathBuf,
}

impl PdfReadTool {
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

shadow_core::tool_attribution!(PdfReadTool, shadow_core::ToolKind::Shell);

#[async_trait]
impl Tool for PdfReadTool {
    fn name(&self) -> &str {
        "pdf_read"
    }

    fn description(&self) -> &str {
        "Extract plain text from a PDF file. Returns all readable text. \
         Image-only or encrypted PDFs return an empty result. \
         Requires pdftotext (poppler) installed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the PDF file. Can be absolute or relative to workspace."
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return. Default: 50000, max: 200000.",
                    "minimum": 1,
                    "maximum": 200000
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let path_str = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(ToolResult::err("Missing or empty required parameter: path")),
        };

        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_OUTPUT_CHARS))
            .unwrap_or(DEFAULT_MAX_CHARS);

        let path = self.resolve_path(path_str);

        // 检查文件
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) => return Ok(ToolResult::err(format!("Failed to read file metadata: {e}"))),
        };

        if metadata.len() > MAX_PDF_BYTES {
            return Ok(ToolResult::err(format!(
                "PDF too large: {} bytes (limit: {} bytes)",
                metadata.len(),
                MAX_PDF_BYTES
            )));
        }

        // 用 pdftotext 提取文本
        let output = tokio::process::Command::new("pdftotext")
            .arg(&path)
            .arg("-") // 输出到 stdout
            .output()
            .await;

        let output = match output {
            Ok(o) => o,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(ToolResult::err(
                        "pdftotext not found. Install: brew install poppler (macOS) / apt install poppler-utils (Linux)",
                    ));
                }
                return Ok(ToolResult::err(format!("Failed to run pdftotext: {e}")));
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Ok(ToolResult::err(format!("pdftotext failed: {}", stderr.trim())));
        }

        let text = String::from_utf8_lossy(&output.stdout).to_string();

        if text.trim().is_empty() {
            return Ok(ToolResult::ok(
                "PDF contains no extractable text (may be image-only or encrypted)",
            ));
        }

        // 截断
        let result = if text.chars().count() > max_chars {
            let mut truncated: String = text.chars().take(max_chars).collect();
            truncated.push_str(&format!("\n\n... [truncated at {max_chars} chars]"));
            truncated
        } else {
            text
        };

        Ok(ToolResult::ok(result))
    }
}