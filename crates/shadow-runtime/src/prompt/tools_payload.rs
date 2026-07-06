//! ToolsPayload 多态 -- 不同 provider 的工具格式适配
//!
//! 不同 LLM provider 对工具调用的 API 格式各不相同:
//! - **OpenAI**: `{type: "function", function: {name, description, parameters}}`
//! - **Anthropic**: `{name, description, input_schema}`
//! - **PromptGuided**: 不支持原生工具时降级为文本指令 (见 [`prompt_guided`] 模块)
//!
//! 本模块通过 [`ToolFormat`] 枚举 + [`convert_tools`] 函数实现统一适配,
//! 参考 ZeroClaw 的 `ToolsPayload` 设计.
//!
//! [`prompt_guided`]: super::prompt_guided

use serde_json::{Value, json};

use shadow_core::ToolSpec;

use super::prompt_guided::build_prompt_guided_instructions;

/// 目标工具格式 -- 对应不同 provider 的工具调用 API
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolFormat {
    /// OpenAI function calling 格式
    OpenAI,
    /// Anthropic tool_use 格式
    Anthropic,
    /// 文本降级 -- 不支持原生工具的 provider
    PromptGuided,
}

/// 工具载荷 -- 转换后的工具描述, 形态随 [`ToolFormat`] 多态
///
/// - `OpenAI` / `Anthropic`: 携带结构化工具定义列表, 直接放入 API 请求的 `tools` 字段
/// - `PromptGuided`: 携带文本指令字符串, 注入 system prompt
#[derive(Debug, Clone)]
pub enum ToolsPayload {
    /// OpenAI function calling -- tools 数组放入请求体
    OpenAI {
        /// 工具定义列表
        tools: Vec<Value>,
    },
    /// Anthropic tool_use -- tools 数组放入请求体
    Anthropic {
        /// 工具定义列表
        tools: Vec<Value>,
    },
    /// 文本降级 -- 指令字符串注入 system prompt
    PromptGuided {
        /// 工具说明文本 (含调用模板)
        instructions: String,
    },
}

impl ToolsPayload {
    /// 是否为文本降级模式
    pub fn is_prompt_guided(&self) -> bool {
        matches!(self, ToolsPayload::PromptGuided { .. })
    }

    /// 获取工具数量 (PromptGuided 模式返回 0, 因为没有结构化工具)
    pub fn tool_count(&self) -> usize {
        match self {
            ToolsPayload::OpenAI { tools } | ToolsPayload::Anthropic { tools } => tools.len(),
            ToolsPayload::PromptGuided { .. } => 0,
        }
    }

    /// 获取结构化工具列表引用 (仅 OpenAI / Anthropic 模式有值)
    pub fn tools(&self) -> Option<&[Value]> {
        match self {
            ToolsPayload::OpenAI { tools } | ToolsPayload::Anthropic { tools } => Some(tools),
            ToolsPayload::PromptGuided { .. } => None,
        }
    }

    /// 获取文本指令 (仅 PromptGuided 模式有值)
    pub fn instructions(&self) -> Option<&str> {
        match self {
            ToolsPayload::PromptGuided { instructions } => Some(instructions),
            _ => None,
        }
    }
}

/// 将工具规格列表转换为目标格式的工具载荷
///
/// # 参数
/// - `tool_specs`: 工具规格列表
/// - `format`: 目标格式
///
/// # 返回
/// 对应格式的 [`ToolsPayload`]
pub fn convert_tools(tool_specs: &[ToolSpec], format: ToolFormat) -> ToolsPayload {
    match format {
        ToolFormat::OpenAI => ToolsPayload::OpenAI {
            tools: tool_specs.iter().map(convert_to_openai).collect(),
        },
        ToolFormat::Anthropic => ToolsPayload::Anthropic {
            tools: tool_specs.iter().map(convert_to_anthropic).collect(),
        },
        ToolFormat::PromptGuided => ToolsPayload::PromptGuided {
            instructions: build_prompt_guided_instructions(tool_specs),
        },
    }
}

/// 转换单个工具为 OpenAI function calling 格式
///
/// ```json
/// {"type": "function", "function": {"name": "...", "description": "...", "parameters": {...}}}
/// ```
fn convert_to_openai(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.parameters,
        }
    })
}

/// 转换单个工具为 Anthropic tool_use 格式
///
/// ```json
/// {"name": "...", "description": "...", "input_schema": {...}}
/// ```
fn convert_to_anthropic(spec: &ToolSpec) -> Value {
    json!({
        "name": spec.name,
        "description": spec.description,
        "input_schema": spec.parameters,
    })
}

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造测试用 ToolSpec
    fn make_tool(name: &str, desc: &str) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: desc.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"]
            }),
        }
    }

    /// 构造测试用工具列表
    fn make_tools() -> Vec<ToolSpec> {
        vec![
            make_tool("shell", "执行 shell 命令"),
            make_tool("file_read", "读取文件"),
        ]
    }

    /// 测试: OpenAI 格式转换
    #[test]
    fn test_convert_openai() {
        let tools = make_tools();
        let payload = convert_tools(&tools, ToolFormat::OpenAI);

        let payload_tools = payload.tools().expect("OpenAI 应有结构化工具");
        assert_eq!(payload_tools.len(), 2);
        assert_eq!(payload.tool_count(), 2);
        assert!(!payload.is_prompt_guided());

        // 验证结构
        let first = &payload_tools[0];
        assert_eq!(first["type"], "function");
        assert_eq!(first["function"]["name"], "shell");
        assert_eq!(first["function"]["description"], "执行 shell 命令");
        assert!(first["function"]["parameters"].is_object());
    }

    /// 测试: Anthropic 格式转换
    #[test]
    fn test_convert_anthropic() {
        let tools = make_tools();
        let payload = convert_tools(&tools, ToolFormat::Anthropic);

        let payload_tools = payload.tools().expect("Anthropic 应有结构化工具");
        assert_eq!(payload_tools.len(), 2);
        assert!(!payload.is_prompt_guided());

        // 验证结构 -- Anthropic 用 input_schema 而非 parameters
        let first = &payload_tools[0];
        assert_eq!(first["name"], "shell");
        assert_eq!(first["description"], "执行 shell 命令");
        assert!(first["input_schema"].is_object());
        // 不应包含 OpenAI 特有的 type/function 字段
        assert!(
            first.get("type").is_none(),
            "Anthropic 格式不应有 type 字段"
        );
        assert!(
            first.get("function").is_none(),
            "Anthropic 格式不应有 function 字段"
        );
    }

    /// 测试: PromptGuided 降级
    #[test]
    fn test_convert_prompt_guided() {
        let tools = make_tools();
        let payload = convert_tools(&tools, ToolFormat::PromptGuided);

        assert!(payload.is_prompt_guided());
        assert_eq!(payload.tool_count(), 0, "PromptGuided 工具计数应为 0");
        assert!(payload.tools().is_none(), "PromptGuided 不应有结构化工具");

        let instructions = payload.instructions().expect("应有文本指令");
        assert!(instructions.contains("<tool_call>"), "应包含调用模板");
        assert!(instructions.contains("shell"), "应包含工具名");
        assert!(instructions.contains("file_read"), "应包含工具名");
    }

    /// 测试: 空工具列表
    #[test]
    fn test_empty_tools() {
        let payload = convert_tools(&[], ToolFormat::OpenAI);
        assert_eq!(payload.tool_count(), 0);
        assert!(payload.tools().unwrap().is_empty());

        let payload_pg = convert_tools(&[], ToolFormat::PromptGuided);
        assert!(payload_pg.instructions().unwrap().is_empty());
    }

    /// 测试: OpenAI 与 Anthropic 参数传递一致
    #[test]
    fn test_parameters_passed_through() {
        let tools = vec![make_tool("test", "测试工具")];
        let original_params = tools[0].parameters.clone();

        let openai = convert_tools(&tools, ToolFormat::OpenAI);
        let anthropic = convert_tools(&tools, ToolFormat::Anthropic);

        let openai_params = &openai.tools().unwrap()[0]["function"]["parameters"];
        let anthropic_params = &anthropic.tools().unwrap()[0]["input_schema"];

        assert_eq!(
            openai_params, &original_params,
            "OpenAI 应原样传递 parameters"
        );
        assert_eq!(
            anthropic_params, &original_params,
            "Anthropic 应原样传递 parameters 为 input_schema"
        );
    }

    /// 测试: ToolFormat 枚举相等性
    #[test]
    fn test_tool_format_eq() {
        assert_eq!(ToolFormat::OpenAI, ToolFormat::OpenAI);
        assert_ne!(ToolFormat::OpenAI, ToolFormat::Anthropic);
        assert_ne!(ToolFormat::Anthropic, ToolFormat::PromptGuided);
    }
}
