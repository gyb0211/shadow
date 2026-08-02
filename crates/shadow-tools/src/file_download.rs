//! 文件下载工具 -- 从 URL 下载文件到本地
//!
//! 支持 HTTP/HTTPS 下载，自动跟随重定向，文件大小限制

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::json;
use shadow_core::{Tool, ToolResult};
use std::path::PathBuf;

/// 最大下载文件大小 (100 MB)
const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;
/// 下载超时 (60 秒)
const DOWNLOAD_TIMEOUT_SECS: u64 = 60;

/// 文件下载工具
pub struct FileDownloadTool {
    /// 工作区根目录
    workspace: PathBuf,
    /// HTTP 客户端
    http: reqwest::Client,
}

impl FileDownloadTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
            .build()
            .expect("Failed to build HTTP client for file_download");

        Self {
            workspace: workspace.into(),
            http,
        }
    }

    /// 解析保存路径
    fn resolve_path(&self, input: &str) -> PathBuf {
        if std::path::Path::new(input).is_absolute() {
            PathBuf::from(input)
        } else {
            self.workspace.join(input)
        }
    }
}

shadow_core::tool_attribution!(FileDownloadTool, shadow_core::ToolKind::FetchUrl);

#[async_trait]
impl Tool for FileDownloadTool {
    fn name(&self) -> &str {
        "file_download"
    }

    fn description(&self) -> &str {
        "Download a file from a URL to the local filesystem. \
         Supports HTTP/HTTPS. Max file size: 100MB. \
         Automatically creates parent directories."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to download from (http:// or https://)"
                },
                "dest_path": {
                    "type": "string",
                    "description": "Local file path to save the downloaded file. Can be absolute or relative to workspace."
                }
            },
            "required": ["url", "dest_path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let url = match args.get("url").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u,
            _ => return Ok(ToolResult::err("Missing or empty required parameter: url")),
        };

        let dest_path = match args.get("dest_path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(ToolResult::err("Missing or empty required parameter: dest_path")),
        };

        // 验证 URL scheme
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(ToolResult::err("URL must start with http:// or https://"));
        }

        let dest = self.resolve_path(dest_path);

        // 创建父目录
        if let Some(parent) = dest.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult::err(format!("Failed to create directory: {e}")));
            }
        }

        // 下载
        let resp = match self.http.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("Download request failed: {e}"))),
        };

        let status = resp.status();
        if !status.is_success() {
            return Ok(ToolResult::err(format!("Download failed: HTTP {status}")));
        }

        // 检查文件大小
        if let Some(len) = resp.content_length() {
            if len > MAX_DOWNLOAD_BYTES {
                return Ok(ToolResult::err(format!(
                    "File too large: {} bytes (limit: {} bytes)",
                    len,
                    MAX_DOWNLOAD_BYTES
                )));
            }
        }

        // 读取响应体（流式，防止超大文件）
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::err(format!("Failed to read response: {e}"))),
        };

        if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
            return Ok(ToolResult::err(format!(
                "Downloaded data too large: {} bytes (limit: {} bytes)",
                bytes.len(),
                MAX_DOWNLOAD_BYTES
            )));
        }

        // 写入文件
        if let Err(e) = tokio::fs::write(&dest, &bytes).await {
            return Ok(ToolResult::err(format!("Failed to write file: {e}")));
        }

        Ok(ToolResult::ok(format!(
            "Downloaded {} ({} bytes) to {}",
            url,
            bytes.len(),
            dest.display()
        )))
    }
}