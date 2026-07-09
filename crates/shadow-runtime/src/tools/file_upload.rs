//! 文件上传工具 -- 通过 multipart/form-data 上传文件到 URL

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// 文件上传工具
pub struct FileUploadTool;

impl FileUploadTool {
    pub fn new() -> Self {
        Self
    }

    /// SSRF 防护 -- 检查 URL 是否安全
    fn is_url_safe(url: &str) -> bool {
        let Ok(parsed) = url::Url::parse(url) else {
            return false;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return false;
        }
        let host = parsed.host_str().unwrap_or("").to_lowercase();
        if host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" || host == "::1" {
            return false;
        }
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if is_private_ip(&ip) {
                return false;
            }
        }
        true
    }
}

/// 判断是否为内网/保留 IP
fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

impl Default for FileUploadTool {
    fn default() -> Self {
        Self::new()
    }
}



#[async_trait]
impl Tool for FileUploadTool {
    fn name(&self) -> &str {
        "file_upload"
    }

    fn description(&self) -> &str {
        "上传文件到指定 URL (multipart/form-data)。支持自定义请求头和字段名。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "上传目标 URL (http:// 或 https://)"
                },
                "path": {
                    "type": "string",
                    "description": "要上传的本地文件路径"
                },
                "field_name": {
                    "type": "string",
                    "description": "表单字段名 (默认 file)",
                    "default": "file"
                },
                "headers": {
                    "type": "object",
                    "description": "额外请求头",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["url", "path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 url 参数"))?
            .to_string();

        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?
            .to_string();

        let field_name = args
            .get("field_name")
            .and_then(|v| v.as_str())
            .unwrap_or("file")
            .to_string();

        let headers: HashMap<String, String> = args
            .get("headers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let path = Path::new(&path_str);

        // SSRF 防护 (先检查 URL 安全, 再检查文件)
        if !Self::is_url_safe(&url) {
            return Ok(ToolResult::err(
                "SSRF 防护: URL 不安全 (禁止内网/localhost)",
            ));
        }

        // 检查文件存在
        if !path.exists() {
            return Ok(ToolResult::err(format!("文件不存在: {path_str}")));
        }

        // 检查是文件不是目录
        if !path.is_file() {
            return Ok(ToolResult::err(format!("路径不是文件: {path_str}")));
        }

        // 读取文件
        let file_bytes = match tokio::fs::read(path).await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::err(format!("读取文件失败: {e}"))),
        };

        let file_size = file_bytes.len();
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("upload.bin")
            .to_string();

        // 构建 multipart 请求
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(filename.clone())
            .mime_str("application/octet-stream")
            .unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));

        let form = reqwest::multipart::Form::new().part(field_name, part);

        let mut req = client.post(&url).multipart(form);

        // 添加额外请求头
        for (key, value) in &headers {
            req = req.header(key, value);
        }

        // 发送请求
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let status_code = status.as_u16();
                let body = resp.text().await.unwrap_or_default();

                // 截断响应体
                let body_display = if body.len() > 5120 {
                    format!("{}...(截断, 共 {} 字节)", &body[..5120], body.len())
                } else {
                    body
                };

                if status.is_success() {
                    Ok(ToolResult::ok(format!(
                        "上传成功: {} ({} bytes) -> HTTP {}\n\n{}",
                        filename, file_size, status_code, body_display
                    )))
                } else {
                    Ok(ToolResult {
                        success: false,
                        output: format!("HTTP {} | {}", status_code, body_display),
                        error: Some(format!("上传失败: HTTP {status_code}")),
                    })
                }
            }
            Err(e) => Ok(ToolResult::err(format!("上传请求失败: {e}"))),
        }
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(120))
    }

    fn requires_approval(&self) -> bool {
        true
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_safety() {
        assert!(FileUploadTool::is_url_safe("https://example.com/upload"));
        assert!(FileUploadTool::is_url_safe("http://api.example.com/file"));
        assert!(!FileUploadTool::is_url_safe("http://localhost:8080"));
        assert!(!FileUploadTool::is_url_safe("http://127.0.0.1:3000"));
        assert!(!FileUploadTool::is_url_safe("http://192.168.1.1"));
        assert!(!FileUploadTool::is_url_safe("ftp://example.com"));
        assert!(!FileUploadTool::is_url_safe("not-a-url"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = FileUploadTool::new();
        assert_eq!(tool.name(), "file_upload");
        assert!(tool.requires_approval());
        assert_eq!(tool.timeout(), Some(Duration::from_secs(120)));
    }

    #[test]
    fn test_schema() {
        let tool = FileUploadTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("path").is_some());
        assert!(schema["properties"].get("field_name").is_some());
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = FileUploadTool::new();
        let result = tool.execute(json!({"path": "/tmp/test"})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ssrf_blocked() {
        let tool = FileUploadTool::new();
        // SSRF 检查在文件存在检查之前
        let result = tool
            .execute(json!({
                "url": "http://127.0.0.1:8080",
                "path": "/tmp/test"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("SSRF"));
    }

    #[tokio::test]
    async fn test_file_not_found() {
        let tool = FileUploadTool::new();
        let result = tool
            .execute(json!({
                "url": "https://example.com/upload",
                "path": "/nonexistent/file.txt"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("不存在"));
    }
}
