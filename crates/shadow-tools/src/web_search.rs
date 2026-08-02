//! 网络搜索工具 -- 使用 DuckDuckGo 搜索

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use shadow_core::{Tool, ToolResult};
use std::time::Duration;

/// 网络搜索工具（DuckDuckGo HTML 版，免费无需 API Key）
pub struct WebSearchTool {
    http: reqwest::Client,
}

impl WebSearchTool {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("Failed to build HTTP client");
        Self { http }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

shadow_core::tool_attribution!(WebSearchTool, shadow_core::ToolKind::Search);

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns relevant search results \
         with titles, URLs, and descriptions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Be specific for better results."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results. Default: 5, max: 20.",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q,
            _ => return Ok(ToolResult::err("Missing required parameter: query")),
        };

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(20))
            .unwrap_or(5);

        // DuckDuckGo HTML 搜索
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );

        let resp = match self
            .http
            .get(&url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("Search request failed: {e}"))),
        };

        if !resp.status().is_success() {
            return Ok(ToolResult::err(format!("Search failed: HTTP {}", resp.status())));
        }

        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => return Ok(ToolResult::err(format!("Failed to read response: {e}"))),
        };

        // 解析搜索结果
        let results = parse_ddg_results(&html, max_results);

        if results.is_empty() {
            return Ok(ToolResult::ok("No results found"));
        }

        let output = json!({ "results": results });
        Ok(ToolResult::ok(serde_json::to_string_pretty(&output).unwrap_or_default()))
    }
}

/// 解析 DuckDuckGo HTML 搜索结果
fn parse_ddg_results(html: &str, max: usize) -> Vec<Value> {
    let mut results = Vec::new();

    // DuckDuckGo HTML 格式:
    // <a class="result__a" href="...">Title</a>
    // <a class="result__snippet">Description</a>
    for chunk in html.split("result__a") {
        if results.len() >= max {
            break;
        }

        // 提取 href
        let href = chunk
            .find("href=\"")
            .and_then(|i| {
                let rest = &chunk[i + 6..];
                rest.find('"').map(|end| &rest[..end])
            })
            .map(|s| s.to_string());

        // 提取标题（在 > 和 </a> 之间）
        let title = chunk
            .find('>')
            .and_then(|i| {
                let rest = &chunk[i + 1..];
                rest.find("</a>").map(|end| rest[..end].trim().to_string())
            })
            .unwrap_or_default();

        // 提取描述
        let snippet = if let Some(idx) = chunk.find("result__snippet") {
            let rest = &chunk[idx..];
            rest.find('>')
                    .and_then(|i| {
                        let r = &rest[i + 1..];
                        r.find("</a>").map(|end| r[..end].trim().to_string())
                    })
                    .unwrap_or_default()
        } else {
            String::new()
        };

        if let Some(url) = href {
            // DuckDuckGo 的 URL 可能是重定向格式
            let clean_url = if url.starts_with("//duckduckgo.com/l/?uddg=") {
                url.split("uddg=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .and_then(|s| urlencoding::decode(s).ok())
                    .map(|s| s.to_string())
                    .unwrap_or(url)
            } else {
                url
            };

            if !title.is_empty() {
                results.push(json!({
                    "title": clean_html(&title),
                    "url": clean_url,
                    "description": clean_html(&snippet),
                }));
            }
        }
    }

    results
}

/// 清理 HTML 实体
fn clean_html(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("<b>", "")
        .replace("</b>", "")
        .replace("<strong>", "")
        .replace("</strong>", "")
        .trim()
        .to_string()
}