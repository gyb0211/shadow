//! 模型提供商 trait -- LLM 推理后端抽象

use crate::attribution::Attributable;
use crate::attribution::Role;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 聊天消息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,      // "system" / "user" / "assistant" / "tool"
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
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub model: String,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<crate::tool::ToolSpec>,
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

/// 模型提供商 trait
///
/// 每个 LLM 后端实现此 trait (OpenAI/Anthropic/Ollama...)
/// 通过工厂函数按字符串 key 注册。
/// 借鉴 ZeroClaw ModelProvider, 重命名自 agent-core。
#[async_trait]
pub trait Provider: Attributable {
    /// 提供商类型名 (如 "openai", "anthropic")
    fn provider_type(&self) -> &str;

    /// 同步聊天
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;

    /// 列出可用模型
    async fn list_models(&self) -> Result<Vec<String>>;

    /// 是否支持原生工具调用
    fn supports_native_tools(&self) -> bool {
        false
    }

    /// 默认温度
    fn default_temperature(&self) -> f64 {
        0.7
    }
}

/// 默认提供商 -- 用于未配置时的占位
pub struct DefaultProvider {
    name: String,
}

impl DefaultProvider {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string() }
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
impl Provider for DefaultProvider {
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
