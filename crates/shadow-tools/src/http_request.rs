//! HTTP 请求工具 -- 发送自定义 HTTP 请求
//!
//! 支持 GET/POST/PUT/DELETE/PATCH 方法

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use shadow_core::{Tool, ToolResult};
use std::time::Duration;

/// HTTP 请求工具
pub struct HttpRequestTool {
    http: reqwest::Client,
}

impl HttpRequestTool {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        Self { http }
    }
}

impl Default for HttpRequestTool {
    fn default() -> Self {
        Self::new()
    }
}

shadow_core::tool_attribution!(HttpRequestTool, shadow_core::ToolKind::HttpRequest);

#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "Make HTTP requests to external APIs. Supports GET, POST, PUT, DELETE, PATCH methods. \
         Returns status code, headers, and response body."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTP or HTTPS URL to request"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"],
                    "description": "HTTP method. Default: GET",
                    "default": "GET"
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers as key-value pairs",
                    "default": {}
                },
                "body": {
                    "type": "string",
                    "description": "Optional request body (for POST, PUT, PATCH)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let url = match args.get("url").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u,
            _ => return Ok(ToolResult::err("Missing required parameter: url")),
        };

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(ToolResult::err("URL must start with http:// or https://"));
        }

        let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
        let headers = args.get("headers").cloned().unwrap_or(json!({}));
        let body = args.get("body").and_then(|v| v.as_str());

        let method = match method.to_uppercase().as_str() {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "DELETE" => reqwest::Method::DELETE,
            "PATCH" => reqwest::Method::PATCH,
            other => return Ok(ToolResult::err(format!("Unsupported method: {other}"))),
        };

        let mut req = self.http.request(method, url);

        // 添加 headers
        if let Some(obj) = headers.as_object() {
            for (key, val) in obj {
                if let Some(s) = val.as_str() {
                    req = req.header(key, s);
                }
            }
        }

        // 添加 body
        if let Some(b) = body {
            req = req.body(b.to_string());
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("Request failed: {e}"))),
        };

        let status = resp.status();
        let resp_headers: serde_json::Map<String, Value> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_str().unwrap_or("").to_string())))
            .collect();

        let resp_body = match resp.text().await {
            Ok(t) => t,
            Err(e) => return Ok(ToolResult::err(format!("Failed to read response: {e}"))),
        };

        // 截断过大的响应
        let max_body = 50_000;
        let truncated = if resp_body.len() > max_body {
            format!("{}...\n[truncated at {} chars]", &resp_body[..max_body], max_body)
        } else {
            resp_body
        };

        let result = json!({
            "status": status.as_u16(),
            "status_text": status.canonical_reason().unwrap_or(""),
            "headers": Value::Object(resp_headers),
            "body": truncated,
        });

        Ok(ToolResult::ok(serde_json::to_string_pretty(&result).unwrap_or_default()))
    }
}