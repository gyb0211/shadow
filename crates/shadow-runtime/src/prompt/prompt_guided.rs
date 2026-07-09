//! PromptGuided 降级 -- 不支持原生工具调用的 provider 用文本解析
//!
//! 某些 LLM provider (如 Ollama 部分模型、旧版 API) 不支持原生 function calling,
//! 此时将工具说明写入 system prompt, 让 LLM 用 XML 块 `<tool_call>` 表达调用意图,
//! 再由本模块从文本响应中解析出结构化的工具调用.
//!
//! # 工作流程
//! 1. [`build_prompt_guided_instructions`] 生成工具说明文本, 注入 system prompt
//! 2. LLM 回复中包含 `<tool_call>{...}</tool_call>` 块
//! 3. [`parse_prompt_guided_response`] 解析出 [`ParsedToolCall`] 列表
//!
//! 参考 ZeroClaw 的 prompt_guided 降级方案.

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

use shadow_core::ToolSpec;

/// 解析出的工具调用 -- 从 LLM 文本响应中提取
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    /// 工具名称
    pub name: String,
    /// 工具参数 (JSON Value)
    pub arguments: Value,
}

/// 匹配 `<tool_call>...</tool_call>` 块的正则 (单行模式, 非贪婪)
fn tool_call_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // 匹配 <tool_call> 标签包裹的内容, 支持前后空白
        Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap()
    })
}

/// 生成 PromptGuided 模式的工具说明文本 (注入 system prompt)
///
/// 格式包含两部分:
/// 1. 调用模板 -- 告诉 LLM 用 XML 块表达工具调用
/// 2. 可用工具列表 -- 每个工具的名称、描述、参数 schema
///
/// # 示例输出
/// ```text
/// 你可以使用以下工具。调用工具时使用 XML 格式:
/// <tool_call>
/// {"name": "shell", "arguments": {"command": "ls -la"}}
/// </tool_call>
///
/// 可用工具:
/// 1. shell - 执行 shell 命令
///    参数: {"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}
/// ```
pub fn build_prompt_guided_instructions(tools: &[ToolSpec]) -> String {
    if tools.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();

    // 调用模板说明
    lines.push("你可以使用以下工具。调用工具时使用 XML 格式:".to_string());
    lines.push("<tool_call>".to_string());
    lines.push(r#"{"name": "shell", "arguments": {"command": "ls -la"}}"#.to_string());
    lines.push("</tool_call>".to_string());
    lines.push(String::new()); // 空行分隔
    lines.push("可用工具:".to_string());

    // 工具列表
    for (idx, tool) in tools.iter().enumerate() {
        let n = idx + 1;
        lines.push(format!("{n}. {} - {}", tool.name, tool.description));
        // 将 parameters schema 序列化为紧凑 JSON
        let params = serde_json::to_string(&tool.parameters).unwrap_or_else(|_| "{}".to_string());
        lines.push(format!("   参数: {params}"));
    }

    lines.join("\n")
}

/// 从 LLM 文本响应中解析 `<tool_call>...</tool_call>` 块
///
/// 每个块内应为一个 JSON 对象, 包含 `name` 和 `arguments` 字段.
/// 解析失败的块会被跳过 (返回的结果中不包含).
///
/// # 参数
/// - `text`: LLM 的完整文本响应
///
/// # 返回
/// 解析出的工具调用列表 (保持出现顺序)
pub fn parse_prompt_guided_response(text: &str) -> Vec<ParsedToolCall> {
    let re = tool_call_regex();
    let mut calls = Vec::new();

    for cap in re.captures_iter(text) {
        let raw = cap.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        if raw.is_empty() {
            continue;
        }
        // 解析 JSON
        match serde_json::from_str::<Value>(raw) {
            Ok(obj) => {
                let name = obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let arguments = obj.get("arguments").cloned().unwrap_or(Value::Null);
                calls.push(ParsedToolCall { name, arguments });
            }
            Err(_) => {
                // JSON 解析失败, 跳过该块
                continue;
            }
        }
    }

    calls
}

// ── 单元测试 ──
