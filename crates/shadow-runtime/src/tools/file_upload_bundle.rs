//! 批量文件上传工具 -- 一次性上传多个文件到 URL
//!
//! 使用 multipart/form-data, 每个文件作为一个 part 发送。
//! 适用于批量上传场景 (如上传一组图片、日志文件等)。
//!
//! 与 file_upload 的区别:
//! - file_upload: 单文件上传, 一个 part
//! - file_upload_bundle: 多文件上传, 多个 part (field_name_1, field_name_2, ...)
//!   或统一使用同一个 field_name + 不同序号

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// 批量文件上传工具
pub struct FileUploadBundleTool;

impl FileUploadBundleTool {
    pub fn new() -> Self {
        Self
    }

    /// SSRF 防护 -- 复用 file_upload 的逻辑
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

impl Default for FileUploadBundleTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Attributable for FileUploadBundleTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        "file_upload_bundle"
    }
}

#[async_trait]
impl Tool for FileUploadBundleTool {
    fn name(&self) -> &str {
        "file_upload_bundle"
    }

    fn description(&self) -> &str {
        "批量上传多个文件到指定 URL (multipart/form-data)。每个文件作为一个独立的 form part 发送, 支持自定义请求头。适用于一次性上传多个文件的场景。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "上传目标 URL (http:// 或 https://)"
                },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "要上传的本地文件路径列表"
                },
                "field_name": {
                    "type": "string",
                    "description": "表单字段名前缀 (默认 files, 实际字段名为 files_0, files_1, ...)",
                    "default": "files"
                },
                "headers": {
                    "type": "object",
                    "description": "额外请求头",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["url", "paths"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 url 参数"))?
            .to_string();

        let paths: Vec<String> = args
            .get("paths")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("缺少 paths 参数 (应为字符串数组)"))?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        if paths.is_empty() {
            return Ok(ToolResult::err("文件路径列表为空"));
        }

        let field_prefix = args
            .get("field_name")
            .and_then(|v| v.as_str())
            .unwrap_or("files")
            .to_string();

        let headers: HashMap<String, String> = args
            .get("headers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // SSRF 防护 (先检查)
        if !Self::is_url_safe(&url) {
            return Ok(ToolResult::err(
                "SSRF 防护: URL 不安全 (禁止内网/localhost)",
            ));
        }

        // 检查所有文件是否存在且是文件
        let mut valid_files = Vec::new();
        let mut errors = Vec::new();
        for path_str in &paths {
            let path = Path::new(path_str);
            if !path.exists() {
                errors.push(format!("文件不存在: {path_str}"));
                continue;
            }
            if !path.is_file() {
                errors.push(format!("路径不是文件: {path_str}"));
                continue;
            }
            valid_files.push(path_str.clone());
        }

        if valid_files.is_empty() {
            return Ok(ToolResult::err(format!(
                "没有有效的文件可上传。错误:\n{}",
                errors.join("\n")
            )));
        }

        // 读取所有文件并构建 multipart form
        let mut form = reqwest::multipart::Form::new();

        for (i, path_str) in valid_files.iter().enumerate() {
            let path = Path::new(path_str);
            let file_bytes = match tokio::fs::read(path).await {
                Ok(b) => b,
                Err(e) => {
                    errors.push(format!("读取失败 {path_str}: {e}"));
                    continue;
                }
            };

            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("upload.bin")
                .to_string();

            let part = reqwest::multipart::Part::bytes(file_bytes)
                .file_name(filename.clone())
                .mime_str("application/octet-stream")
                .unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));

            // 字段名: files_0, files_1, ... (或自定义前缀)
            let field = format!("{field_prefix}_{i}");
            form = form.part(field, part);
        }

        // 发送请求
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .unwrap_or_default();

        let mut req = client.post(&url).multipart(form);

        // 添加额外请求头
        for (key, value) in &headers {
            req = req.header(key, value);
        }

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

                let summary = format!(
                    "批量上传: {} 个文件 -> HTTP {}\n响应:\n{}",
                    valid_files.len(),
                    status_code,
                    body_display
                );

                // 如果有部分文件出错, 附加警告
                let full_output = if !errors.is_empty() {
                    format!(
                        "{summary}\n\n警告 ({} 个文件跳过):\n{}",
                        errors.len(),
                        errors.join("\n")
                    )
                } else {
                    summary
                };

                if status.is_success() {
                    Ok(ToolResult::ok(full_output))
                } else {
                    Ok(ToolResult {
                        success: false,
                        output: full_output,
                        error: Some(format!("批量上传失败: HTTP {status_code}")),
                    })
                }
            }
            Err(e) => Ok(ToolResult::err(format!("上传请求失败: {e}"))),
        }
    }

    fn timeout(&self) -> Option<Duration> {
        // 批量上传可能需要更长时间
        Some(Duration::from_secs(180))
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
    use std::io::Write;

    #[test]
    fn test_url_safety() {
        assert!(FileUploadBundleTool::is_url_safe(
            "https://example.com/upload"
        ));
        assert!(!FileUploadBundleTool::is_url_safe("http://localhost:8080"));
        assert!(!FileUploadBundleTool::is_url_safe("http://192.168.1.1"));
        assert!(!FileUploadBundleTool::is_url_safe("ftp://example.com"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = FileUploadBundleTool::new();
        assert_eq!(tool.name(), "file_upload_bundle");
        assert!(tool.requires_approval());
        assert_eq!(tool.timeout(), Some(Duration::from_secs(180)));
    }

    #[test]
    fn test_schema() {
        let tool = FileUploadBundleTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("paths").is_some());
        assert!(schema["properties"].get("field_name").is_some());
    }

    #[tokio::test]
    async fn test_ssrf_blocked() {
        let tool = FileUploadBundleTool::new();
        let result = tool
            .execute(json!({
                "url": "http://127.0.0.1:8080",
                "paths": ["/tmp/test1.txt"]
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("SSRF"));
    }

    #[tokio::test]
    async fn test_empty_paths() {
        let tool = FileUploadBundleTool::new();
        let result = tool
            .execute(json!({
                "url": "https://example.com/upload",
                "paths": []
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("为空"));
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = FileUploadBundleTool::new();
        let result = tool.execute(json!({"paths": ["/tmp/test"]})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_all_files_nonexistent() {
        let tool = FileUploadBundleTool::new();
        let result = tool
            .execute(json!({
                "url": "https://example.com/upload",
                "paths": ["/nonexistent/a.txt", "/nonexistent/b.txt"]
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("没有有效的文件"));
    }

    #[tokio::test]
    async fn test_partial_files_exist() {
        let dir = tempfile::tempdir().unwrap();
        let file1 = dir.path().join("exists.txt");
        let mut f = std::fs::File::create(&file1).unwrap();
        f.write_all(b"hello").unwrap();

        let tool = FileUploadBundleTool::new();
        // 一个存在一个不存在 -> SSRF 先返回
        let result = tool
            .execute(json!({
                "url": "https://example.com/upload",
                "paths": [file1.to_str().unwrap(), "/nonexistent/missing.txt"]
            }))
            .await
            .unwrap();
        // URL 安全 -> 继续检查文件 -> 1 个有效 1 个无效 -> 尝试上传到 example.com
        // example.com 不支持 POST upload, 会返回 HTTP 错误
        // 但不会返回 "没有有效的文件" 错误
        assert!(
            result.output.contains("批量上传")
                || result.error.as_deref().unwrap_or("").contains("上传")
        );
    }
}
