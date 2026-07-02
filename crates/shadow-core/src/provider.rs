//! 模型提供商 trait -- LLM 推理后端抽象

use crate::attribution::Attributable;
use anyhow::Result;
use async_trait::async_trait;
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::ToolSpec;

/// 流式聊天块 -- SSE 增量
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// 文本增量 (assistant 回复的逐字/逐词片段)
    TextDelta(StreamChunk),
    /// 工具调用增量 (arguments 可能分多次到达, 按 index 分组累积)
    ToolCallDelta(ToolCall),
    PreExecutedToolCall {
        name: String,
        output: String,
    },
    PreExecutedToolResult {
        name: String,
        output: String,
    },
    Usage(TokenUsage),
    Final,
}

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

    pub fn with_count_tokens(mut self) -> Self {
        self.count_tokens = true;
        self
    }
}

pub type StreamResult<T> = std::result::Result<T, StreamError>;

/// 流式错误 -- 当前为空枚举 (所有错误通过 StreamEvent::TextDelta(is_final) 传递)
///
/// 实现 Error/Display 以便消费者用 `?` 转换到 anyhow::Error. 由于无变体,
/// 实际不可构造, 所有方法体都是 `match self {}` (exhaustive on empty enum).
#[derive(Debug)]
pub enum StreamError {}

impl std::fmt::Display for StreamError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不可构造 (空枚举), 用 *self 解引用后 exhaustive match
        match *self {}
    }
}

impl std::error::Error for StreamError {}

/// 聊天消息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "system" / "user" / "assistant" / "tool"
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// assistant 消息携带的工具调用 (发给 LLM 时序列化)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// 思考模型的原始推理内容 (DeepSeek-R1, GLM-4.7 等)
    /// 从 provider 响应的 reasoning_content 字段解析; 回传给 API 时原样发送,
    /// 因为部分 provider 拒绝缺少此字段的 tool-call 历史.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

/// 聊天请求
#[derive(Debug, Clone)]
pub struct ChatRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub tools: Option<&'a [ToolSpec]>
}

/// 聊天响应
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
    /// 思考模型的原始推理内容 (DeepSeek-R1 等 API 返回的 reasoning_content 字段)
    /// 与 content 分离, 不直接显示给用户, 但回传时需要带上.
    pub reasoning_content: Option<String>,
}

/// 工具调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub extra_content: Option<serde_json::Value>,
}

/// Token 用量
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
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
    Query(String),
}

/// ModelProvider 运行时选项 -- 注入 HTTP 层细节
///
/// 由 factory (shadow-providers::create_provider) 接收, 透传给 Compat 层.
/// 设计为 Option-heavy: MVP 阶段大部分字段为 None, 未来 Reliable 层 / 推理控制会填充.
///
/// 注: `extra_headers` 用 `HashMap<String, String>` 而非 `reqwest::HeaderMap`,
/// 是为了让 shadow-core 保持 HTTP-agnostic (不依赖 reqwest). shadow-providers
/// 在调用 reqwest 时做一次转换.
#[derive(Debug, Clone, Default)]
pub struct ModelProviderRuntimeOptions {
    /// HTTP 请求超时 (None = reqwest 默认)
    pub timeout: Option<std::time::Duration>,
    /// 推理强度 (如 "low" / "medium" / "high"), OpenAI o-series / Anthropic 用
    pub reasoning_effort: Option<String>,
    /// 自定义 API path 后缀 (None = 各 family 默认, 如 "/chat/completions")
    pub api_path: Option<String>,
    /// 附加 HTTP headers (会与 auth header 合并)
    pub extra_headers: HashMap<String, String>,
    /// API key 注入位置
    pub auth_style: AuthStyle,
}

/// 模型提供商 trait
///
/// 每个 LLM 后端实现此 trait (OpenAI/Anthropic/Ollama...)
/// 通过工厂函数按字符串 key 注册。
/// 借鉴 ZeroClaw ModelProvider, 重命名自 agent-core。
#[async_trait]
pub trait ModelProvider: Attributable {
    /// ⭐ 唯一必需实现的方法
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String>;

    async fn simple_chat(
        &self,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        self.chat_with_system(None, message, model, temperature)
            .await
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
        let last_user = messages
            .iter()
            .rfind(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        self.chat_with_system(system, last_user, model, temperature)
            .await
    }

    /// 同步聊天
    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse>;

    /// 同步聊天 + 工具调用
    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse>;

    /// 流式聊天 -- 返回 BoxStream, 逐块推送 ChatChunk
    ///
    /// 默认实现: 调用 chat() 获取完整响应, 包装成单个 Done chunk.
    /// 支持 SSE 的 provider 应覆写此方法.
    fn stream_chat(
        &self,
        _request: ChatRequest,
        _model: &str,
        _temperature: Option<f64>,
        _options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamEvent>> {
        // 默认不支持
        stream::empty().boxed()
    }

    fn stream_chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
        _options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamEvent>> {
        // 默认不支持
        stream::empty().boxed()
    }

    fn stream_chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamEvent>> {
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.as_str());
        let last_user = messages
            .iter()
            .rfind(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        self.stream_chat_with_system(system, last_user, model, temperature, options)
    }

    /// 列出可用模型
    ///
    /// 默认实现返回空 vec (provider 不支持 list models 时无需覆写).
    async fn list_models(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }

    /// 是否支持原生工具调用
    fn supports_native_tools(&self) -> bool {
        false
    }

    /// 是否支持视觉输入 (vision/image)
    fn supports_vision(&self) -> bool {
        false
    }

    /// 默认温度
    fn default_temperature(&self) -> f64 {
        0.7
    }
}
