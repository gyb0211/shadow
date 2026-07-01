//! OpenAI 兼容 provider -- 支持 OpenAI/OpenRouter/Ollama
//!
//! 实现 OpenAI Chat Completions API 的 tool calling 功能.
//! 将 agent-core 的 ToolSpec 转换为 API 格式, 解析响应中的 tool_calls.

use shadow_core::{
    Attributable, ChatRequest, ChatResponse, Provider, Role, TokenUsage, ToolCall,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct OpenAiProvider {
    provider_type: String,
    api_key: Option<String>,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(provider_type: &str, api_key: Option<&str>, base_url: Option<&str>) -> Result<Self> {
        let default_url = match provider_type {
            "openai" => "https://api.openai.com/v1",
            "openrouter" => "https://openrouter.ai/api/v1",
            "ollama" => "http://localhost:11434/v1",
            _ => "https://api.openai.com/v1",
        };
        Ok(Self {
            provider_type: provider_type.to_string(),
            api_key: api_key.map(String::from),
            base_url: base_url.unwrap_or(default_url).to_string(),
        })
    }

    fn client(&self) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder();
        if let Some(ref key) = self.api_key {
            builder = builder.default_headers(
                reqwest::header::HeaderMap::from_iter([(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
                        .context("无效的 API key")?,
                )]),
            );
        }
        builder.build().context("创建 HTTP 客户端失败")
    }
}

impl Attributable for OpenAiProvider {
    fn role(&self) -> Role {
        Role::Provider
    }
    fn alias(&self) -> &str {
        &self.provider_type
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn provider_type(&self) -> &str {
        &self.provider_type
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let client = self.client()?;
        let url = format!("{}/chat/completions", self.base_url);

        // 转换消息: ChatMessage -> ApiMessage
        let messages: Vec<ApiMessage> = request
            .messages
            .iter()
            .map(|m| {
                let tool_calls: Option<Vec<ApiToolCall>> = if m.tool_calls.is_empty() {
                    None
                } else {
                    Some(
                        m.tool_calls
                            .iter()
                            .map(|tc| ApiToolCall {
                                id: tc.id.clone(),
                                call_type: "function".to_string(),
                                function: ApiFunction {
                                    name: tc.name.clone(),
                                    arguments: serde_json::to_string(&tc.arguments)
                                        .unwrap_or_default(),
                                },
                            })
                            .collect(),
                    )
                };

                // content 为空且有 tool_calls 时, API 期望 content 为 null
                let content = if m.content.is_empty() && tool_calls.is_some() {
                    None
                } else {
                    Some(m.content.clone())
                };

                ApiMessage {
                    role: m.role.clone(),
                    content,
                    tool_call_id: m.tool_call_id.clone(),
                    tool_calls,
                }
            })
            .collect();

        // 转换工具规格: ToolSpec -> ApiTool
        let tools: Vec<ApiTool> = request
            .tools
            .iter()
            .map(|t| ApiTool {
                tool_type: "function".to_string(),
                function: ApiToolSpec {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect();

        let body = ApiRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            tools: if tools.is_empty() { None } else { Some(tools) },
        };

        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("LLM 请求失败")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM 返回错误 {status}: {text}");
        }

        let api_resp: ApiResponse = resp
            .json()
            .await
            .context("解析 LLM 响应失败")?;

        let choice = api_resp.choices.first().context("LLM 响应无 choices")?;
        let content = choice.message.content.clone().unwrap_or_default();

        // 解析 tool_calls
        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .as_ref()
            .map(|tcs| {
                tcs.iter()
                    .map(|tc| {
                        let args: Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(Value::Null);
                        ToolCall {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            arguments: args,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let usage = TokenUsage {
            prompt_tokens: api_resp.usage.prompt_tokens,
            completion_tokens: api_resp.usage.completion_tokens,
            total_tokens: api_resp.usage.total_tokens,
        };

        Ok(ChatResponse {
            content,
            tool_calls,
            usage,
        })
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let client = self.client()?;
        let url = format!("{}/models", self.base_url);
        let resp: ModelsResponse = client.get(&url).send().await?.json().await?;
        Ok(resp.data.into_iter().map(|m| m.id).collect())
    }
}

// ── API 类型 (OpenAI Chat Completions 格式) ──

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
}

#[derive(Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ApiToolCall>>,
}

/// 请求中的工具调用 (assistant 消息携带)
#[derive(Serialize, Deserialize)]
struct ApiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ApiFunction,
}

#[derive(Serialize, Deserialize)]
struct ApiFunction {
    name: String,
    arguments: String,
}

/// 请求中的工具定义
#[derive(Serialize)]
struct ApiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: ApiToolSpec,
}

#[derive(Serialize)]
struct ApiToolSpec {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Deserialize)]
struct ApiResponse {
    choices: Vec<ApiChoice>,
    usage: ApiUsage,
}

#[derive(Deserialize)]
struct ApiChoice {
    message: ApiChoiceMessage,
}

#[derive(Deserialize)]
struct ApiChoiceMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ApiToolCallResponse>>,
}

/// 响应中的工具调用
#[derive(Deserialize)]
struct ApiToolCallResponse {
    id: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    call_type: String,
    function: ApiFunctionResponse,
}

#[derive(Deserialize)]
struct ApiFunctionResponse {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct ApiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ApiModel>,
}

#[derive(Deserialize)]
struct ApiModel {
    id: String,
}
