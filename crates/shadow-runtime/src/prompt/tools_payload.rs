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
