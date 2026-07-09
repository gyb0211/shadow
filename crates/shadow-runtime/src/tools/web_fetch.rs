//! Web 抓取工具 -- 获取 URL 内容并转为文本/Markdown
//!
//! 支持 text / markdown / raw 三种输出格式

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// Web 抓取工具
pub struct WebFetchTool {
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();
        Self { client }
    }

    /// 去除 HTML 标签, 提取纯文本
    fn strip_html(html: &str) -> String {
        let mut result = String::with_capacity(html.len() / 2);
        let mut in_tag = false;
        let mut in_script = false;
        let mut in_style = false;
        let chars: Vec<char> = html.chars().collect();
        let lower = html.to_lowercase();

        let mut i = 0;
        while i < chars.len() {
            // 检测 <script 和 <style
            if !in_tag && lower[i..].starts_with("<script") {
                in_script = true;
                i += 7;
                continue;
            }
            if in_script {
                if lower[i..].starts_with("</script>") {
                    in_script = false;
                    i += 9;
                } else {
                    i += 1;
                }
                continue;
            }
            if !in_tag && lower[i..].starts_with("<style") {
                in_style = true;
                i += 6;
                continue;
            }
            if in_style {
                if lower[i..].starts_with("</style>") {
                    in_style = false;
                    i += 8;
                } else {
                    i += 1;
                }
                continue;
            }

            let c = chars[i];
            if c == '<' {
                in_tag = true;
            } else if c == '>' {
                in_tag = false;
                // 标签结束后插入空格/换行
                result.push('\n');
            } else if !in_tag {
                result.push(c);
            }
            i += 1;
        }

        // 解码 HTML 实体
        result
            .replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&apos;", "'")
    }

    /// 简单 HTML -> Markdown 转换
    fn html_to_markdown(html: &str) -> String {
        let mut md = String::with_capacity(html.len());
        let chars: Vec<char> = html.chars().collect();
        let lower = html.to_lowercase();
        let mut i = 0;

        while i < chars.len() {
            if lower[i..].starts_with("<script") || lower[i..].starts_with("<style") {
                // 跳过 script/style
                let close_tag = if lower[i..].starts_with("<script") {
                    "</script>"
                } else {
                    "</style>"
                };
                if let Some(pos) = lower[i..].find(close_tag) {
                    i += pos + close_tag.len();
                } else {
                    break;
                }
                continue;
            }

            // 标题
            for (tag, prefix) in [
                ("<h1", "# "),
                ("<h2", "## "),
                ("<h3", "### "),
                ("<h4", "#### "),
                ("<h5", "##### "),
                ("<h6", "###### "),
            ] {
                if lower[i..].starts_with(tag) {
                    md.push_str(prefix);
                    if let Some(gt) = chars[i..].iter().position(|&c| c == '>') {
                        i += gt + 1;
                    }
                    break;
                }
            }

            // 段落/换行
            if lower[i..].starts_with("<br") {
                md.push('\n');
                if let Some(gt) = chars[i..].iter().position(|&c| c == '>') {
                    i += gt + 1;
                }
                continue;
            }
            if lower[i..].starts_with("</p>") || lower[i..].starts_with("</div>") {
                md.push('\n');
                i += if lower[i..].starts_with("</p>") { 4 } else { 6 };
                continue;
            }

            // 链接 <a href="url">text</a>
            if lower[i..].starts_with("<a ") {
                let rest = &html[i..];
                if let Some(href_start) = rest.find("href=\"") {
                    let href_rest = &rest[href_start + 6..];
                    if let Some(href_end) = href_rest.find('"') {
                        let href = &href_rest[..href_end];
                        // 找到 > 后的文本
                        if let Some(gt) = rest.find('>') {
                            let text_start = gt + 1;
                            if let Some(close) = rest[text_start..].find("</a>") {
                                let link_text =
                                    Self::strip_html(&rest[text_start..text_start + close]);
                                md.push_str(&format!("[{link_text}]({href})"));
                                i += text_start + close + 4;
                                continue;
                            }
                        }
                    }
                }
            }

            // 代码块
            if lower[i..].starts_with("<code>") {
                i += 6;
                if let Some(close) = lower[i..].find("</code>") {
                    let code = &html[i..i + close];
                    md.push('`');
                    md.push_str(&code.replace('`', "\\`"));
                    md.push('`');
                    i += close + 7;
                    continue;
                }
            }

            // 列表项
            if lower[i..].starts_with("<li>") {
                md.push_str("- ");
                i += 4;
                continue;
            }
            if lower[i..].starts_with("</li>") {
                md.push('\n');
                i += 5;
                continue;
            }

            // 其他 HTML 标签 -- 跳过
            if chars[i] == '<' {
                if let Some(gt) = chars[i..].iter().position(|&c| c == '>') {
                    i += gt + 1;
                    continue;
                }
            }

            md.push(chars[i]);
            i += 1;
        }

        // 解码实体
        md.replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
    }

    /// 清理多余空行
    fn cleanup(text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        let mut blank_count = 0;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                blank_count += 1;
                if blank_count <= 2 {
                    result.push('\n');
                }
            } else {
                blank_count = 0;
                result.push_str(trimmed);
                result.push('\n');
            }
        }
        result.trim().to_string()
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}



#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "获取指定 URL 的内容并转为文本或 Markdown 格式。支持 text/markdown/raw 三种输出模式。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "要抓取的 URL (http:// 或 https://)"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "raw"],
                    "default": "text",
                    "description": "输出格式: text (纯文本) / markdown (Markdown) / raw (原始内容)"
                },
                "max_length": {
                    "type": "integer",
                    "description": "最大返回字符数 (默认 50000)",
                    "default": 50000
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

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let max_length = args
            .get("max_length")
            .and_then(|v| v.as_u64())
            .unwrap_or(50_000) as usize;

        // 发送请求
        let resp = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("请求失败: {e}"))),
        };

        let status = resp.status();
        if !status.is_success() {
            return Ok(ToolResult::err(format!("HTTP {}", status.as_u16())));
        }

        // 检查 content-type
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        let body = resp.text().await.unwrap_or_default();

        // 根据格式处理
        let processed = if format == "raw" {
            body.clone()
        } else if content_type.contains("json") {
            // JSON 直接返回 (美化)
            if let Ok(json_val) = serde_json::from_str::<Value>(&body) {
                serde_json::to_string_pretty(&json_val).unwrap_or(body.clone())
            } else {
                body.clone()
            }
        } else if content_type.contains("html") || content_type.contains("text/html") {
            // HTML 处理
            let text = if format == "markdown" {
                Self::html_to_markdown(&body)
            } else {
                Self::strip_html(&body)
            };
            Self::cleanup(&text)
        } else {
            // 其他类型直接返回
            body.clone()
        };

        // 截断
        let result = if processed.len() > max_length {
            format!(
                "{}\n...(截断, 共 {} 字符)",
                &processed[..max_length],
                processed.len()
            )
        } else {
            processed
        };

        Ok(ToolResult::ok(result))
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
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
    fn test_strip_html() {
        let html = "<p>Hello <b>World</b></p><script>alert('xss')</script>";
        let text = WebFetchTool::strip_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("<script>"));
    }

    #[test]
    fn test_html_to_markdown() {
        let html = "<h1>Title</h1><p>Some <a href=\"https://example.com\">link</a></p>";
        let md = WebFetchTool::html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("[link](https://example.com)"));
    }

    #[test]
    fn test_cleanup() {
        let text = "line1\n\n\n\n\nline2\n\n\n\n\n\nline3";
        let cleaned = WebFetchTool::cleanup(text);
        // cleanup 应该将连续空行压缩到最多 2 个换行
        assert!(!cleaned.contains("\n\n\n\n"));
        assert!(cleaned.contains("line1"));
        assert!(cleaned.contains("line2"));
        assert!(cleaned.contains("line3"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = WebFetchTool::new();
        assert_eq!(tool.name(), "web_fetch");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.timeout(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_schema() {
        let tool = WebFetchTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("format").is_some());
        assert!(schema["properties"].get("max_length").is_some());
    }

    #[test]
    fn test_strip_html_entities() {
        let html = "a &lt; b &amp; c &gt; d";
        let text = WebFetchTool::strip_html(html);
        assert_eq!(text.trim(), "a < b & c > d");
    }
}
