//! OpenAI 兼容 provider -- 支持 OpenAI/OpenRouter/Ollama
//!
//! 实现 OpenAI Chat Completions API 的 tool calling 功能.
//! 将 agent-core 的 ToolSpec 转换为 API 格式, 解析响应中的 tool_calls.

use crate::error::ChatError;
use shadow_core::{
    Attributable, AuthStyle, ChatChunk, ChatRequest, ChatResponse, ModelProviderRuntimeOptions,
    Provider, Role, TokenUsage, ToolCall,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, RwLock};

pub struct OpenAiProvider {
    provider_type: String,
    /// 当前 API key -- Arc<RwLock> 支持运行时切换 (key 轮换)
    api_key: Arc<RwLock<Option<String>>>,
    base_url: String,
    opts: ModelProviderRuntimeOptions,
    /// 复用连接池 -- 构造一次, 不再 per-call 重建
    client: reqwest::Client,
}

impl OpenAiProvider {
    /// 构造器 (向后兼容) -- 不带运行时选项, 默认 Bearer auth
    pub fn new(provider_type: &str, api_key: Option<&str>, base_url: Option<&str>) -> Result<Self> {
        Self::new_with_opts(
            provider_type,
            api_key,
            base_url,
            ModelProviderRuntimeOptions::default(),
        )
    }

    /// 构造器 (带运行时选项) -- 支持 auth_style / timeout / extra_headers / api_path
    pub fn new_with_opts(
        provider_type: &str,
        api_key: Option<&str>,
        base_url: Option<&str>,
        opts: ModelProviderRuntimeOptions,
    ) -> Result<Self> {
        let default_url = match provider_type {
            "openai" => "https://api.openai.com/v1",
            "openrouter" => "https://openrouter.ai/api/v1",
            "ollama" => "http://localhost:11434/v1",
            _ => "https://api.openai.com/v1",
        };
        // HTTP client 构造一次, 后续 chat/stream/list_models 复用
        let mut builder = reqwest::Client::builder();
        if let Some(timeout) = opts.timeout {
            builder = builder.timeout(timeout);
        }
        let client = builder.build().context("创建 HTTP 客户端失败")?;
        Ok(Self {
            provider_type: provider_type.to_string(),
            api_key: Arc::new(RwLock::new(api_key.map(String::from))),
            base_url: base_url.unwrap_or(default_url).to_string(),
            opts,
            client,
        })
    }

    /// 设置/切换 API key -- Reliable 层 key 轮换时调用
    ///
    /// 传入 None 清空 key (匿名 provider, 如本地 ollama)
    pub fn set_api_key(&self, key: Option<String>) {
        if let Ok(mut guard) = self.api_key.write() {
            *guard = key;
        }
    }

    /// 借用共享的 api_key Arc -- Reliable 层用于把 key 池与 inner 同步
    #[must_use]
    pub fn shared_api_key(&self) -> Arc<RwLock<Option<String>>> {
        Arc::clone(&self.api_key)
    }

    /// 借用 HTTP client -- chat/stream/list_models 用
    fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 构建 API URL -- 尊重 opts.api_path (默认 chat/completions)
    fn build_url(&self) -> String {
        let path = self.opts.api_path.as_deref().unwrap_or("chat/completions");
        format!("{}/{path}", self.base_url)
    }

    /// 把 auth header / extra headers / query 参数应用到请求构建器
    fn apply_auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        // 附加 extra_headers
        for (k, v) in &self.opts.extra_headers {
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                && let Ok(value) = reqwest::header::HeaderValue::from_str(v)
            {
                req = req.header(name, value);
            }
        }
        let key_opt = self
            .api_key
            .read()
            .map(|guard| guard.clone())
            .ok()
            .flatten();
        let Some(key) = key_opt else {
            return req;
        };
        match &self.opts.auth_style {
            AuthStyle::Bearer => {
                if let Ok(value) =
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
                {
                    req = req.header(reqwest::header::AUTHORIZATION, value);
                }
            }
            AuthStyle::XApiKey => {
                if let Ok(value) = reqwest::header::HeaderValue::from_str(&key) {
                    req = req.header("x-api-key", value);
                }
            }
            AuthStyle::Query(param_name) => {
                req = req.query(&[(param_name.as_str(), key.as_str())]);
            }
        }
        req
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
        let client = self.client();
        let url = self.build_url();

        // 转换消息和工具
        let messages = convert_messages(&request.messages);
        let tools = convert_tools(&request.tools);

        let body = ApiRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            tools: if tools.is_empty() { None } else { Some(tools) },
            stream: false,
        };

        let resp = self
            .apply_auth(client.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::Error::new(ChatError::network(format!("LLM 请求失败: {e}"))))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::Error::new(ChatError::from_status(status, text)));
        }

        let api_resp: ApiResponse = resp
            .json()
            .await
            .context("解析 LLM 响应失败")?;

        let choice = api_resp.choices.first().context("LLM 响应无 choices")?;
        let reasoning_content = choice.message.reasoning_content.clone();

        // content 为空时退化到 reasoning_content (有些思考模型只填 reasoning_content)
        let content = match &choice.message.content {
            Some(c) if !c.is_empty() => c.clone(),
            _ => reasoning_content.clone().unwrap_or_default(),
        };

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
            reasoning_content,
        })
    }

    /// 流式聊天 -- SSE 解析
    ///
    /// 发送 `stream: true` 请求, 解析 text/event-stream 响应.
    /// 每个 `data: {json}` 行解析为 ChatChunk, `data: [DONE]` 结束流.
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatChunk>>> {
        let client = self.client();
        let url = self.build_url();

        // 转换消息和工具
        let messages = convert_messages(&request.messages);
        let tools = convert_tools(&request.tools);

        let body = ApiRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            tools: if tools.is_empty() { None } else { Some(tools) },
            stream: true,
        };

        let resp = self
            .apply_auth(client.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::Error::new(ChatError::network(format!("LLM 流式请求失败: {e}"))))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::Error::new(ChatError::from_status(status, text)));
        }

        // 创建 channel 传递解析后的 ChatChunk
        let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel::<Result<ChatChunk>>(64);

        // 后台 task: 解析 SSE 字节流, 逐行解析 JSON, 发送 ChatChunk
        tokio::spawn(async move {
            let mut byte_stream = resp.bytes_stream();
            let mut buffer = String::new();
            // 累积器: 按 index 分组累积 tool_calls 的 arguments fragments
            let mut tool_calls_map: std::collections::BTreeMap<usize, ToolCallAccum> =
                std::collections::BTreeMap::new();
            let mut content = String::new();
            let mut reasoning_content = String::new();
            let mut usage = TokenUsage::default();
            let mut in_think_block = false;

            while let Some(result) = byte_stream.next().await {
                match result {
                    Ok(bytes) => {
                        // 将字节追加到缓冲区, 按 \n 分割行
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].trim().to_string();
                            buffer = buffer[pos + 1..].to_string();

                            if line.is_empty() {
                                continue;
                            }

                            // 只处理 `data: ` 前缀的行
                            let Some(data) = line.strip_prefix("data: ") else {
                                continue;
                            };

                            // 使用提取的解析函数处理 SSE data
                            match process_sse_data(
                                data,
                                &mut content,
                                &mut tool_calls_map,
                                &mut reasoning_content,
                                &mut usage,
                                &mut in_think_block,
                            ) {
                                None => {
                                    // 收到 [DONE] -- 发送最终 Done chunk
                                    let final_tool_calls = build_tool_calls(&tool_calls_map);
                                    let _ = chunk_tx
                                        .send(Ok(ChatChunk::Done {
                                            content: std::mem::take(&mut content),
                                            tool_calls: final_tool_calls,
                                            usage: std::mem::take(&mut usage),
                                            reasoning_content: if reasoning_content.is_empty() {
                                                None
                                            } else {
                                                Some(std::mem::take(&mut reasoning_content))
                                            },
                                        }))
                                        .await;
                                    return;
                                }
                                Some(chunks) => {
                                    // 发送解析产生的 chunks
                                    for chunk in chunks {
                                        let _ = chunk_tx.send(Ok(chunk)).await;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = chunk_tx
                            .send(Err(anyhow::anyhow!("SSE 流读取失败: {e}")))
                            .await;
                        return;
                    }
                }
            }

            // 流自然结束但未收到 [DONE] -- 发送最终 Done chunk
            let final_tool_calls = build_tool_calls(&tool_calls_map);
            let _ = chunk_tx
                .send(Ok(ChatChunk::Done {
                    content: std::mem::take(&mut content),
                    tool_calls: final_tool_calls,
                    usage: std::mem::take(&mut usage),
                    reasoning_content: if reasoning_content.is_empty() {
                        None
                    } else {
                        Some(std::mem::take(&mut reasoning_content))
                    },
                }))
                .await;
        });

        // 将 mpsc::Receiver 转为 BoxStream
        let stream = futures::stream::unfold(chunk_rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        })
        .boxed();

        Ok(stream)
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let client = self.client();
        let url = format!("{}/models", self.base_url);
        let resp: ModelsResponse = self.apply_auth(client.get(&url)).send().await?.json().await?;
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
    tool_call_id: Option<String>,
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
    /// 思考模型 (DeepSeek-R1 等) 返回的推理内容, 与 content 分离
    #[serde(default)]
    reasoning_content: Option<String>,
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

// ── 辅助函数和类型 ──

/// 工具调用累积器 -- 流式响应中按 index 分组累积 tool_call 的 fragments
#[derive(Default)]
struct ToolCallAccum {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// 将累积的 ToolCallAccum 转换为完整的 ToolCall 列表
fn build_tool_calls(
    map: &std::collections::BTreeMap<usize, ToolCallAccum>,
) -> Vec<ToolCall> {
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
                                arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
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
                reasoning_content: m.reasoning_content.clone(),
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
fn split_think_content(text: &str, in_think_block: &mut bool) -> Vec<ChatChunk> {
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
                    chunks.push(ChatChunk::ReasoningDelta(remaining[..pos].to_string()));
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
                chunks.push(ChatChunk::ReasoningDelta(remaining.to_string()));
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
                    chunks.push(ChatChunk::ContentDelta(remaining[..pos].to_string()));
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
                chunks.push(ChatChunk::ContentDelta(remaining.to_string()));
                remaining = "";
            }
        }
    }

    chunks
}

/// 处理单行 SSE data payload -- 解析 JSON, 更新累积器, 返回需要发送的 chunks
///
/// 返回值:
/// - `Some(Vec<ChatChunk>)`: 解析产生的 chunks (可能为空)
/// - `None`: 收到 `[DONE]` 标记, 流结束
fn process_sse_data(
    data: &str,
    content: &mut String,
    tool_calls_map: &mut std::collections::BTreeMap<usize, ToolCallAccum>,
    reasoning_content: &mut String,
    usage: &mut TokenUsage,
    in_think_block: &mut bool,
) -> Option<Vec<ChatChunk>> {
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
    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            for chunk in split_think_content(text, in_think_block) {
                match &chunk {
                    ChatChunk::ContentDelta(c) => content.push_str(c),
                    ChatChunk::ReasoningDelta(r) => reasoning_content.push_str(r),
                    _ => {}
                }
                chunks.push(chunk);
            }
        }
    }

    // 推理内容增量 (DeepSeek-R1 等思考模型的独立字段) -- 累积并推送 ReasoningDelta
    if let Some(rc) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
        if !rc.is_empty() {
            reasoning_content.push_str(rc);
            chunks.push(ChatChunk::ReasoningDelta(rc.to_string()));
        }
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
                    chunks.push(ChatChunk::ToolCallDelta {
                        index,
                        id: entry.id.clone(),
                        name: entry.name.clone(),
                        arguments_fragment: args.to_string(),
                    });
                }
            }
        }
    }

    Some(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::ChatMessage;

    #[test]
    fn build_tool_calls_from_accumulated_fragments() {
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            0,
            ToolCallAccum {
                id: Some("call_123".to_string()),
                name: Some("get_weather".to_string()),
                arguments: r#"{"city":"北京"}"#.to_string(),
            },
        );
        let result = build_tool_calls(&map);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "call_123");
        assert_eq!(result[0].name, "get_weather");
        assert_eq!(result[0].arguments["city"], "北京");
    }

    #[test]
    fn build_tool_calls_empty_map() {
        let map = std::collections::BTreeMap::new();
        let result = build_tool_calls(&map);
        assert!(result.is_empty());
    }

    #[test]
    fn process_sse_data_content_delta() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut in_think_block = false;

        let sse_data = r#"{"choices":[{"delta":{"content":"你好"}}]}"#;
        let result = process_sse_data(sse_data, &mut content, &mut tool_calls_map, &mut reasoning, &mut usage, &mut in_think_block);

        assert!(result.is_some());
        let chunks = result.unwrap();
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            ChatChunk::ContentDelta(text) => assert_eq!(text, "你好"),
            _ => panic!("应该是 ContentDelta"),
        }
        assert_eq!(content, "你好");
    }

    #[test]
    fn process_sse_data_done_marker() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut in_think_block = false;

        let result = process_sse_data("[DONE]", &mut content, &mut tool_calls_map, &mut reasoning, &mut usage, &mut in_think_block);
        assert!(result.is_none()); // None 表示流结束
    }

    #[test]
    fn process_sse_data_tool_call_delta() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut in_think_block = false;

        // 第一个 fragment: 工具名 + 部分 arguments
        let sse1 = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"search","arguments":"{\"q\":"}}]}}]}"#;
        let _ = process_sse_data(sse1, &mut content, &mut tool_calls_map, &mut reasoning, &mut usage, &mut in_think_block);
        
        // 第二个 fragment: 剩余 arguments
        let sse2 = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"rust\"}"}}]}}]}"#;
        let _ = process_sse_data(sse2, &mut content, &mut tool_calls_map, &mut reasoning, &mut usage, &mut in_think_block);

        // 验证累积结果
        let accum = &tool_calls_map[&0];
        assert_eq!(accum.id.as_deref(), Some("call_1"));
        assert_eq!(accum.name.as_deref(), Some("search"));
        assert_eq!(accum.arguments, r#"{"q":"rust"}"#);
    }

    #[test]
    fn process_sse_data_usage_extraction() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut in_think_block = false;

        let sse_data = r#"{"choices":[{"delta":{}}],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}"#;
        let _ = process_sse_data(sse_data, &mut content, &mut tool_calls_map, &mut reasoning, &mut usage, &mut in_think_block);

        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total_tokens, 30);
    }

    #[test]
    fn process_sse_data_invalid_json_skipped() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut in_think_block = false;

        let result = process_sse_data("not valid json", &mut content, &mut tool_calls_map, &mut reasoning, &mut usage, &mut in_think_block);
        // 返回空 chunks (跳过), 不报错
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn convert_messages_handles_tool_calls() {
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_call_id: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"q": "rust"}),
            }],
            reasoning_content: None,
        }];
        let result = convert_messages(&messages);
        assert_eq!(result.len(), 1);
        assert!(result[0].content.is_none()); // 空 content + 有 tool_calls → null
        assert!(result[0].tool_calls.is_some());
    }

    #[test]
    fn convert_messages_plain_text() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            reasoning_content: None,
        }];
        let result = convert_messages(&messages);
        assert_eq!(result[0].content.as_deref(), Some("hello"));
        assert!(result[0].tool_calls.is_none());
    }
}
