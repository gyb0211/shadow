//! HTTP 请求工具 -- 独立的 HTTP 客户端工具
//!
//! 支持 GET / POST / PUT / DELETE 方法,
//! 内置 SSRF 防护 (禁止访问内网地址)

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// HTTP 请求工具
pub struct HttpRequestTool {
    client: reqwest::Client,
}

impl HttpRequestTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self { client }
    }

    /// SSRF 防护 -- 检查 URL 是否安全
    pub(crate) fn is_url_safe(url: &str) -> Result<()> {
        let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("无效的 URL: {e}"))?;

        // 仅允许 http/https
        match parsed.scheme() {
            "http" | "https" => {}
            other => anyhow::bail!("不支持的协议: {other}, 仅允许 http/https"),
        }

        // 检查主机名
        let host = parsed.host_str().unwrap_or("").to_lowercase();

        // 禁止 localhost
        if host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" || host == "::1" {
            anyhow::bail!("SSRF 防护: 禁止访问本地地址 {host}");
        }

        // 禁止内网 IP (10.x / 172.16-31.x / 192.168.x / 169.254.x)
        if let Some(ip) = host_to_ip(&host) {
            if is_private_ip(&ip) {
                anyhow::bail!("SSRF 防护: 禁止访问内网地址 {host}");
            }
        }

        Ok(())
    }
}

/// 尝试将主机名解析为 IP (用于 SSRF 检查)
fn host_to_ip(host: &str) -> Option<std::net::IpAddr> {
    // 如果主机名本身就是 IP
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Some(ip);
    }
    None // 域名解析在请求时进行, 这里只检查 IP 形式的主机名
}

/// 判断是否为内网/保留 IP
fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local() // 169.254.x.x
                || v4.is_unspecified() // 0.0.0.0
                || v4.is_documentation()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_unicast_link_local()
        }
    }
}

impl Default for HttpRequestTool {
    fn default() -> Self {
        Self::new()
    }
}



#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "发送 HTTP 请求到指定 URL, 支持 GET/POST/PUT/DELETE 方法。内置 SSRF 防护, 禁止访问内网地址。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "请求 URL (必须以 http:// 或 https:// 开头)"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE"],
                    "default": "GET",
                    "description": "HTTP 方法"
                },
                "headers": {
                    "type": "object",
                    "description": "请求头 (键值对)",
                    "additionalProperties": { "type": "string" }
                },
                "body": {
                    "type": "string",
                    "description": "请求体 (POST/PUT 时使用)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "超时秒数 (默认 30)",
                    "default": 30
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 url 参数"))?;

        let method = args
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_uppercase();

        let headers: HashMap<String, String> = args
            .get("headers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let body = args.get("body").and_then(|v| v.as_str());

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        // SSRF 防护
        if let Err(e) = Self::is_url_safe(url) {
            return Ok(ToolResult::err(e.to_string()));
        }

        // 构建请求
        let mut req = match method.as_str() {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "DELETE" => self.client.delete(url),
            other => return Ok(ToolResult::err(format!("不支持的方法: {other}"))),
        };

        // 添加请求头
        for (key, value) in &headers {
            req = req.header(key, value);
        }

        // 添加请求体
        if let Some(b) = body {
            // 如果 Content-Type 未设置, 默认 application/json
            if !headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("content-type"))
            {
                req = req.header("Content-Type", "application/json");
            }
            req = req.body(b.to_string());
        }

        // 设置超时
        req = req.timeout(Duration::from_secs(timeout_secs));

        // 发送请求
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let status_code = status.as_u16();

                // 提取响应头
                let mut resp_headers = Vec::new();
                for (name, value) in resp.headers() {
                    if let Ok(v) = value.to_str() {
                        resp_headers.push(format!("{}: {}", name, v));
                    }
                }

                // 读取响应体 (截断到 10KB)
                let body_text = resp.text().await.unwrap_or_default();
                let truncated = body_text.len() > 10_240;
                let body_display = if truncated {
                    format!(
                        "{}...\n(截断, 共 {} 字节)",
                        &body_text[..10_240],
                        body_text.len()
                    )
                } else {
                    body_text
                };

                let output = format!(
                    "HTTP {}\n{}\n\n{}",
                    status_code,
                    resp_headers.join("\n"),
                    body_display
                );

                if status.is_success() {
                    Ok(ToolResult::ok(output))
                } else {
                    Ok(ToolResult {
                        success: false,
                        output,
                        error: Some(format!("HTTP 请求失败: {status_code}")),
                    })
                }
            }
            Err(e) => Ok(ToolResult::err(format!("HTTP 请求失败: {e}"))),
        }
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
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
    fn test_url_validation() {
        // 正常 URL
        assert!(HttpRequestTool::is_url_safe("https://example.com").is_ok());
        assert!(HttpRequestTool::is_url_safe("http://example.com/api").is_ok());

        // 危险 URL
        assert!(HttpRequestTool::is_url_safe("http://localhost:8080").is_err());
        assert!(HttpRequestTool::is_url_safe("http://127.0.0.1:8080").is_err());
        assert!(HttpRequestTool::is_url_safe("http://0.0.0.0").is_err());
        assert!(HttpRequestTool::is_url_safe("http://192.168.1.1").is_err());
        assert!(HttpRequestTool::is_url_safe("http://10.0.0.1").is_err());
        assert!(HttpRequestTool::is_url_safe("http://172.16.0.1").is_err());
        assert!(HttpRequestTool::is_url_safe("http://169.254.169.254").is_err());

        // 非法协议
        assert!(HttpRequestTool::is_url_safe("ftp://example.com").is_err());
        assert!(HttpRequestTool::is_url_safe("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_private_ip() {
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse().unwrap()));
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_ip(&"169.254.169.254".parse().unwrap()));
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = HttpRequestTool::new();
        assert_eq!(tool.name(), "http_request");
        assert!(!tool.description().is_empty());
        assert!(tool.requires_approval() == false);
        assert_eq!(tool.timeout(), Some(Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn test_ssrf_blocked() {
        let tool = HttpRequestTool::new();
        let result = tool
            .execute(json!({
                "url": "http://127.0.0.1:8080"
            }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("SSRF"));
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = HttpRequestTool::new();
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_schema() {
        let tool = HttpRequestTool::new();
        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("method").is_some());
    }
}
