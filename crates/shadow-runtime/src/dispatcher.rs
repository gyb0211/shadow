//! 工具协议分发器 -- 隔离不同 LLM provider 的工具调用格式差异
//!
//! 借鉴 ZeroClaw 的 ToolDispatcher, 但大幅精简:
//! - NativeToolDispatcher: 原生 API 工具调用 (OpenAI/Anthropic)
//! - XmlToolDispatcher: XML 文本协议 (<tool_call> 标签)

use shadow_core::{ChatMessage, ChatResponse, ToolCall, ToolResult};

/// 工具协议分发器 -- 隔离不同 LLM provider 的工具调用格式差异
pub trait ToolDispatcher: Send + Sync {
    /// 解析 LLM 响应, 提取工具调用
    /// 返回 (文本内容, 工具调用列表)
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ToolCall>);

    /// 格式化工具结果为消息
    fn format_results(&self, results: &[ToolResult]) -> ChatMessage;

    /// 是否在 API 请求中发送工具规格
    fn should_send_tool_specs(&self) -> bool;
}

/// 原生工具分发器 -- 用于支持原生 API 工具调用的 provider (OpenAI/Anthropic)
///
/// 直接从 ChatResponse.tool_calls 读取工具调用,
/// 工具结果以 role="tool" 消息格式返回。
pub struct NativeToolDispatcher;

impl ToolDispatcher for NativeToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ToolCall>) {
        (response.content.clone(), response.tool_calls.clone())
    }

    fn format_results(&self, results: &[ToolResult]) -> ChatMessage {
        let content = results
            .iter()
            .map(|r| {
                if r.success {
                    r.output.clone()
                } else {
                    format!(
                        "[工具执行失败] {}",
                        r.error.as_deref().unwrap_or("未知错误")
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        ChatMessage {
            role: "tool".to_string(),
            content,
            tool_call_id: None,
            ..Default::default()
        }
    }

    fn should_send_tool_specs(&self) -> bool {
        true
    }
}

/// XML 工具分发器 -- 用于不支持原生工具调用的 provider
///
/// 通过解析 LLM 文本输出中的 <tool_call>JSON</tool_call> 标签提取工具调用。
/// 工具结果以 user 消息格式返回 (因为 provider 不支持 tool 角色)。
pub struct XmlToolDispatcher;

impl XmlToolDispatcher {
    /// 从文本中解析所有 <tool_call>JSON</tool_call> 标签
    fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
        let mut calls = Vec::new();
        let mut remaining = text;

        loop {
            // 查找 <tool_call> 开标签
            let Some(start) = remaining.find("<tool_call>") else {
                break;
            };
            let after_open = &remaining[start + "<tool_call>".len()..];
            // 查找 </tool_call> 闭标签
            let Some(end) = after_open.find("</tool_call>") else {
                break;
            };
            let json_str = after_open[..end].trim();

            // 解析 JSON: {"name": "...", "arguments": {...}}
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                let name = val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let arguments = val
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                // XML 协议无 id, 用序号生成
                let id = format!("xml-{}", calls.len());
                calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }

            // 继续查找剩余文本
            remaining = &after_open[end + "</tool_call>".len()..];
        }

        calls
    }

    /// 移除文本中的 <tool_call>...</tool_call> 标签, 保留纯文本内容
    fn strip_tool_calls(text: &str) -> String {
        let mut result = text.to_string();
        while let Some(start) = result.find("<tool_call>") {
            if let Some(end_rel) = result[start..].find("</tool_call>") {
                let end = start + end_rel + "</tool_call>".len();
                result.replace_range(start..end, "");
            } else {
                break;
            }
        }
        result.trim().to_string()
    }
}

impl ToolDispatcher for XmlToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ToolCall>) {
        let tool_calls = Self::parse_tool_calls(&response.content);
        let content = Self::strip_tool_calls(&response.content);
        (content, tool_calls)
    }

    fn format_results(&self, results: &[ToolResult]) -> ChatMessage {
        let content = results
            .iter()
            .map(|r| {
                if r.success {
                    format!("[tool_result]{}[/tool_result]", r.output)
                } else {
                    format!(
                        "[tool_result error=\"{}\"][/tool_result]",
                        r.error.as_deref().unwrap_or("未知错误")
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        ChatMessage {
            role: "user".to_string(),
            content,
            tool_call_id: None,
            ..Default::default()
        }
    }

    fn should_send_tool_specs(&self) -> bool {
        false
    }
}

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::TokenUsage;

    /// 构造测试用 ChatResponse
    fn make_response(content: &str, tool_calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            content: content.to_string(),
            tool_calls,
            usage: TokenUsage::default(),
            reasoning_content: None,
        }
    }

    /// 测试: NativeToolDispatcher 解析原生工具调用
    #[test]
    fn test_native_parse_response() {
        let dispatcher = NativeToolDispatcher;
        let tool_call = ToolCall {
            id: "call-1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };
        let response = make_response("正在搜索...", vec![tool_call.clone()]);

        let (content, calls) = dispatcher.parse_response(&response);
        assert_eq!(content, "正在搜索...");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call-1");
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments["query"], "rust");
        assert!(dispatcher.should_send_tool_specs());
    }

    /// 测试: XmlToolDispatcher 解析 XML 格式工具调用
    #[test]
    fn test_xml_parse_response() {
        let dispatcher = XmlToolDispatcher;
        let content = "我来帮你搜索。\n<tool_call>{\"name\": \"search\", \"arguments\": {\"query\": \"rust\"}}</tool_call>\n请稍等";
        let response = make_response(content, vec![]);

        let (text, calls) = dispatcher.parse_response(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments["query"], "rust");
        // 文本中不应再包含 tool_call 标签
        assert!(!text.contains("<tool_call>"));
        assert!(text.contains("我来帮你搜索"));
        assert!(text.contains("请稍等"));
        assert!(!dispatcher.should_send_tool_specs());
    }

    /// 测试: format_results 格式化工具结果 (两种分发器对比)
    #[test]
    fn test_format_results() {
        let results = vec![ToolResult::ok("搜索完成"), ToolResult::err("网络错误")];

        // 原生分发器: role="tool"
        let native = NativeToolDispatcher;
        let msg = native.format_results(&results);
        assert_eq!(msg.role, "tool");
        assert!(msg.content.contains("搜索完成"));
        assert!(msg.content.contains("工具执行失败"));
        assert!(msg.content.contains("网络错误"));

        // XML 分发器: role="user", 用 [tool_result] 标签
        let xml = XmlToolDispatcher;
        let msg = xml.format_results(&results);
        assert_eq!(msg.role, "user");
        assert!(msg.content.contains("[tool_result]搜索完成[/tool_result]"));
        assert!(msg.content.contains("error=\"网络错误\""));
    }
}
