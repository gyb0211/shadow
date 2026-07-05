//! FileDownload 工具 -- 从 URL 下载文件到本地
//!
//! 使用 reqwest 下载, 内置 SSRF 防护.
//! 采用原子写入: 先下载到临时文件, 再 rename 到目标路径,
//! 避免下载中断导致文件损坏.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// FileDownload 工具 -- 从 URL 下载文件到本地路径
///
/// - 内置 SSRF 防护 (禁止 localhost / 内网 IP)
/// - 原子写入: 先写 .tmp 再 rename
/// - overwrite=false 时, 文件已存在则报错
/// - 超时 120 秒 (支持大文件)
pub struct FileDownloadTool {
    client: reqwest::Client,
}

impl FileDownloadTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();
        Self { client }
    }

    /// SSRF 防护 -- 检查 URL 是否安全
    fn check_url(url: &str) -> Result<()> {
        let parsed = url::Url::parse(url)
            .map_err(|e| anyhow::anyhow!("无效的 URL: {e}"))?;

        match parsed.scheme() {
            "http" | "https" => {}
            other => anyhow::bail!("不支持的协议: {other}, 仅允许 http/https"),
        }

        let host = parsed.host_str().unwrap_or("").to_lowercase();

        if host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" || host == "::1" {
            anyhow::bail!("SSRF 防护: 禁止访问本地地址 {host}");
        }

        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if is_private_ip(&ip) {
                anyhow::bail!("SSRF 防护: 禁止访问内网地址 {host}");
            }
        }

        Ok(())
    }
}

/// 判断是否为内网/保留 IP
fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified()
        }
    }
}

impl Default for FileDownloadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Attributable for FileDownloadTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        "file_download"
    }
}

#[async_trait]
impl Tool for FileDownloadTool {
    fn name(&self) -> &str {
        "file_download"
    }

    fn description(&self) -> &str {
        "从 URL 下载文件到本地路径。内置 SSRF 防护, 采用原子写入 (先写临时文件再 rename)。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "下载 URL (必须 http:// 或 https://)"
                },
                "path": {
                    "type": "string",
                    "description": "本地保存路径"
                },
                "overwrite": {
                    "type": "boolean",
                    "description": "是否覆盖已存在文件 (默认 false, 文件存在时报错)",
                    "default": false
                }
            },
            "required": ["url", "path"]
        })
    }

    fn requires_approval(&self) -> bool {
        true
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(120))
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let url = args.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 url 参数"))?;

        let path = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?;

        let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);

        // SSRF 防护
        if let Err(e) = Self::check_url(url) {
            return Ok(ToolResult::err(e.to_string()));
        }

        let target = std::path::Path::new(path);

        // 检查文件是否已存在
        if !overwrite && tokio::fs::metadata(target).await.is_ok() {
            return Ok(ToolResult::err(format!(
                "文件已存在 '{path}', 如需覆盖请设置 overwrite=true"
            )));
        }

        // 确保父目录存在
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
        }

        // 生成临时文件路径
        let tmp_path = {
            let mut s = target.to_string_lossy().into_owned();
            s.push_str(".tmp");
            std::path::PathBuf::from(s)
        };

        // 发起下载请求
        let resp = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("下载失败: {e}"))),
        };

        let status = resp.status();
        if !status.is_success() {
            return Ok(ToolResult::err(format!("下载失败: HTTP {status}")));
        }

        // 读取响应体
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::err(format!("读取响应数据失败: {e}"))),
        };

        let size = bytes.len();

        // 写入临时文件
        if let Err(e) = tokio::fs::write(&tmp_path, &bytes).await {
            return Ok(ToolResult::err(format!("写入临时文件失败: {e}")));
        }

        // 原子 rename 到目标路径
        if let Err(e) = tokio::fs::rename(&tmp_path, target).await {
            tokio::fs::remove_file(&tmp_path).await.ok();
            return Ok(ToolResult::err(format!("重命名文件失败: {e}")));
        }

        Ok(ToolResult::ok(format!(
            "下载完成: {url} -> {path} ({} bytes)",
            format_size(size)
        )))
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

/// 格式化文件大小
fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_url() {
        assert!(FileDownloadTool::check_url("https://example.com/file.zip").is_ok());
        assert!(FileDownloadTool::check_url("http://example.com/file.zip").is_ok());

        assert!(FileDownloadTool::check_url("http://localhost:8080").is_err());
        assert!(FileDownloadTool::check_url("http://127.0.0.1:3000").is_err());
        assert!(FileDownloadTool::check_url("http://192.168.1.1").is_err());
        assert!(FileDownloadTool::check_url("http://10.0.0.1").is_err());
        assert!(FileDownloadTool::check_url("ftp://example.com").is_err());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
    }

    #[test]
    fn test_tool_metadata() {
        let tool = FileDownloadTool::new();
        assert_eq!(tool.name(), "file_download");
        assert!(tool.requires_approval());
        assert_eq!(tool.timeout(), Some(Duration::from_secs(120)));
    }

    #[test]
    fn test_schema() {
        let tool = FileDownloadTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("path").is_some());
        assert!(schema["properties"].get("overwrite").is_some());
    }

    #[tokio::test]
    async fn test_ssrf_blocked() {
        let tool = FileDownloadTool::new();
        let result = tool.execute(json!({
            "url": "http://127.0.0.1:8080/file.zip",
            "path": "/tmp/test_download.zip"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("SSRF"));
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = FileDownloadTool::new();
        let result = tool.execute(json!({"path": "/tmp/test"})).await;
        assert!(result.is_err());
    }
}
