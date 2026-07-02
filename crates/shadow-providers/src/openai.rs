//! OpenAI 兼容 provider -- 支持 OpenAI/OpenRouter/Ollama
//!
//! 实现 OpenAI Chat Completions API 的 tool calling 功能.
//! 将 agent-core 的 ToolSpec 转换为 API 格式, 解析响应中的 tool_calls.

use shadow_core::{
    Attributable, AuthStyle, ChatMessage, ChatRequest, ChatResponse, ModelProvider,
    ModelProviderRuntimeOptions, Role, TokenUsage, ToolCall,
};
use shadow_core::provider::{StreamChunk, StreamEvent, StreamOptions, StreamResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shadow_core::ToolSpec;

/// OpenAI 兼容适配器 -- Compat 层
///
/// 把家族差异 (auth style, base_url, API path) 适配为统一 OpenAI Chat Completions 形态.
pub struct OpenAiCompat {
    alias: String,
    /// 保留 family 以便日志/调试; trait 不再暴露 provider_type()
    #[allow(dead_code)]
    family: String,
    api_key: Option<String>,
    base_url: String,
    opts: ModelProviderRuntimeOptions,
}

impl OpenAiCompat {
    /// 构造器
    pub fn new(
        alias: &str,
        family: &str,
        api_key: Option<&str>,
        base_url: Option<&str>,
        opts: ModelProviderRuntimeOptions,
    ) -> Result<Self> {
        let default_url = match family {
            "openai" => "https://api.openai.com/v1",
            "openrouter" => "https://openrouter.ai/api/v1",
            "ollama" => "http://localhost:11434/v1",
            _ => "https://api.openai.com/v1",
        };
        Ok(Self {
            alias: alias.to_string(),
            family: family.to_string(),
            api_key: api_key.map(String::from),
            base_url: base_url.unwrap_or(default_url).to_string(),
            opts,
        })
    }

    fn client(&self) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder();
        if let Some(timeout) = self.opts.timeout {
            builder = builder.timeout(timeout);
        }
        builder.build().context("创建 HTTP 客户端失败")
    }

    fn build_url(&self) -> String {
        let path = self.opts.api_path.as_deref().unwrap_or("chat/completions");
        format!("{}/{path}", self.base_url)
    }

    fn apply_auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        for (k, v) in &self.opts.extra_headers {
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                && let Ok(value) = reqwest::header::HeaderValue::from_str(v)
            {
                req = req.header(name, value);
            }
        }
        let Some(ref key) = self.api_key else {
            return req;
        };
        match &self.opts.auth_style {
            AuthStyle::Bearer => {
                if let Ok(value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
                {
                    req = req.header(reqwest::header::AUTHORIZATION, value);
                }
            }
            AuthStyle::XApiKey => {
                if let Ok(value) = reqwest::header::HeaderValue::from_str(key) {
                    req = req.header("x-api-key", value);
                }
            }
            AuthStyle::Query(param_name) => {
                req = req.query(&[(param_name.as_str(), key.as_str())]);
            }
        }
        req
    }

    /// 内部: 发起一次非流式 HTTP 请求, 返回完整 ChatResponse
    async fn do_chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSpec]>,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        let client = self.client()?;
        let url = self.build_url();

        let api_messages = convert_messages(messages);
        let api_tools = tools.map(convert_tools);
        let body = ApiRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature,
            tools: api_tools.filter(|t| !t.is_empty()),
            stream: false,
        };

        let resp = self
            .apply_auth(client.post(&url))
            .json(&body)
            .send()
            .await
            .context("LLM 请求失败")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM 返回错误 {status}: {text}");
        }

        let api_resp: ApiResponse = resp.json().await.context("解析 LLM 响应失败")?;
        let choice = api_resp.choices.first().context("LLM 响应无 choices")?;
        let reasoning_content = choice.message.reasoning_content.clone();

        let content = match &choice.message.content {
            Some(c) if !c.is_empty() => c.clone(),
            _ => reasoning_content.clone().unwrap_or_default(),
        };

        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .as_ref()
            .map(|tcs| tcs.iter().map(api_tool_call_to_core).collect())
            .unwrap_or_default();

        let usage = map_usage(&api_resp.usage);

        Ok(ChatResponse {
            content,
            tool_calls,
            usage,
            reasoning_content,
        })
    }

    /// 内部: 发起一次流式 HTTP 请求, 返回 StreamEvent 流
    fn do_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolSpec>>,
        model: String,
        temperature: Option<f64>,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let url = self.build_url();
        let api_messages = convert_messages(&messages);
        let api_tools = tools.map(|t| convert_tools(&t));
        let body = ApiRequest {
            model,
            messages: api_messages,
            temperature,
            tools: api_tools.filter(|t| !t.is_empty()),
            stream: true,
        };

        let client = match self.client() {
            Ok(c) => c,
            Err(e) => {
                return futures::stream::iter([
                    Ok(map_anyhow_to_event(e)),
                    Ok(StreamEvent::Final),
                ])
                .boxed();
            }
        };

        // 同步构建请求 -- self 只在这里被借用, 之后只有 owned 数据进入 spawn
        let request_builder = self.apply_auth(client.post(&url)).json(&body);

        let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel::<StreamResult<StreamEvent>>(64);

        tokio::spawn(async move {
            let resp = match request_builder
                .send()
                .await
                .context("LLM 流式请求失败")
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = chunk_tx.send(Ok(map_anyhow_to_event(e))).await;
                    let _ = chunk_tx.send(Ok(StreamEvent::Final)).await;
                    return;
                }
            };
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                let _ = chunk_tx
                    .send(Ok(map_anyhow_to_event(anyhow::anyhow!(
                        "LLM 返回错误 {status}: {text}"
                    ))))
                    .await;
                let _ = chunk_tx.send(Ok(StreamEvent::Final)).await;
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut buffer = String::new();
            let mut tool_calls_map: std::collections::BTreeMap<usize, ToolCallAccum> =
                std::collections::BTreeMap::new();
            let mut content = String::new();
            let mut reasoning_content = String::new();
            let mut usage = TokenUsage::default();

            while let Some(result) = byte_stream.next().await {
                match result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].trim().to_string();
                            buffer = buffer[pos + 1..].to_string();
                            if line.is_empty() {
                                continue;
                            }
                            let Some(data) = line.strip_prefix("data: ") else {
                                continue;
                            };
                            match process_sse_data(
                                data,
                                &mut content,
                                &mut tool_calls_map,
                                &mut reasoning_content,
                                &mut usage,
                            ) {
                                None => {
                                    // [DONE] -- 发送 usage + Final
                                    let _ = chunk_tx.send(Ok(StreamEvent::Usage(usage))).await;
                                    let _ = chunk_tx.send(Ok(StreamEvent::Final)).await;
                                    return;
                                }
                                Some(events) => {
                                    for ev in events {
                                        let _ = chunk_tx.send(Ok(ev)).await;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = chunk_tx
                            .send(Ok(map_anyhow_to_event(anyhow::anyhow!(
                                "SSE 流读取失败: {e}"
                            ))))
                            .await;
                        let _ = chunk_tx.send(Ok(StreamEvent::Final)).await;
                        return;
                    }
                }
            }

            // 流自然结束 -- 同样发送 usage + Final
            let _ = chunk_tx.send(Ok(StreamEvent::Usage(usage))).await;
            let _ = chunk_tx.send(Ok(StreamEvent::Final)).await;
        });

        futures::stream::unfold(
            chunk_rx,
            |mut rx: tokio::sync::mpsc::Receiver<StreamResult<StreamEvent>>| async move {
                rx.recv().await.map(|item| (item, rx))
            },
        )
        .boxed()
    }
}

impl Attributable for OpenAiCompat {
    fn role(&self) -> Role {
        Role::Provider
    }
    fn alias(&self) -> &str {
        &self.alias
    }
}

#[async_trait]
impl ModelProvider for OpenAiCompat {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        let mut messages = Vec::with_capacity(2);
        if let Some(s) = system_prompt {
            messages.push(ChatMessage {
                role: "system".into(),
                content: s.to_string(),
                ..Default::default()
            });
        }
        messages.push(ChatMessage {
            role: "user".into(),
            content: message.to_string(),
            ..Default::default()
        });
        let resp = self.do_chat(&messages, None, model, temperature).await?;
        Ok(resp.content)
    }

    // chat_with_history / simple_chat 用 trait 默认实现 (派发到 chat_with_system)

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        self.do_chat(request.messages, request.tools, model, temperature)
            .await
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        // 把 serde_json::Value 形态的工具转成 ToolSpec (MVP: 透传 schema)
        let specs: Vec<ToolSpec> = tools
            .iter()
            .filter_map(|v| {
                let name = v.get("function")?.get("name")?.as_str()?.to_string();
                let description = v
                    .get("function")
                    .and_then(|f| f.get("description"))
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                let parameters = v
                    .get("function")
                    .and_then(|f| f.get("parameters"))
                    .cloned()
                    .unwrap_or(Value::Object(Default::default()));
                Some(ToolSpec {
                    name,
                    description,
                    parameters,
                })
            })
            .collect();

        if specs.is_empty() {
            // 没有有效工具 -- 退化为普通 chat
            self.do_chat(messages, None, model, temperature).await
        } else {
            self.do_chat(messages, Some(&specs), model, temperature)
                .await
        }
    }

    fn stream_chat(
        &self,
        request: ChatRequest,
        model: &str,
        temperature: Option<f64>,
        _options: StreamOptions,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let tools = request.tools.map(|t| t.to_vec());
        self.do_stream(
            request.messages.to_vec(),
            tools,
            model.to_string(),
            temperature,
        )
    }

    fn stream_chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
        _options: StreamOptions,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let mut messages = Vec::with_capacity(2);
        if let Some(s) = system_prompt {
            messages.push(ChatMessage {
                role: "system".into(),
                content: s.to_string(),
                ..Default::default()
            });
        }
        messages.push(ChatMessage {
            role: "user".into(),
            content: message.to_string(),
            ..Default::default()
        });
        self.do_stream(messages, None, model.to_string(), temperature)
    }

    // stream_chat_with_history 用 trait 默认实现 (派发到 stream_chat_with_system)

    async fn list_models(&self) -> Result<Vec<String>> {
        let client = self.client()?;
        let url = format!("{}/models", self.base_url);
        let resp: ModelsResponse = self.apply_auth(client.get(&url)).send().await?.json().await?;
        Ok(resp.data.into_iter().map(|m| m.id).collect())
    }

    fn supports_native_tools(&self) -> bool {
        true
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
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

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
    #[serde(default)]
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
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ApiToolCallResponse>>,
}

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

#[derive(Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    #[allow(dead_code)]
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

/// 工具调用累积器
#[derive(Default)]
struct ToolCallAccum {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// 把响应中的 ApiToolCallResponse 转为核心 ToolCall
fn api_tool_call_to_core(tc: &ApiToolCallResponse) -> ToolCall {
    ToolCall {
        id: tc.id.clone(),
        name: tc.function.name.clone(),
        arguments: tc.function.arguments.clone(),
        extra_content: None,
    }
}

/// OpenAI ApiUsage → 核心 TokenUsage (字段名不同)
fn map_usage(u: &ApiUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: Some(u.prompt_tokens),
        output_tokens: Some(u.completion_tokens),
        cached_input_tokens: None,
    }
}

/// anyhow::Error → 错误事件 (TextDelta with is_final=true)
///
/// 当前 shadow-core 的 `StreamError` 是空枚举 (无可构造变体),
/// 无法走 `Err(StreamError)` 路径. 因此把错误包装成 final chunk,
/// 让消费者通过 `StreamChunk::is_final` 感知流异常结束.
fn map_anyhow_to_event(e: anyhow::Error) -> StreamEvent {
    let msg = format!("{e:#}");
    tracing::warn!("stream error: {msg}");
    StreamEvent::TextDelta(StreamChunk::error(msg))
}

/// 转换消息: ChatMessage -> ApiMessage
fn convert_messages(messages: &[ChatMessage]) -> Vec<ApiMessage> {
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
                                arguments: tc.arguments.clone(),
                            },
                        })
                        .collect(),
                )
            };

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
fn convert_tools(tools: &[ToolSpec]) -> Vec<ApiTool> {
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

/// 处理单行 SSE data payload
///
/// 返回:
/// - `Some(Vec<StreamEvent>)`: 解析产生的事件 (可能为空)
/// - `None`: 收到 `[DONE]`, 流结束
fn process_sse_data(
    data: &str,
    content: &mut String,
    tool_calls_map: &mut std::collections::BTreeMap<usize, ToolCallAccum>,
    reasoning_content: &mut String,
    usage: &mut TokenUsage,
) -> Option<Vec<StreamEvent>> {
    if data == "[DONE]" {
        return None;
    }

    let Ok(chunk_json) = serde_json::from_str::<Value>(data) else {
        return Some(Vec::new());
    };

    let mut events = Vec::new();

    if let Some(u) = chunk_json.get("usage") {
        let p = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let c = u
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        usage.input_tokens = Some(p);
        usage.output_tokens = Some(c);
    }

    let Some(delta) = chunk_json
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("delta"))
    else {
        return Some(events);
    };

    if let Some(text) = delta.get("content").and_then(|v| v.as_str())
        && !text.is_empty()
    {
        content.push_str(text);
        events.push(StreamEvent::TextDelta(StreamChunk::delta(text)));
    }

    if let Some(rc) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
        reasoning_content.push_str(rc);
    }

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
                    events.push(StreamEvent::ToolCallDelta(ToolCall {
                        id: entry.id.clone().unwrap_or_default(),
                        name: entry.name.clone().unwrap_or_default(),
                        arguments: args.to_string(),
                        extra_content: None,
                    }));
                }
            }
        }
    }

    Some(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_sse_data_text_delta() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();

        let sse_data = r#"{"choices":[{"delta":{"content":"你好"}}]}"#;
        let result = process_sse_data(
            sse_data,
            &mut content,
            &mut tool_calls_map,
            &mut reasoning,
            &mut usage,
        );

        let events = result.unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::TextDelta(chunk) => assert_eq!(chunk.delta, "你好"),
            _ => panic!("应该是 TextDelta"),
        }
        assert_eq!(content, "你好");
    }

    #[test]
    fn process_sse_data_done_marker() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();

        let result = process_sse_data(
            "[DONE]",
            &mut content,
            &mut tool_calls_map,
            &mut reasoning,
            &mut usage,
        );
        assert!(result.is_none());
    }

    #[test]
    fn process_sse_data_tool_call_delta() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();

        let sse1 = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"search","arguments":"{\"q\":"}}]}}]}"#;
        let _ = process_sse_data(
            sse1,
            &mut content,
            &mut tool_calls_map,
            &mut reasoning,
            &mut usage,
        );
        let sse2 = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"rust\"}"}}]}}]}"#;
        let _ = process_sse_data(
            sse2,
            &mut content,
            &mut tool_calls_map,
            &mut reasoning,
            &mut usage,
        );

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

        let sse_data = r#"{"choices":[{"delta":{}}],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}"#;
        let _ = process_sse_data(
            sse_data,
            &mut content,
            &mut tool_calls_map,
            &mut reasoning,
            &mut usage,
        );

        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(20));
    }

    #[test]
    fn process_sse_data_invalid_json_skipped() {
        let mut content = String::new();
        let mut tool_calls_map = std::collections::BTreeMap::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();

        let result = process_sse_data(
            "not valid json",
            &mut content,
            &mut tool_calls_map,
            &mut reasoning,
            &mut usage,
        );
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn convert_messages_plain_text() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            ..Default::default()
        }];
        let result = convert_messages(&messages);
        assert_eq!(result[0].content.as_deref(), Some("hello"));
        assert!(result[0].tool_calls.is_none());
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
                arguments: r#"{"q":"rust"}"#.to_string(),
                extra_content: None,
            }],
            ..Default::default()
        }];
        let result = convert_messages(&messages);
        assert_eq!(result.len(), 1);
        assert!(result[0].content.is_none());
        assert!(result[0].tool_calls.is_some());
    }

    #[test]
    fn openai_compat_exposes_alias_and_family() {
        use shadow_core::Attributable;
        let p = OpenAiCompat::new(
            "openai.default",
            "openai",
            Some("sk-test"),
            None,
            ModelProviderRuntimeOptions::default(),
        )
        .unwrap();
        assert_eq!(p.alias(), "openai.default");
    }

    #[test]
    fn map_usage_converts_correctly() {
        let api = ApiUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        };
        let u = map_usage(&api);
        assert_eq!(u.input_tokens, Some(100));
        assert_eq!(u.output_tokens, Some(50));
        assert_eq!(u.cached_input_tokens, None);
    }
}
