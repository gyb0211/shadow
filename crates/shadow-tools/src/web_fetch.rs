//! 网页抓取工具 -- 获取网页内容并转为纯文本
//!
//! HTML 自动转为可读文本，JSON/纯文本原样返回

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use shadow_core::{Tool, ToolResult};
use std::time::Duration;

/// 网页抓取工具
pub struct WebFetchTool {
    http: reqwest::Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("Failed to build HTTP client");
        Self { http }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

shadow_core::tool_attribution!(WebFetchTool, shadow_core::ToolKind::FetchUrl);

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page and return its content as clean plain text. \
         HTML pages are automatically converted to readable text. \
         JSON and plain text responses are returned as-is."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The HTTP or HTTPS URL to fetch"
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

        let resp = match self.http.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("Fetch failed: {e}"))),
        };

        let status = resp.status();
        if !status.is_success() {
            return Ok(ToolResult::err(format!("HTTP {status}")));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = match resp.text().await {
            Ok(t) => t,
            Err(e) => return Ok(ToolResult::err(format!("Failed to read body: {e}"))),
        };

        // JSON 或纯文本直接返回
        let text = if content_type.contains("json") || content_type.contains("text/plain") {
            body
        } else if content_type.contains("html") {
            // 简单 HTML 转文本
            html_to_text(&body)
        } else {
            body
        };

        // 截断
        let max_chars = 50_000;
        let result = if text.len() > max_chars {
            format!("{}...\n[truncated at {} chars]", &text[..max_chars], max_chars)
        } else {
            text
        };

        Ok(ToolResult::ok(result))
    }
}

/// 简单 HTML 转纯文本
fn html_to_text(html: &str) -> String {
    // 移除 script/style 标签及内容
    let mut text = html.to_string();

    // 移除 <script>...</script>
    while let Some(start) = text.find("<script") {
        if let Some(end) = text[start..].find("</script>") {
            text.replace_range(start..start + end + 9, "");
        } else {
            break;
        }
    }

    // 移除 <style>...</style>
    while let Some(start) = text.find("<style") {
        if let Some(end) = text[start..].find("</style>") {
            text.replace_range(start..start + end + 8, "");
        } else {
            break;
        }
    }

    // 块级元素前加换行
    for tag in &["<p", "<div", "<br", "<li", "<h1", "<h2", "<h3", "<h4", "<tr"] {
        text = text.replace(tag, &format!("\n{}", tag));
    }

    // 移除所有 HTML 标签
    let mut result = String::new();
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    // 解码常见 HTML 实体
    result = result
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");

    // 清理多余空白
    result = result
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    result
}