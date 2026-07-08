//! OpenAI 兼容 provider -- 支持 OpenAI/OpenRouter/Ollama
//!
//! 实现 OpenAI Chat Completions API 的 tool calling 功能.
//! 将 agent-core 的 ToolSpec 转换为 API 格式, 解析响应中的 tool_calls.


use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use reqwest::{Client, Error, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use shadow_core::{
    Attributable, AuthStyle, ChatRequest, ChatResponse, ModelProvider, ModelProviderKind,
    ModelProviderRuntimeOptions, ProviderKind, Role, StreamChunk, TokenUsage, ToolCall,
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

        let body = ApiRequest {
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

    async fn list_models(&self) -> Result<Vec<String>> {
        let client = self.client();
        let url = format!("{}/models", self.base_url);
        let resp: ModelsResponse = self
            .apply_auth(client.get(&url))
            .send()
            .await?
            .json()
            .await?;
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
    pub fn effective_content(&self) -> String {
        self.content.as_ref().map(|c| )
    }
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
enum  OpenAiAssistantContent{
    Text(String),
    Parts(Vec<OpenAiAssistantContentPart>)
}
#[derive(Debug, Deserialize)]
struct OpenAiAssistantContentPart{
    #[serde(rename="type")]
    kind:Option<String>,

    text: Option<String>,
}

impl From<RawResponseMessage> for ResponseMessage {
    fn from(raw: RawResponseMessage) -> Self {
        let reasoning_content = raw.reasoning_content.or(raw.reasoning);
        Self{
            content: openai_assistant_content_plaintext(raw.content),
            reasoning_content,
            tool_calls: raw.tool_calls
        }
    }
}

fn openai_assistant_content_plaintext(content: Option<OpenAiAssistantContent>) -> Option<String>{
    match content? {
        OpenAiAssistantContent::Text(t) => {
            if t.is_empty() { None }else { Some(t) }
        }
        OpenAiAssistantContent::Parts(parts) => {
            let mut text = String::new();
            for p in parts{
                if p.kind.as_deref() != Some(text) {
                    continue;
                }
                let Some(pt) = p.text.filter(text| !text.is_empty()) else{continue;}
                if !text.is_empty() {text.push('\n');}
                text.push_str(&pt);

            }
            if text.is_empty() {None} else {Some(text)}
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
struct ToolCallAccum {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// 将累积的 ToolCallAccum 转换为完整的 ToolCall 列表
fn build_tool_calls(map: &std::collections::BTreeMap<usize, ToolCallAccum>) -> Vec<ToolCall> {
    map.values()
        .map(|t| ToolCall {
            id: t.id.clone().unwrap_or_default(),
            name: t.name.clone().unwrap_or_default(),
            arguments: serde_json::from_str(&t.arguments).unwrap_or(Value::Null),
        })
        .collect()
}

/// 转换消息: ChatMessage -> ApiMessage
fn convert_messages(messages: &[shadow_core::ChatMessage]) -> Vec<ApiMessage> {
    messages
        .iter()
        .map(|m| {
            // content 为空时, API 期望 content 为 null
            let content = if m.content.is_empty() {
                None
            } else {
                Some(m.content.clone())
            };

            ApiMessage {
                role: m.role.clone(),
                content,
                tool_calls: None,
                reasoning_content: None,
            }
        })
        .collect()
}

/// 转换工具规格: ToolSpec -> ApiTool
fn convert_tools(tools: &[shadow_core::ToolSpec]) -> Vec<ApiTool> {
    tools
        .iter()
        .map(|t| ApiTool {
            tool_type: "function".to_string(),
            function: ApiToolSpec {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        })
        .collect()
}

/// 雪花字符 (U+2744), 部分 provider 用此标记思考内容
const THINK_SNOWFLAKE: &str = "\u{2744}";

/// 将 content 增量拆分为 ContentDelta 和 ReasoningDelta
///
/// 处理两种思考标签格式:
/// 1. `<think`/`</think` (标准格式, 如 MiniMax)
/// 2. 雪花字符 U+2744 (部分模型使用, 开闭标签相同)
///
/// 使用 `in_think_block` 跟踪当前是否在思考块内, 支持跨 chunk 的状态维护.
fn split_think_content(text: &str, in_think_block: &mut bool) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if *in_think_block {
            // 在思考块内: 查找结束标签 </think 或 雪花字符
            let end_pos = remaining
                .find("</think")
                .map(|pos| (pos, "</think".len(), false))
                .or_else(|| {
                    remaining
                        .find(THINK_SNOWFLAKE)
                        .map(|pos| (pos, THINK_SNOWFLAKE.len(), true))
                });

            if let Some((pos, tag_len, is_snowflake)) = end_pos {
                // 结束标签之前的内容是 ReasoningDelta
                if pos > 0 {
                    chunks.push(StreamChunk::reasoning(remaining[..pos].to_string()));
                }
                // 跳过结束标签
                remaining = &remaining[pos + tag_len..];
                // 跳过可选的 > (仅对 </think 标签)
                if !is_snowflake && remaining.starts_with('>') {
                    remaining = &remaining[1..];
                }
                *in_think_block = false;
            } else {
                // 没有结束标签, 全部是 ReasoningDelta
                chunks.push(StreamChunk::reasoning(remaining.to_string()));
                remaining = "";
            }
        } else {
            // 不在思考块内: 查找开始标签 <think 或 雪花字符
            let start_pos = remaining
                .find("<think")
                .map(|pos| (pos, "<think".len(), false))
                .or_else(|| {
                    remaining
                        .find(THINK_SNOWFLAKE)
                        .map(|pos| (pos, THINK_SNOWFLAKE.len(), true))
                });

            if let Some((pos, tag_len, is_snowflake)) = start_pos {
                // 开始标签之前的内容是 ContentDelta
                if pos > 0 {
                    chunks.push(StreamChunk::delta(remaining[..pos].to_string()));
                }
                // 跳过开始标签
                remaining = &remaining[pos + tag_len..];
                // 跳过可选的 > (仅对 <think 标签)
                if !is_snowflake && remaining.starts_with('>') {
                    remaining = &remaining[1..];
                }
                *in_think_block = true;
            } else {
                // 没有开始标签, 全部是 ContentDelta
                chunks.push(StreamChunk::delta(remaining.to_string()));
                remaining = "";
            }
        }
    }

    chunks
}

/// 处理单行 SSE data payload -- 解析 JSON, 更新累积器, 返回需要发送的 chunks
///
/// 返回值:
/// - `Some(Vec<StreamChunk>)`: 解析产生的 chunks (可能为空)
/// - `None`: 收到 `[DONE]` 标记, 流结束
fn process_sse_data(
    data: &str,
    content: &mut String,
    tool_calls_map: &mut std::collections::BTreeMap<usize, ToolCallAccum>,
    reasoning_content: &mut String,
    usage: &mut TokenUsage,
    in_think_block: &mut bool,
) -> Option<Vec<StreamChunk>> {
    // [DONE] 标记 -- 返回 None 表示流结束
    if data == "[DONE]" {
        return None;
    }

    // 解析 JSON, 失败则返回空 chunks (跳过此行)
    let Ok(chunk_json) = serde_json::from_str::<Value>(data) else {
        return Some(Vec::new());
    };

    let mut chunks = Vec::new();

    // 提取 usage (通常在最后一个 chunk)
    if let Some(u) = chunk_json.get("usage") {
        usage.prompt_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        usage.completion_tokens = u
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        usage.total_tokens = u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    }

    // 提取 choices[0].delta
    let Some(delta) = chunk_json
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("delta"))
    else {
        return Some(chunks);
    };

    // 文本增量 -- 可能含 <think 标签, 需要拆分为 ContentDelta 和 ReasoningDelta
    if let Some(text) = delta.get("content").and_then(|v| v.as_str())
        && !text.is_empty()
    {
        for chunk in split_think_content(text, in_think_block) {
            if !chunk.delta.is_empty() {
                content.push_str(&chunk.delta);
            }
            if let Some(r) = &chunk.reasoning {
                reasoning_content.push_str(r);
            }
            chunks.push(chunk);
        }
    }

    // 推理内容增量 (DeepSeek-R1 等思考模型的独立字段) -- 累积并推送 ReasoningDelta
    if let Some(rc) = delta.get("reasoning_content").and_then(|v| v.as_str())
        && !rc.is_empty()
    {
        reasoning_content.push_str(rc);
        chunks.push(StreamChunk::reasoning(rc.to_string()));
    }

    // 工具调用增量
    if let Some(tool_call_deltas) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_call_deltas {
            let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let entry = tool_calls_map.entry(index).or_default();

            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                entry.id = Some(id.to_string());
            }
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    entry.name = Some(name.to_string());
                }
                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                    entry.arguments.push_str(args);
                    // 工具调用增量在 provider 内部累积, 不再推送 StreamChunk
                }
            }
        }
    }

    Some(chunks)
}
