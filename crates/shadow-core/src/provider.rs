//! 模型提供商 trait -- LLM 推理后端抽象

use crate::attribution::Attributable;
use crate::attribution::Role;
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::ToolSpec;

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
    pub messages: Vec<ChatMessage>,
    pub model: String,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub tools: Option<&'a [ToolSpec]>,
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
    pub arguments: serde_json::Value,
}

/// Token 用量
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
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

/// Provider 运行时选项 -- 注入 HTTP 层细节
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

/// 流式聊天块 -- SSE 增量
///
/// ModelProvider::chat_stream() 返回 BoxStream<Result<ChatChunk>>,
/// 调用方逐块消费, 实现逐字/逐词显示.
#[derive(Debug, Clone)]
pub enum ChatChunk {
    /// 文本增量 (assistant 回复的逐字/逐词片段)
    ContentDelta(String),
    /// 思考内容增量 (reasoning_content 的流式片段)
    ReasoningDelta(String),
    /// 工具调用增量 (arguments 可能分多次到达, 按 index 分组累积)
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_fragment: String,
    },
    /// 流结束 -- 包含完整累积结果
    Done {
        content: String,
        tool_calls: Vec<ToolCall>,
        usage: TokenUsage,
        reasoning_content: Option<String>,
    },
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

const BASE_TEMPERATURE: f64 = 0.7;
const BASE_MAX_TOKEN: u32 = 4096;
const BASE_TIMEOUT_SECS: u32 = 120;
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

    fn default_timeout_secs(&self) -> u32 {
        BASE_TIMEOUT_SECS
    }
    fn default_wire_api(&self) -> &str {
        BASE_WIRE_API
    }

    fn supports_native_tools(&self) -> bool{
        self.capabilities().native_tool_calling
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
            .map(|id| ModelInfo { id, pricing: None }).collect())

    }


    async fn chat_with_history(&self, messages:&[ChatMessage], model:&str, temperature: Option<f64>) -> Result<String>{
        let system = messages.iter().find(|m| m.role == "system").map(|m| m.content.as_str());
        let user = messages.iter().rfind(|m| m.role == "user").map(|m| m.content.as_str()).unwrap_or("");
        self.chat_with_system(system, user, model, temperature).await
    }

    async fn chat(&self, request: ChatRequest<'_>, model: &str, temperature:Option<f64>) -> Result<ChatResponse>{
        if let Some(tools) = request.tools && !tools.is_empty() && self.supports_native_tools() {
            let tool_instructions = match self.convert_tools(tools) {

            };



        }
        Ok(ChatResponse{
            content: "".to_string(),
            tool_calls: vec![],
            usage: Default::default(),
            reasoning_content: None,
        })
    }


}

/// 默认提供商 -- 用于未配置时的占位
pub struct DefaultProvider {
    name: String,
}

impl DefaultProvider {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

impl Attributable for DefaultProvider {
    fn role(&self) -> Role {
        Role::Provider
    }
    fn alias(&self) -> &str {
        &self.name
    }
}

#[async_trait]
impl ModelProvider for DefaultProvider {
    fn provider_type(&self) -> &str {
        "default"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        anyhow::bail!("默认提供商未配置, 请通过 `shadow config set` 配置 LLM 提供商")
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
}
