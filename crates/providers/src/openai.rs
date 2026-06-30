//! OpenAI 兼容 provider -- 支持 OpenAI/OpenRouter/Ollama

use agent_core::{
    Attributable, ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role, TokenUsage, ToolCall,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
impl ModelProvider for OpenAiProvider {
    fn provider_type(&self) -> &str {
        &self.provider_type
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let client = self.client()?;
        let url = format!("{}/chat/completions", self.base_url);

        let messages: Vec<ApiMessage> = request
            .messages
            .iter()
            .map(|m| ApiMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let body = ApiRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
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
        let usage = TokenUsage {
            prompt_tokens: api_resp.usage.prompt_tokens,
            completion_tokens: api_resp.usage.completion_tokens,
            total_tokens: api_resp.usage.total_tokens,
        };

        Ok(ChatResponse {
            content,
            tool_calls: vec![],
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

// ── API 类型 ──

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    temperature: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
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
