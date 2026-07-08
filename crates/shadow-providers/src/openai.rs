//! OpenAI 兼容 provider -- 支持 OpenAI/OpenRouter/Ollama
//!
//! 实现 OpenAI Chat Completions API 的 tool calling 功能.
//! 将 agent-core 的 ToolSpec 转换为 API 格式, 解析响应中的 tool_calls.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::{Client, Error, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use shadow_core::{
    Attributable, AuthStyle, ChatResponse, ModelProvider, ModelProviderKind,
    ModelProviderRuntimeOptions, ProviderKind, Role, StreamChunk, TokenUsage,
};

use std::sync::{Arc, RwLock};

pub struct OpenAiCompatibleModelProvider {
    pub alias: String,
    pub name: String,
    pub base_url: String,
    pub credential: Option<String>,
    pub auth_header: AuthStyle,
    supports_vision: bool,
    native_tool_calling: bool,
    timeout_secs: u64,
}

impl OpenAiCompatibleModelProvider {
    pub fn new_with_vision(
        alias: &str,
        name: &str,
        base_url: &str,
        credential: Option<&str>,
        auth_style: AuthStyle,
        supports_vision: bool,
    ) -> Self {
        Self::new_with_opts(
            alias,
            name,
            base_url,
            credential,
            auth_style,
            supports_vision,
            None,
            false,
        )
    }

    /// 构造器 (带运行时选项) -- 支持 auth_style / timeout / extra_headers / api_path
    pub fn new_with_opts(
        alias: &str,
        name: &str,
        base_url: &str,
        credential: Option<&str>,
        auth_style: AuthStyle,
        supports_vision: bool,
        user_agent: Option<&str>,
        merge_system_into_user: bool,
    ) -> Self {
        Self {
            alias: alias.to_string(),
            name: name.to_string(),
            base_url: base_url.to_string(),
            credential: credential.map(ToString::to_string),
            auth_header: auth_style,
            supports_vision,
            native_tool_calling: true,
            timeout_secs: 60,
        }
    }
    pub fn without_native_tools(mut self) -> Self {
        self.native_tool_calling = false;
        self
    }

    pub fn chat_completions_url(&self) -> String {
        format!("{}/{}", self.base_url, "chat/completions")
    }

    pub fn apply_auth_header(
        &self,
        builder: RequestBuilder,
        credential: Option<&str>,
    ) -> RequestBuilder {
        apply_auth_to_request(builder, &self.auth_header, credential)
    }
    pub fn http_client(&self) -> Client {
        shadow_config::build_runtime_proxy_client_with_timeouts(
            "model_provider.compatible",
            self.timeout_secs,
            10,
        )
    }
}

fn apply_auth_to_request(
    builder: RequestBuilder,
    auth_style: &AuthStyle,
    credential: Option<&str>,
) -> RequestBuilder {
    let credential = match credential {
        None => return builder,
        Some(c) => c,
    };

    match auth_style {
        AuthStyle::Bearer => builder.bearer_auth(credential),
        AuthStyle::XApiKey => builder.header("x-api-key", credential),
        AuthStyle::Custom(header) => builder.header(header, credential),
    }
}

fn parse_chat_response_body(name: &str, body: &str) -> anyhow::Result<ApiResponse> {
    serde_json::from_str(body)
        .map_err(|_| anyhow::Error::msg(format!("{name} API returned an unexpected payload")))
}

impl Attributable for OpenAiCompatibleModelProvider {
    fn role(&self) -> Role {
        Role::Provider(ProviderKind::Model(ModelProviderKind::Custom))
    }
    fn alias(&self) -> &str {
        &self.alias
    }
}
//
// impl KeyRotator for OpenAiCompatibleModelProvider {
//     fn set_key(&self, key: Option<&str>) {
//         OpenAiCompatibleModelProvider::set_api_key(self, key.map(String::from));
//     }
// }

#[async_trait]
impl ModelProvider for OpenAiCompatibleModelProvider {
    fn supports_native_tools(&self) -> bool {
        true
    }

    /// chat_with_system -- OpenAI Chat Completions API 单轮调用
    ///
    /// 发送 system + user 消息到 /chat/completions, 返回文本响应.
    /// content 为空时退化到 reasoning_content (思考模型).
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        // 构建消息列表
        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(ApiMessage {
                role: "system".to_string(),
                content: Some(sys.to_string()),
                tool_calls: None,
                reasoning_content: None,
            });
        }
        messages.push(ApiMessage {
            role: "user".to_string(),
            content: Some(message.to_string()),
            tool_calls: None,
            reasoning_content: None,
        });

        let body = ChatRequest {
            model: model.to_string(),
            messages,
            temperature,
            tools: None,
            stream: false,
        };

        let url = self.chat_completions_url();

        let resp = match self
            .apply_auth_header(
                self.http_client().post(&url).json(&body),
                self.credential.as_deref(),
            )
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => return Err(e.into()),
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let error = resp.text().await?;
            anyhow::bail!("{} API error {status}: {error}", self.name);
        }

        let body = resp.text().await?;

        let chat_resp = parse_chat_response_body(&self.name, &body)?;
        chat_resp
            .choices
            .into_iter()
            .next()
            .map(|c| {
                if c.message.tool_calls.is_some()
                    && c.message.tool_calls.as_ref().is_some_and(|t| !t.is_empty())
                {
                    serde_json::to_string(&c.message)
                        .unwrap_or_else(|_| c.message.effective_content())
                } else {
                    c.message.effective_content()
                }
            })
            .ok_or_else(|| anyhow::Error::msg(format!("{} no response", self.name)))
    }
}

// ── API 类型 (OpenAI Chat Completions 格式) ──

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ApiMessage>,
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ApiToolCall>>,
    /// 思考模型要求 assistant tool-call 历史消息回传 reasoning_content
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
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
    choices: Vec<Choice>,
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(from = "RawResponseMessage")]
struct ResponseMessage {
    content: Option<String>,
    /// 思考模型 (DeepSeek-R1 等) 返回的推理内容, 与 content 分离
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

impl ResponseMessage {
    fn effective_content(&self) -> String {
        self.content.as_ref().map(|c| strip_thing_tags(c))
            .filter(|c| !c.is_empty()).unwrap_or_default()
    }

    fn effective_content_options(&self) -> Option<String> {
        self.content.as_ref().map(|c| strip_thing_tags(c))
            .filter(|c| !c.is_empty())
    }
}

fn strip_thing_tags(content: &str) -> String {
    let mut result = String::with_capacity(content.len());

    let mut rest = content;
    loop {
        if let Some(start) = rest.find("<think>") {
            result.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</think>"){
                rest = &rest[start + end + "</think>".len()..];
            }else{
                break;
            }
        }else{
            result.push_str(rest);
            break;
        }
    }
    result.trim().to_string()
}


#[derive(Debug, Deserialize)]
struct RawResponseMessage {
    content: Option<OpenAiAssistantContent>,
    reasoning_content: Option<String>,
    reasoning: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiAssistantContent {
    Text(String),
    Parts(Vec<OpenAiAssistantContentPart>),
}
#[derive(Debug, Deserialize)]
struct OpenAiAssistantContentPart {
    #[serde(rename = "type")]
    kind: Option<String>,

    text: Option<String>,
}

impl From<RawResponseMessage> for ResponseMessage {
    fn from(raw: RawResponseMessage) -> Self {
        let reasoning_content = raw.reasoning_content.or(raw.reasoning);
        Self {
            content: openai_assistant_content_plaintext(raw.content),
            reasoning_content,
            tool_calls: raw.tool_calls,
        }
    }
}

fn openai_assistant_content_plaintext(content: Option<OpenAiAssistantContent>) -> Option<String> {
    match content? {
        OpenAiAssistantContent::Text(t) => {
            if t.is_empty() { None } else { Some(t) }
        }
        OpenAiAssistantContent::Parts(parts) => {
            let mut text = String::new();
            for p in parts {
                if p.kind.as_deref() != Some("text") {
                    continue;
                }
                let Some(pt) = p.text.filter(|text | !text.is_empty()) else { continue; };
                if !text.is_empty() { text.push('\n'); }
                text.push_str(&pt);
            }
            if text.is_empty() { None } else { Some(text) }
        }
    }
}

/// 响应中的工具调用
#[derive(Debug, Serialize, Deserialize)]
struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function: Option<Function>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra_content: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Function {
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

// ── 辅助函数和类型 ──

/// 工具调用累积器 -- 流式响应中按 index 分组累积 tool_call 的 fragments
#[derive(Default)]
struct StreamToolCallAccumulator {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    extra_content: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallDelta{
    index: Option<usize>,
    id: Option<String>,
    function: Option<StreamFunctionDelta>,
    name: Option<String>,
    arguments: Option<String>,
    extra_content: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize)]
struct StreamFunctionDelta{
    name: Option<String>,
    arguments: Option<String>,
}

impl StreamToolCallAccumulator {
    
}
