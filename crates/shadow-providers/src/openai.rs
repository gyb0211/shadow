//! OpenAI 兼容 provider -- 支持 OpenAI/OpenRouter/Ollama
//!
//! 实现 OpenAI Chat Completions API 的 tool calling 功能.
//! 将 agent-core 的 ToolSpec 转换为 API 格式, 解析响应中的 tool_calls.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::{Client, Error, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Deserializer, Value};
use shadow_core::kennel::provider::{
    ChatResponse as ProviderChatResponse, ToolCall as ProviderToolCall,
};
use shadow_core::{
    Attributable, AuthStyle, ChatMessage, ChatResponse, ModelProvider, ModelProviderKind,
    ProviderCapabilities, ProviderKind, Role, StreamChunk, TokenUsage,
};
use std::collections::{HashMap, HashSet};

use crate::{models_dev, non_empty_string_field};
use shadow_config::model_provider;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: MessageContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Parts(Vec<MessagePart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MessagePart {
    Text { text: String },
    ImageUrl { image_url: ImageUrlPart },
}

#[derive(Debug, Serialize)]
struct ImageUrlPart {
    url: String,
}

pub struct OpenAiCompatibleModelProvider {
    pub alias: String,
    pub name: String,
    pub base_url: String,
    pub credential: Option<String>,
    pub auth_header: AuthStyle,
    supports_vision: bool,
    native_tool_calling: bool,
    timeout_secs: u64,

    replay_assistant_reasoning: bool,
    reasoning_effort: Option<String>,
    max_tokens: Option<u32>,
    merge_system_into_user: bool,

    model_dev_key: Option<String>,
    extra_body: Option<Value>,
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
            replay_assistant_reasoning: false,
            reasoning_effort: None,
            max_tokens: None,
            merge_system_into_user,
            model_dev_key: None,
            extra_body: None,
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

    /// 打薄系统消息
    /// 严格兼容OpenAI格式 要求的 只能以一个 system 开头的历史消息
    /// 1. 多个 system 合并成一个
    /// 2. 没有system或者为空 原样返回（过滤掉空的 len=0）
    /// 3. 不需要合并的 就把合并后的system放到第一个
    /// 4. 需要合并的 且 有用户消息的 第一条User.content前 插入system_content
    /// 5. 要合并 且没有用户消息的 system_content 作为第一个user msg
    ///
    fn flatten_system_messages(messages: &[ChatMessage], merge: bool) -> Vec<ChatMessage> {
        let mut saw_system = false;
        let mut system_content = String::new();
        let mut result = Vec::with_capacity(messages.len());
        for message in messages {
            if message.is_system() {
                saw_system = true;
                if !message.content.is_empty() {
                    if !system_content.is_empty() {
                        system_content.push_str("\n\n");
                    }
                    system_content.push_str(&message.content);
                }
            } else {
                result.push(message.clone());
            }
        }

        if !saw_system {
            return messages.to_vec();
        }

        if system_content.is_empty() {
            return result;
        }

        if !merge {
            result.insert(0, ChatMessage::system(system_content));
            return result;
        }

        if let Some(first_user) = result.iter_mut().find(|m| m.is_user()) {
            if !system_content.is_empty() {
                first_user.content = format!("{system_content}\n\n{}", first_user.content);
            }
        } else {
            result.insert(0, ChatMessage::user(&system_content));
        }

        result
    }

    fn strip_native_tool_messages(&self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        if self.native_tool_calling {
            return messages.to_vec();
        }

        let intermediate = messages.iter().enumerate().filter_map(|(idx, msg)| {
            // todo 丢弃被标记过的消息

            if msg.is_tool() {
                return None;
            }
            if msg.is_assistant()
                && let Ok(value) = serde_json::from_str::<Value>(&msg.content)
                && value.get("tool_calls").is_some()
            {
                let text = value
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                return if text.is_empty() {
                    None
                } else {
                    Some(ChatMessage::assistant(&text))
                };
            }
            Some(msg.clone())
        });

        let mut coalesced: Vec<ChatMessage> = Vec::with_capacity(messages.len());
        for msg in intermediate {
            match coalesced.last_mut() {
                Some(last) if last.role == msg.role && !msg.is_system() => {
                    if !last.content.is_empty() && !msg.content.is_empty() {
                        last.content.push_str("\n\n");
                    }
                    last.content.push_str(&msg.content)
                }
                _ => coalesced.push(msg),
            }
        }

        coalesced
    }

    fn assistant_reasoning_value(value: &Value) -> Option<&str> {
        value
            .get("reasoning_content")
            .and_then(Value::as_str)
            .or_else(|| value.get("reasoning").and_then(Value::as_str))
    }

    fn assistant_reasoning_pair_for_replay(
        &self,
        value: &Value,
    ) -> (Option<String>, Option<String>) {
        if !self.replay_assistant_reasoning {
            return (None, None);
        }
        let reasoning_content = value
            .get("reasoning_content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);

        let reasoning = value
            .get("reasoning")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);

        (reasoning_content, reasoning)
    }

    fn targets_mistral_tool_call_contract(&self) -> bool {
        if self.name.eq_ignore_ascii_case("mistral") {
            return true;
        }
        reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(|h| h.to_ascii_lowercase()))
            .is_some_and(|h| h == "mistral.ai" || h.ends_with(".mistral.ai"))
    }

    fn convert_messages_for_native(
        &self,
        messages: &[ChatMessage],
        allow_user_image_parts: bool,
    ) -> Vec<NativeMessage> {
        let mistral_tool_call = self.targets_mistral_tool_call_contract();
        let mut used_tool_call_ids = HashSet::new();
        let mut tool_call_id_map = HashMap::new();
        let mut last_assistant_tool_call_ids: Vec<String> = Vec::new();
        let mut tool_name_map = HashMap::new();

        messages
            .iter()
            .map(|m| {
                if m.is_assistant()
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content)
                    && let Some(tool_calls_value) = value.get("tool_calls")
                    && let Ok(parsed_calls) =
                        serde_json::from_value::<Vec<ProviderToolCall>>(tool_calls_value.clone())
                {
                    let tool_calls = parsed_calls
                        .into_iter()
                        .map(|tc| {
                            let tc_id = tc.id.clone();
                            let tc_name = tc.name.clone();
                            tool_name_map.insert(tc_id, tc_name);
                            ToolCall {
                                id: Some({
                                    let normalized_id = reserve_tool_call_id_for_contract(
                                        mistral_tool_call,
                                        Some(tc.id.clone()),
                                        &mut used_tool_call_ids,
                                    );
                                    tool_call_id_map.insert(tc.id.clone(), normalized_id.clone());
                                    normalized_id
                                }),
                                kind: Some("function".to_string()),
                                function: Some(Function {
                                    name: Some(tc.name),
                                    arguments: Some(tc.arguments),
                                }),
                                name: None,
                                arguments: None,
                                parameters: None,
                                extra_content: tc.extra_content,
                            }
                        })
                        .collect::<Vec<_>>();

                    last_assistant_tool_call_ids =
                        tool_calls.iter().filter_map(|tc| tc.id.clone()).collect();

                    let content =
                        non_empty_string_field(&value, "content").map(MessageContent::Text);

                    let (reasoning_content, reasoning) =
                        self.assistant_reasoning_pair_for_replay(&value);
                    return NativeMessage {
                        role: "assistant".to_string(),
                        content,
                        tool_call_id: None,
                        tool_calls: Some(tool_calls),
                        reasoning_content,
                        reasoning,
                        name: None,
                    };
                }

                if m.is_assistant()
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content)
                    && value.get("tool_calls").is_none()
                    && Self::assistant_reasoning_value(&value).is_some()
                    && matches!(
                        value.get("content"),
                        None | Some(serde_json::Value::Null | serde_json::Value::String(_))
                    )
                {
                    let content = value
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .map(|v| MessageContent::Text(v.to_string()));

                    let (reasoning_content, reasoning) =
                        self.assistant_reasoning_pair_for_replay(&value);
                    return NativeMessage {
                        role: "assistant".to_string(),
                        content,
                        tool_call_id: None,
                        tool_calls: None,
                        reasoning_content,
                        reasoning,
                        name: None,
                    };
                }

                if m.is_tool()
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content)
                {
                    let mut tool_call_id = value
                        .get("tool_call_id")
                        .and_then(serde_json::Value::as_str)
                        .map(|id| {
                            tool_call_id_map.get(id).cloned().unwrap_or_else(|| {
                                let normalized_id = reserve_tool_call_id_for_contract(
                                    mistral_tool_call,
                                    Some(id.to_string()),
                                    &mut used_tool_call_ids,
                                );
                                tool_call_id_map.insert(id.to_string(), normalized_id.clone());
                                normalized_id
                            })
                        });

                    if tool_call_id.is_none() && !last_assistant_tool_call_ids.is_empty() {
                        tool_call_id = last_assistant_tool_call_ids.first().cloned();
                    }

                    let content = value
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .map(|v| {
                            if allow_user_image_parts {
                                Self::content_with_image_parts(v)
                            } else {
                                MessageContent::Text(v.to_string())
                            }
                        })
                        .or_else(|| Some(MessageContent::Text(m.content.clone())));

                    let tool_name = value
                        .get("tool_call_id")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|id| tool_name_map.get(id).cloned())
                        .or_else(|| {
                            value
                                .get("name")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string)
                        });
                    return NativeMessage {
                        role: "tool".to_string(),
                        content,
                        tool_call_id,
                        tool_calls: None,
                        reasoning_content: None,
                        reasoning: None,
                        name: tool_name,
                    };
                }

                NativeMessage {
                    role: m.role.to_string(),
                    content: Some(Self::to_message_content(
                        &m,
                        &m.content,
                        allow_user_image_parts,
                    )),
                    tool_call_id: None,
                    tool_calls: None,
                    reasoning_content: None,
                    reasoning: None,
                    name: None,
                }
            })
            .collect()
    }

    fn reasoning_effort_for_model(&self, model: &str) -> Option<String> {
        let effort = self.reasoning_effort.as_ref()?;
        let id = model
            .rsplit('/')
            .next()
            .unwrap_or(model)
            .to_ascii_lowercase();

        let is_gpt5_chat_latest = id.starts_with("gpt-5") && id.ends_with("-chat-latest");
        let is_openai_reasoning_model = id == "o1"
            || id.starts_with("o1-")
            || id == "o3"
            || id.starts_with("o3-")
            || id == "o4"
            || id.starts_with("o4-")
            || (id.starts_with("gpt-5") && !is_gpt5_chat_latest);
        let is_likely_codex_supported = id.contains("codex") && id.starts_with("gpt-");

        (is_openai_reasoning_model || is_likely_codex_supported).then(|| effort.clone())
    }

    fn requires_tool_stream(&self) -> bool {
        let host_requires_tool_stream = reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|h| h == "api.z.ai" || h.ends_with(".z.ai"));
        host_requires_tool_stream || matches!(self.name.as_str(), "zai" | "z.ai")
    }

    fn tool_stream_for_tools(&self, has_tools: bool) -> Option<bool> {
        if has_tools && self.requires_tool_stream() {
            Some(true)
        } else {
            None
        }
    }

    fn build_native_tool_chat_request(
        &self,
        messages: &[ChatMessage],
        tools: Option<Vec<serde_json::Value>>,
        model: &str,
        temperature: Option<f64>,
        allow_user_image_parts: bool,
    ) -> NativeChatRequest {
        let has_tool_entries = tools.as_ref().is_some_and(|t| !t.is_empty());
        let tool_choice = has_tool_entries.then(|| "auto".to_string());
        NativeChatRequest {
            model: model.to_string(),
            messages: self.convert_messages_for_native(messages, allow_user_image_parts),
            temperature,
            stream: Some(false),
            stream_options: None,
            reasoning_effort: self.reasoning_effort_for_model(model),
            tool_stream: self.tool_stream_for_tools(has_tool_entries),
            tools,
            tool_choice,
            max_tokens: self.max_tokens,
            extra_body: self.extra_body.clone(),
        }
    }

    fn to_message_content(m: &ChatMessage, content: &str, merge: bool) -> MessageContent {
        if !m.is_user() || !merge {
            return MessageContent::Text(content.to_string());
        }

        Self::content_with_image_parts(content)
    }

    fn effective_merge_system(&self, model: &str) -> bool {
        self.merge_system_into_user
        // || Self::model_requires_system_merge(model)
    }

    fn content_with_image_parts(content: &str) -> MessageContent {
        let mut parts = Vec::with_capacity(1);
        parts.push(MessagePart::Text {
            text: content.to_string(),
        });
        MessageContent::Parts(parts)
    }

    fn reserve_tool_call_id(
        &self,
        tool_id: Option<String>,
        used_ids: &mut HashSet<String>,
    ) -> String {
        reserve_tool_call_id_for_contract(
            self.targets_mistral_tool_call_contract(),
            tool_id,
            used_ids,
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
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: self.native_tool_calling,
            vision: self.supports_vision,
            prompt_caching: false,
            extended_thinking: false,
        }
    }

    fn supports_native_tools(&self) -> bool {
        self.native_tool_calling
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        if let Some(key) = &self.model_dev_key {
            match models_dev::list_models_for(key).await {
                Ok(models) if !models.is_empty() => return Ok(models),
                Ok(_) => {}
                Err(e) => {}
            }
        }
        anyhow::bail!("live model listing is not supported for this model_provider")
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

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        let normalized = Vec::from(messages);
        let merge = self.effective_merge_system(model);
        let eff_msg = Self::flatten_system_messages(&normalized, merge);

        let eff_msg = self.strip_native_tool_messages(&eff_msg);

        let api_messages: Vec<Message> = eff_msg
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: Self::to_message_content(&m, &m.content, !merge),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature,
            stream: Some(false),
            stream_options: None,
            reasoning_effort: self.reasoning_effort.clone(),
            tool_stream: None,
            tools: None,
            tool_choice: None,
            max_tokens: self.max_tokens,
        };

        let url = self.chat_completions_url();
        let response = match self
            .apply_auth_header(
                self.http_client().post(&url).json(&request),
                self.credential.as_deref(),
            )
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => return Err(err.into()),
        };
        println!("request: {:?}, response: {:?}", request, response);
        if !response.status().is_success() {
            return Err(anyhow::Error::msg(format!("API error")));
        }

        let body = response.text().await?;
        let chat_resp = parse_chat_response_body(&self.name, &body)?;
        chat_resp
            .choices
            .into_iter()
            .next()
            .map(|c| {
                if c.message.tool_calls.is_some()
                    && c.message.tool_calls.as_ref().is_some_and(|t| t.is_empty())
                {
                    serde_json::to_string(&c.message)
                        .unwrap_or_else(|_| c.message.effective_content())
                } else {
                    c.message.effective_content()
                }
            })
            .ok_or_else(|| {
                // todo log

                anyhow::Error::msg(format!("No Response from {}", self.name))
            })
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        let normalized = Vec::from(messages);
        let merge = self.effective_merge_system(model);
        let eff_msg = Self::flatten_system_messages(&normalized, merge);

        let eff_msg = self.strip_native_tool_messages(&eff_msg);

        let tools = if tools.is_empty() {
            None
        } else {
            Some(tools.to_vec())
        };

        let request =
            self.build_native_tool_chat_request(&eff_msg, tools, model, temperature, !merge);

        let url = self.chat_completions_url();
        let response = match self
            .apply_auth_header(
                self.http_client().post(&url).json(&request),
                self.credential.as_deref(),
            )
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_outcome(shadow_log::EventOutcome::Unknown),
                    &format!(
                        "{} native tool transport failed: {err}; failing back to history path",
                        self.name
                    )
                );
                let text = self.chat_with_history(messages, model, temperature).await?;
                return Ok(ProviderChatResponse {
                    text: Some(text),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                });
            }
        };
        if !response.status().is_success() {
            return Err(anyhow::Error::msg(format!("API error")));
        }

        let body = response.text().await?;
        let chat_resp = parse_chat_response_body(&self.name, &body)?;

        let usage = chat_resp.usage.map(UsageInfo::into_provider_usage);

        let choice = chat_resp.choices.into_iter().next().ok_or_else(|| {
            shadow_log::record!(
                ERROR,
                shadow_log::Event::new(module_path!(), shadow_log::Action::Fail)
                    .with_outcome(shadow_log::EventOutcome::Failure)
                    .with_attrs(serde_json::json!({"model_provider": &self.name})),
                "compatible: empty choices in response"
            );
            anyhow::Error::msg(format!("No response from {}", self.name))
        })?;

        let text = choice.message.effective_content_optional();
        let reasoning_content = choice.message.reasoning_content;
        let mut used_tool_call_ids = HashSet::new();
        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tc| {
                let func = tc.function?;
                let name = func.name?;
                let arguments = func.arguments.unwrap_or_else(|| "{}".to_string());
                Some(ProviderToolCall {
                    id: self.reserve_tool_call_id(tc.id, &mut used_tool_call_ids),
                    name,
                    arguments,
                    extra_content: tc.extra_content,
                })
            })
            .collect::<Vec<_>>();

        Ok(ProviderChatResponse {
            text,
            tool_calls,
            usage,
            reasoning_content,
        })
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

#[derive(Debug, Serialize)]
struct ApiChatRequest {
    model: String,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptionsBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct StreamOptionsBody {
    include_usage: bool,
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
    usage: Option<UsageInfo>,
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
        self.content
            .as_ref()
            .map(|c| strip_thing_tags(c))
            .filter(|c| !c.is_empty())
            .unwrap_or_default()
    }

    fn effective_content_optional(&self) -> Option<String> {
        self.content
            .as_ref()
            .map(|c| strip_thing_tags(c))
            .filter(|c| !c.is_empty())
    }
}

fn strip_thing_tags(content: &str) -> String {
    let mut result = String::with_capacity(content.len());

    let mut rest = content;
    loop {
        if let Some(start) = rest.find("<think>") {
            result.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</think>") {
                rest = &rest[start + end + "</think>".len()..];
            } else {
                break;
            }
        } else {
            result.push_str(rest);
            break;
        }
    }
    result.trim().to_string()
}

#[derive(Debug, Deserialize, Clone)]
struct UsageInfo {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default, deserialize_with = "deserialize_optional_token_count")]
    prompt_cache_hit_tokens: Option<u64>,
}

impl UsageInfo {
    fn cached_input_tokens(&self) -> Option<u64> {
        self.prompt_cache_hit_tokens.or_else(|| {
            self.prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
        })
    }
    fn into_provider_usage(self) -> TokenUsage {
        let cached_input_tokens = self.cached_input_tokens();
        TokenUsage {
            input_tokens: self.prompt_tokens,
            output_tokens: self.completion_tokens,
            cached_input_tokens,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
struct PromptTokensDetails {
    #[serde(default, deserialize_with = "deserialize_optional_token_count")]
    cached_tokens: Option<u64>,
}

fn deserialize_optional_token_count<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(normalize_token_count_value))
}

fn normalize_token_count_value(value: serde_json::Value) -> Option<u64> {
    match value {
        Value::Number(num) => {
            if let Some(value) = num.as_u64() {
                Some(value)
            } else if let Some(value) = num.as_i64() {
                if value < 0 {
                    return None;
                } else {
                    u64::try_from(value).ok()
                }
            } else {
                num.as_f64().and_then(normalize_token_count_float)
            }
        }
        Value::String(v) => v
            .trim()
            .parse::<f64>()
            .ok()
            .and_then(normalize_token_count_float),
        _ => None,
    }
}

fn normalize_token_count_float(f: f64) -> Option<u64> {
    if !f.is_finite() || f < 0.0 {
        return None;
    }
    if f < 1.0 {
        return Some(0);
    }

    if f > u64::MAX as f64 {
        return None;
    }
    Some(f.floor() as u64)
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
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        }
        OpenAiAssistantContent::Parts(parts) => {
            let mut text = String::new();
            for p in parts {
                if p.kind.as_deref() != Some("text") {
                    continue;
                }
                let Some(pt) = p.text.filter(|text| !text.is_empty()) else {
                    continue;
                };
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&pt);
            }
            if text.is_empty() { None } else { Some(text) }
        }
    }
}

fn is_valid_mistral_tool_call_id(id: &str) -> bool {
    id.len() == 9 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn reserve_tool_call_id_for_contract(
    mistral_tool_call: bool,
    tool_id: Option<String>,
    used_ids: &mut HashSet<String>,
) -> String {
    if !mistral_tool_call {
        let id = tool_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        if used_ids.insert(id.clone()) {
            return id;
        }
        loop {
            let candidate = Uuid::new_v4().to_string();
            if used_ids.insert(candidate.clone()) {
                return candidate;
            }
        }
    }

    if let Some(id) = tool_id.as_deref()
        && is_valid_mistral_tool_call_id(id)
        && used_ids.insert(id.to_string())
    {
        return id.to_string();
    }

    let mut candidate = tool_id
        .as_deref()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(9)
        .collect::<String>();

    if candidate.len() < 9 {
        candidate.extend(
            Uuid::new_v4()
                .as_simple()
                .to_string()
                .chars()
                .take(9 - candidate.len()),
        );
    }

    if used_ids.insert(candidate.clone()) {
        return candidate;
    }

    loop {
        let candidate = Uuid::new_v4()
            .as_simple()
            .to_string()
            .chars()
            .take(9)
            .collect::<String>();
        if used_ids.insert(candidate.clone()) {
            return candidate;
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
    name: Option<String>,
    arguments: Option<String>,
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
struct StreamToolCallDelta {
    index: Option<usize>,
    id: Option<String>,
    function: Option<StreamFunctionDelta>,
    name: Option<String>,
    arguments: Option<String>,
    extra_content: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize)]
struct StreamFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

impl StreamToolCallAccumulator {}

#[derive(Debug, Serialize)]
struct NativeChatRequest {
    model: String,
    messages: Vec<NativeMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptionsBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(flatten)]
    extra_body: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct NativeMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "reasoning")]
    reasoning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}
