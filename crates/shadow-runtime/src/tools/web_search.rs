//! Web 搜索工具 -- 通过搜索引擎搜索关键词
//!
//! 支持 DuckDuckGo Lite (无需 API key) 和 Google Custom Search (需要 key)

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// Web 搜索工具
pub struct WebSearchTool {
    client: reqwest::Client,
    /// Google API key (可选, 为空时用 DuckDuckGo)
    google_api_key: Option<String>,
    /// Google Custom Search CX
    google_cx: Option<String>,
}

/// 搜索结果条目
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

impl WebSearchTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();
        Self {
            client,
            google_api_key: None,
            google_cx: None,
        }
    }

    /// 配置 Google Custom Search
    pub fn with_google(api_key: String, cx: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self {
            client,
            google_api_key: Some(api_key),
            google_cx: Some(cx),
        }
    }

    /// DuckDuckGo Lite 搜索
    async fn search_duckduckgo(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>> {
        let url = format!(
            "https://lite.duckduckgo.com/lite/?q={}",
            urlencoding::encode(query)
        );
        let resp = self
            .client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .await?;

        let html = resp.text().await.unwrap_or_default();

        // 解析 DuckDuckGo Lite HTML 结果
        // 结果在 <a class="result-link" href="..."> 和 <td class="result-snippet"> 中
        let mut results = Vec::new();
        let mut current_title = String::new();
        let mut current_url = String::new();
        let mut current_snippet = String::new();

        // 简单 HTML 解析: 提取链接和文本
        let lines = html.lines();
        for line in lines {
            let trimmed = line.trim();
            // 链接行: <a rel="nofollow" class="result-link" href="URL">TITLE</a>
            if trimmed.contains("result-link") || trimmed.contains("class=\"result-link\"") {
                if let Some(href_start) = trimmed.find("href=\"") {
                    let rest = &trimmed[href_start + 6..];
                    if let Some(end) = rest.find('"') {
                        current_url = rest[..end].to_string();
                    }
                }
                // 提取标题 (链接文本)
                if let Some(gt) = trimmed.find('>') {
                    let rest = &trimmed[gt + 1..];
                    if let Some(lt) = rest.find('<') {
                        current_title = Self::strip_tags(&rest[..lt]);
                    }
                }
            }
            // 摘要行
            if trimmed.contains("result-snippet") {
                let clean = Self::strip_tags(trimmed);
                if !clean.is_empty() {
                    current_snippet = clean;
                    if !current_url.is_empty() {
                        results.push(SearchResult {
                            title: current_title.clone(),
                            url: current_url.clone(),
                            snippet: current_snippet.clone(),
                        });
                        if results.len() >= max_results {
                            break;
                        }
                    }
                    current_title.clear();
                    current_url.clear();
                    current_snippet.clear();
                }
            }
        }

        // 如果 HTML 解析失败, 尝试更宽松的解析
        if results.is_empty() {
            results = Self::parse_links_loose(&html, max_results);
        }

        Ok(results)
    }

    /// Google Custom Search API
    async fn search_google(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let api_key = self.google_api_key.as_ref().unwrap();
        let cx = self.google_cx.as_ref().unwrap();
        let num = max_results.min(10) as u64;

        let url = format!(
            "https://www.googleapis.com/customsearch/v1?key={}&cx={}&q={}&num={}",
            api_key,
            cx,
            urlencoding::encode(query),
            num
        );

        let resp = self.client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let mut results = Vec::new();
        if let Some(items) = json.get("items").and_then(|v| v.as_array()) {
            for item in items {
                results.push(SearchResult {
                    title: item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: item
                        .get("link")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    snippet: item
                        .get("snippet")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }

        Ok(results)
    }

    /// 去除 HTML 标签
    fn strip_tags(html: &str) -> String {
        let mut result = String::with_capacity(html.len());
        let mut in_tag = false;
        for c in html.chars() {
            if c == '<' {
                in_tag = true;
            } else if c == '>' {
                in_tag = false;
            } else if !in_tag {
                result.push(c);
            }
        }
        result
            .replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .trim()
            .to_string()
    }

    /// 宽松的链接解析 (备用)
    fn parse_links_loose(html: &str, max_results: usize) -> Vec<SearchResult> {
        let mut results = Vec::new();
        let mut iter = html.split("<a ");
        iter.next(); // 跳过第一段 (在第一个 <a 之前)
        for segment in iter {
            if results.len() >= max_results {
                break;
            }
            // 提取 href
            if let Some(href_start) = segment.find("href=\"") {
                let rest = &segment[href_start + 6..];
                if let Some(end) = rest.find('"') {
                    let url = &rest[..end];
                    // 过滤非 HTTP 链接
                    if !url.starts_with("http") {
                        continue;
                    }
                    // 提取标题
                    if let Some(gt) = segment.find('>') {
                        let after = &segment[gt + 1..];
                        if let Some(lt) = after.find('<') {
                            let title = Self::strip_tags(&after[..lt]);
                            if !title.is_empty() && title.len() > 5 {
                                results.push(SearchResult {
                                    title,
                                    url: url.to_string(),
                                    snippet: String::new(),
                                });
                            }
                        }
                    }
                }
            }
        }
        results
    }

    /// 格式化搜索结果
    fn format_results(results: &[SearchResult], query: &str) -> String {
        if results.is_empty() {
            return format!("未找到与 \"{query}\" 相关的搜索结果");
        }
        let mut lines = vec![format!("搜索 \"{query}\" 的结果 ({} 条):", results.len())];
        for (i, r) in results.iter().enumerate() {
            lines.push(format!("\n{}. {}", i + 1, r.title));
            if !r.url.is_empty() {
                lines.push(format!("   URL: {}", r.url));
            }
            if !r.snippet.is_empty() {
                lines.push(format!("   摘要: {}", r.snippet));
            }
        }
        lines.join("\n")
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Attributable for WebSearchTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        "web_search"
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "搜索 Web 内容。默认使用 DuckDuckGo (无需 API key), 可配置 Google Custom Search。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索关键词"
                },
                "max_results": {
                    "type": "integer",
                    "description": "最大返回结果数 (默认 5)",
                    "default": 5
                },
                "engine": {
                    "type": "string",
                    "enum": ["duckduckgo", "google"],
                    "default": "duckduckgo",
                    "description": "搜索引擎 (google 需要 API key)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 query 参数"))?;

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let engine = args
            .get("engine")
            .and_then(|v| v.as_str())
            .unwrap_or("duckduckgo");

        let results = match engine {
            "google" if self.google_api_key.is_some() => {
                self.search_google(query, max_results).await
            }
            _ => self.search_duckduckgo(query, max_results).await,
        };

        match results {
            Ok(r) => Ok(ToolResult::ok(Self::format_results(&r, query))),
            Err(e) => Ok(ToolResult::err(format!("搜索失败: {e}"))),
        }
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(15))
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
    fn test_strip_tags() {
        assert_eq!(WebSearchTool::strip_tags("<b>Hello</b>"), "Hello");
        assert_eq!(WebSearchTool::strip_tags("<a href=\"x\">Link</a>"), "Link");
        assert_eq!(WebSearchTool::strip_tags("No tags"), "No tags");
        assert_eq!(WebSearchTool::strip_tags("&amp;"), "&");
    }

    #[test]
    fn test_parse_links_loose() {
        let html =
            r#"<a href="https://example.com">Example Site</a> <a href="https://test.com">Test</a>"#;
        let results = WebSearchTool::parse_links_loose(html, 5);
        // 宽松解析可能提取到 1-2 个结果, 只验证至少 1 个
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("https://"));
    }

    #[test]
    fn test_format_results_empty() {
        let output = WebSearchTool::format_results(&[], "test");
        assert!(output.contains("未找到"));
    }

    #[test]
    fn test_format_results() {
        let results = vec![SearchResult {
            title: "Test".into(),
            url: "https://example.com".into(),
            snippet: "A test".into(),
        }];
        let output = WebSearchTool::format_results(&results, "test");
        assert!(output.contains("1. Test"));
        assert!(output.contains("https://example.com"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = WebSearchTool::new();
        assert_eq!(tool.name(), "web_search");
        assert_eq!(tool.timeout(), Some(Duration::from_secs(15)));
    }

    #[test]
    fn test_schema() {
        let tool = WebSearchTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("query").is_some());
        assert!(schema["properties"].get("max_results").is_some());
    }
}
