//! 模型提供商 trait -- LLM 推理后端抽象

use crate::ToolSpec;
use crate::kennel::attribution::Attributable;
use anyhow::Ok;
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use futures::stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write;
use std::str;
use std::sync::Arc;

/// 聊天消息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "system" / "user" / "assistant" / "tool"
    pub content: String,
}

impl ChatMessage {
    pub fn system(message: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: message.into(),
        }
    }
}

/// 聊天请求
#[derive(Debug, Clone)]
pub struct ChatRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub tools: Option<&'a [ToolSpec]>,
    pub thinking: Option<NativeThinkingParams>,
}

#[derive(Debug, Clone)]
pub struct NativeThinkingParams {
    pub budget_tokens: u32,
}

/// 聊天响应
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
    /// 思考模型的原始推理内容 (DeepSeek-R1 等 API 返回的 reasoning_content 字段)
    /// 与 content 分离, 不直接显示给用户, 但回传时需要带上.
    pub reasoning_content: Option<String>,
}

/// 工具调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Token 用量
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StreamOptions {
    pub enabled: bool,
    pub count_tokens: bool,
}

impl StreamOptions {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            count_tokens: false,
        }
    }

    pub fn with_token_count(mut self) -> Self {
        self.count_tokens = true;
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("HTTP error: {0}")]
    HTTP(String),
    #[error("JSON parse error: {0}")]
    JSON(serde_json::Error),
    #[error("Invalid SSE format: {0}")]
    InvalidSse(String),
    #[error("ModelProvider error: {0}")]
    ModelProvider(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

type StreamResult<T> = std::result::Result<T, StreamError>;

/// 流式聊天块 -- SSE 增量
///
/// ModelProvider::chat_stream() 返回 BoxStream<Result<StreamChunk>>,
/// 调用方逐块消费, 实现逐字/逐词显示.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub delta: String,
    pub reasoning: Option<String>,
    pub is_final: bool,
    pub token_count: usize,
}

impl StreamChunk {
    pub fn delta(text: impl Into<String>) -> Self {
        Self {
            delta: text.into(),
            reasoning: None,
            is_final: false,
            token_count: 0,
        }
    }
    pub fn reasoning(text: impl Into<String>) -> Self {
        Self {
            delta: String::new(),
            reasoning: Some(text.into()),
            is_final: false,
            token_count: 0,
        }
    }
    pub fn final_chunk() -> Self {
        Self {
            delta: String::new(),
            reasoning: None,
            is_final: true,
            token_count: 0,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            delta: message.into(),
            reasoning: None,
            is_final: true,
            token_count: 0,
        }
    }
    pub fn with_token_estimate(mut self) -> Self {
        self.token_count = self.delta.len().div_ceil(4);
        self
    }
}

/// API key 注入位置 -- 决定 provider 如何把 key 放进 HTTP 请求
///
/// - `Bearer`: OpenAI 风格, `Authorization: Bearer <key>`
/// - `XApiKey`: Anthropic 风格, `x-api-key: <key>` (Phase 2 Anthropic native 用)
/// - `Query(name)`: 把 key 作为 URL query 参数, 如 `?key=<k>` (某些中国厂商)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AuthStyle {
    #[default]
    Bearer,
    XApiKey,
    /// key 作为 URL query 参数, 字段值是参数名 (如 "key" / "apikey")
    Custom(String),
}


#[derive(Clone, Default)]
pub struct ProviderCapabilities {
    /// 原生工具调用
    pub native_tool_calling: bool,
    /// 视觉图片支持
    pub vision: bool,

    /// 提示词缓存
    pub prompt_caching: bool,
    /// 思考能力
    pub extended_thinking: bool,
}

#[derive(Clone, Default)]
pub struct ModelInfo {
    pub id: String,
    pub pricing: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum ToolsPayload {
    Gemini {
        function_declarations: Vec<serde_json::Value>,
    },
    Anthropic {
        tools: Vec<serde_json::Value>,
    },
    OpenAI {
        tools: Vec<serde_json::Value>,
    },
    PromptGuided {
        instructions: String,
    },
}

const BASE_TEMPERATURE: f64 = 0.7;
const BASE_MAX_TOKEN: u32 = 4096;
const BASE_TIMEOUT_SECS: u64 = 120;
const BASE_WIRE_API: &str = "chat_completions";

/// 模型提供商 trait
///
/// 每个 LLM 后端实现此 trait (OpenAI/Anthropic/Ollama...)
/// 通过工厂函数按字符串 key 注册。
/// 借鉴 ZeroClaw ModelProvider, 重命名自 agent-core。
#[async_trait]
pub trait ModelProvider: Attributable {
    /// 模型能力
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    fn default_max_token(&self) -> u32 {
        BASE_MAX_TOKEN
    }

    /// 默认温度
    fn default_temperature(&self) -> f64 {
        BASE_TEMPERATURE
    }

    fn default_timeout_secs(&self) -> u64 {
        BASE_TIMEOUT_SECS
    }
    fn default_wire_api(&self) -> &str {
        BASE_WIRE_API
    }

    fn default_base_url(&self) -> Option<&str> {
        None
    }

    fn convert_tools(&self, tools: &[ToolSpec]) -> ToolsPayload {
        ToolsPayload::PromptGuided {
            instructions: build_tool_instructions_text(tools),
        }
    }

    fn supports_native_tools(&self) -> bool {
        self.capabilities().native_tool_calling
    }
    fn supports_vision(&self) -> bool {
        self.capabilities().vision
    }

    async fn warmup(&self) -> Result<()> {
        Ok(())
    }

    async fn simple_chat(
        &self,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        self.chat_with_system(None, message, model, temperature)
            .await
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String>;

    /// 列出可用模型
    async fn list_models(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }

    async fn list_models_with_pricing(&self) -> Result<Vec<ModelInfo>> {
        Ok(self
            .list_models()
            .await?
            .into_iter()
            .map(|id| ModelInfo { id, pricing: None })
            .collect())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.as_str());
        let user = messages
            .iter()
            .rfind(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        self.chat_with_system(system, user, model, temperature)
            .await
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        // 有工具 且provider不支持工具 ， 就要把工具信息放到system_prompt中
        if let Some(tools) = request.tools
            && !tools.is_empty()
            && !self.supports_native_tools()
        {
            // 必须是 prompt 引导的方式才能注入
            // 如果impl 修改了convert_tools这里 就必须要支持tool
            let tool_instructions = match self.convert_tools(tools) {
                ToolsPayload::PromptGuided { instructions } => instructions,
                payload => {
                    anyhow::bail!(
                        "ModelProvider returned non-prompt-guided tools payload ({payload:?}) while supports_native_tools() is false"
                    )
                }
            };
            // 修改system_prompt 注入工具信息
            let mut modified_messages = request.messages.to_vec();
            if let Some(system_message) = modified_messages.iter_mut().find(|m| m.role == "system")
            {
                if !system_message.content.is_empty() {
                    system_message.content.push_str("\n\n");
                }
                system_message.content.push_str(&tool_instructions);
            } else {
                modified_messages.insert(0, ChatMessage::system(tool_instructions));
            }

            let text = self
                .chat_with_history(&modified_messages, model, temperature)
                .await?;
            return Ok(ChatResponse {
                text: Some(text),
                tool_calls: vec![],
                reasoning_content: None,
                usage: None,
            });
        }

        // 如果impl支持tool。 那就让他自己实现， default中不帮impl做
        // 默认： 支持工具调用 但是 不知道怎么支持的，所以 不注入工具
        let text = self
            .chat_with_history(request.messages, model, temperature)
            .await?;

        Ok(ChatResponse {
            text: Some(text),
            tool_calls: vec![],
            reasoning_content: None,
            usage: None,
        })
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        let text = self.chat_with_history(messages, model, temperature).await?;
        Ok(ChatResponse {
            text: Some(text),
            tool_calls: vec![],
            reasoning_content: None,
            usage: None,
        })
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn supports_streaming_tool_events(&self) -> bool {
        false
    }

    fn stream_chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
        _options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        stream::empty().boxed()
    }

    fn stream_chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.as_str());
        let user = messages
            .iter()
            .rfind(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        self.stream_chat_with_system(system, user, model, temperature, options)
    }

    fn stream_chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamEvent>> {
        self.stream_chat_with_history(request.messages, model, temperature, options)
            .map(|chunk_result| chunk_result.map(StreamEvent::from_chunk))
            .boxed()
    }
}
//
// #[async_trait]
// impl<T: ModelProvider + ?Sized> ModelProvider for Arc<T> {
//     fn capabilities(&self) -> ProviderCapabilities {
//         self.as_ref().capabilities()
//     }
//
//     fn default_max_token(&self) -> u32 {
//         self.as_ref().default_max_token()
//     }
//     fn default_temperature(&self) -> f64 {
//         self.as_ref().default_temperature()
//     }
//
//     fn default_timeout_secs(&self) -> u64 {
//         self.as_ref().default_timeout_secs()
//     }
//
//     fn default_base_url(&self) -> Option<&str> {
//         self.as_ref().default_base_url()
//     }
//
//     fn default_wire_api(&self) -> &str {
//         self.as_ref().default_wire_api()
//     }
//
//     fn convert_tools(&self, tools: &[ToolSpec]) -> ToolsPayload {
//         self.as_ref().convert_tools(tools)
//     }
//
//     fn supports_native_tools(&self) -> bool {
//         self.as_ref().supports_native_tools()
//     }
//
//     fn supports_vision(&self) -> bool {
//         self.as_ref().supports_vision()
//     }
//
//     async fn chat_with_system(
//         &self,
//         system_prompt: Option<&str>,
//         message: &str,
//         model: &str,
//         temperature: Option<f64>,
//     ) -> anyhow::Result<String> {
//         self.as_ref()
//             .chat_with_system(system_prompt, message, model, temperature)
//             .await
//     }
//
//     async fn chat_with_history(
//         &self,
//         messages: &[ChatMessage],
//         model: &str,
//         temperature: Option<f64>,
//     ) -> anyhow::Result<String> {
//         self.as_ref()
//             .chat_with_history(messages, model, temperature)
//             .await
//     }
//
//     async fn chat(
//         &self,
//         request: ChatRequest<'_>,
//         model: &str,
//         temperature: Option<f64>,
//     ) -> anyhow::Result<ChatResponse> {
//         self.as_ref().chat(request, model, temperature).await
//     }
//
//     async fn warmup(&self) -> anyhow::Result<()> {
//         self.as_ref().warmup().await
//     }
//
//     async fn chat_with_tools(
//         &self,
//         messages: &[ChatMessage],
//         tools: &[serde_json::Value],
//         model: &str,
//         temperature: Option<f64>,
//     ) -> anyhow::Result<ChatResponse> {
//         self.as_ref()
//             .chat_with_tools(messages, tools, model, temperature)
//             .await
//     }
//
//     fn supports_streaming(&self) -> bool {
//         self.as_ref().supports_streaming()
//     }
//
//     fn supports_streaming_tool_events(&self) -> bool {
//         self.as_ref().supports_streaming_tool_events()
//     }
//
//     fn stream_chat_with_system(
//         &self,
//         system_prompt: Option<&str>,
//         message: &str,
//         model: &str,
//         temperature: Option<f64>,
//         options: StreamOptions,
//     ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
//         self.as_ref()
//             .stream_chat_with_system(system_prompt, message, model, temperature, options)
//     }
//
//     fn stream_chat_with_history(
//         &self,
//         messages: &[ChatMessage],
//         model: &str,
//         temperature: Option<f64>,
//         options: StreamOptions,
//     ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
//         self.as_ref()
//             .stream_chat_with_history(messages, model, temperature, options)
//     }
//
//     fn stream_chat(
//         &self,
//         request: ChatRequest<'_>,
//         model: &str,
//         temperature: Option<f64>,
//         options: StreamOptions,
//     ) -> stream::BoxStream<'static, StreamResult<StreamEvent>> {
//         self.as_ref()
//             .stream_chat(request, model, temperature, options)
//     }
// }

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelte(StreamChunk),
    ToolCall(ToolCall),
    PreExecutedToolCall { name: String, args: String },
    PreExecutedToolResult { name: String, output: String },
    Usage(TokenUsage),
    Final,
}

impl StreamEvent {
    pub fn from_chunk(chunk: StreamChunk) -> Self {
        if chunk.is_final {
            Self::Final
        } else {
            Self::TextDelte(chunk)
        }
    }
}

pub fn build_tool_instructions_text(tools: &[ToolSpec]) -> String {
    let mut instructions = String::new();

    instructions.push_str("## Tool Use Protocol\n\n");
    instructions.push_str("To use a tool, wrap a JSON object in <tool_call></tool_call> tags:\n\n");
    instructions.push_str("<tool_call>\n");
    instructions.push_str(r#"{"name": "tool_name", "arguments": {"param": "value"}}"#);
    instructions.push_str("\n</tool_call>\n\n");
    instructions.push_str("You may use multiple tool calls in a single response. ");
    instructions.push_str("After tool execution, results appear in <tool_result> tags. ");
    instructions
        .push_str("Continue reasoning with the results until you can give a final answer.\n\n");
    instructions.push_str("### Available Tools\n\n");

    for tool in tools {
        writeln!(&mut instructions, "**{}**: {}", tool.name, tool.description)
            .expect("writing to String cannot fail");

        let parameters =
            serde_json::to_string(&tool.parameters).unwrap_or_else(|_| "{}".to_string());
        writeln!(&mut instructions, "Parameters: `{parameters}`")
            .expect("writing to String cannot fail");
        instructions.push('\n');
    }

    instructions
}
