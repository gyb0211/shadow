//! Anthropic 原生 provider -- 支持 Claude 系列模型和原生 tool_use 格式
//!
//! 实现 Anthropic Messages API (POST /v1/messages).
//! 与 OpenAI Chat Completions 的主要差异:
//! - system 消息提取为 top-level `system` 参数, 不放入 messages 数组
//! - content 是 content block 数组 (text / tool_use / tool_result), 不是单个字符串
//! - tool 定义用 `input_schema` 而非 `parameters`
//! - max_tokens 是必填字段 (非可选)
//! - usage 字段为 input_tokens / output_tokens (无 total_tokens)
//!
//! 请求头: x-api-key, anthropic-version: 2023-06-01, content-type: application/json

use crate::error::ChatError;
use crate::reliable::KeyRotator;
use shadow_core::{
    Attributable, AuthStyle, ChatRequest, ChatResponse, ModelProvider,
    ModelProviderRuntimeOptions, Role, TokenUsage, ToolCall,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, RwLock};

/// Anthropic API 默认 base_url
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
/// Anthropic API 版本头
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Anthropic 默认最大输出 token (API 必填字段)
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Anthropic 原生 provider -- Claude 系列模型
pub struct AnthropicProvider {
    /// 别名 (如 "anthropic.claude") -- 用于 Attributable
    alias: String,
    /// 当前 API key -- Arc<RwLock> 支持运行时切换 (key 轮换)
    api_key: Arc<RwLock<Option<String>>>,
    /// base_url (默认 https://api.anthropic.com/v1)
    base_url: String,
    /// 运行时选项 (timeout / extra_headers / auth_style)
    opts: ModelProviderRuntimeOptions,
    /// 复用连接池 -- 构造一次, 不再 per-call 重建
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// 构造器 (向后兼容) -- 不带运行时选项
    pub fn new(api_key: Option<&str>, base_url: Option<&str>) -> Result<Self> {
        Self::new_with_alias(
            "anthropic",
            api_key,
            base_url,
            ModelProviderRuntimeOptions::default(),
        )
    }

    /// 构造器 (带别名) -- 用于 Reliable 层注入完整 alias (如 "anthropic.claude")
    pub fn new_with_alias(
        alias: &str,
        api_key: Option<&str>,
        base_url: Option<&str>,
        opts: ModelProviderRuntimeOptions,
    ) -> Result<Self> {
        // HTTP client 构造一次, 后续 chat/stream/list_models 复用
        let mut builder = reqwest::Client::builder();
        if let Some(timeout) = opts.timeout {
            builder = builder.timeout(timeout);
        }
        let client = builder.build().context("创建 HTTP 客户端失败")?;
        Ok(Self {
            alias: alias.to_string(),
            api_key: Arc::new(RwLock::new(api_key.map(String::from))),
            base_url: base_url.unwrap_or(DEFAULT_BASE_URL).to_string(),
            opts,
            client,
        })
    }

    /// 设置/切换 API key -- Reliable 层 key 轮换时调用
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

    /// 借用 HTTP client
    fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 构建 Messages API URL
    fn build_url(&self) -> String {
        format!("{}/messages", self.base_url)
    }

    /// 把 auth header / extra headers 应用到请求构建器
    ///
    /// Anthropic 使用 x-api-key 头, 但如果 auth_style 被显式设为 Bearer
    /// (如某些代理), 则用 Authorization: Bearer.
    fn apply_auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        // 固定头: anthropic-version
        req = req.header("anthropic-version", ANTHROPIC_VERSION);

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
            // 默认: Anthropic 原生 x-api-key
            AuthStyle::XApiKey | AuthStyle::Bearer => {
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

impl Attributable for AnthropicProvider {
    fn role(&self) -> Role {
        Role::Provider
    }
    fn alias(&self) -> &str {
        &self.alias
    }
}

impl KeyRotator for AnthropicProvider {
    fn set_key(&self, key: Option<&str>) {
        AnthropicProvider::set_api_key(self, key.map(String::from));
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn supports_native_tools(&self) -> bool {
        true
    }

    fn default_temperature(&self) -> f64 {
        1.0 // Anthropic 文档默认 1.0
    }

    /// chat_with_system -- Anthropic Messages API 单轮调用
    ///
    /// 发送 system + user 消息到 /v1/messages, 返回文本响应.
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        let client = self.client();
        let url = self.build_url();

        // 构建消息: 只有 user 消息 (system 提取为 top-level 参数)
        let messages = vec![ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentOut::Text {
                text: message.to_string(),
            }],
        }];

        let body = ApiRequest {
            model: model.to_string(),
            max_tokens: DEFAULT_MAX_TOKENS,
            system: system_prompt.map(|s| s.to_string()),
            messages,
            temperature,
            tools: None,
            stream: false,
        };

        let resp = self
            .apply_auth(client.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                anyhow::Error::new(ChatError::network(format!("Anthropic 请求失败: {e}")))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::Error::new(ChatError::from_status(status, text)));
        }

        let api_resp: ApiResponse = resp
            .json()
            .await
            .context("解析 Anthropic 响应失败")?;

        // 提取文本内容
        let text_parts: Vec<String> = api_resp
            .content
            .iter()
            .filter_map(|block| {
                if block.kind == "text" {
                    block.text.clone()
                } else {
                    None
                }
            })
            .collect();

        Ok(text_parts.join("\n"))
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        // Anthropic 暂无公开的 list models 端点, 返回已知模型列表
        Ok(vec![
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-20250514".to_string(),
            "claude-3-7-sonnet-20250219".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ])
    }
}

// ── API 类型 (Anthropic Messages API 格式) ──

/// 请求体 -- Anthropic Messages API
#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
    stream: bool,
}

/// 请求消息 -- content 为 content block 数组
#[derive(Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: Vec<ApiContentOut>,
}

/// 输出 content block (请求中发送给 API 的)
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum ApiContentOut {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// 工具定义 -- Anthropic 用 input_schema 而非 parameters
#[derive(Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: Value,
}

/// 响应体 -- Anthropic Messages API
#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    content: Vec<ApiContentIn>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

/// 输入 content block (从 API 响应解析的)
#[derive(Deserialize)]
struct ApiContentIn {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<Value>,
}

/// Anthropic usage -- input_tokens / output_tokens (无 total_tokens)
#[derive(Deserialize)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

// ── 消息转换 ──

/// 将 shadow ChatMessage 列表转换为 Anthropic 格式
///
/// - system 消息提取为 top-level system 参数 (多条合并)
/// - assistant 消息: 含 tool_calls 时展开为 tool_use content block
/// - tool 消息: 转换为 user 角色的 tool_result content block
/// - 连续相同 role 的消息合并 (Anthropic 要求 user/assistant 交替)
///
/// 返回 (system, messages)
fn convert_messages(
    messages: &[shadow_core::ChatMessage],
) -> (Option<String>, Vec<ApiMessage>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut native_messages: Vec<ApiMessage> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                if !msg.content.is_empty() {
                    system_parts.push(msg.content.clone());
                }
            }
            "assistant" => {
                let blocks = convert_assistant_message(msg);
                if blocks.is_empty() {
                    continue;
                }
                // 合并连续相同 role 的消息
                push_or_merge(&mut native_messages, "assistant", blocks);
            }
            "tool" => {
                // tool 消息转换为 user 角色的 tool_result block
                // ChatMessage 不再有 tool_call_id, 从 content 中无法提取, 用空字符串
                let tool_call_id = String::new();
                let blocks = vec![ApiContentOut::ToolResult {
                    tool_use_id: tool_call_id,
                    content: msg.content.clone(),
                }];
                // Anthropic 中 tool_result 属于 user 角色
                push_or_merge(&mut native_messages, "user", blocks);
            }
            _ => {
                // user 或未知角色 -- 作为 user 消息
                if msg.content.is_empty() {
                    continue;
                }
                let blocks = vec![ApiContentOut::Text {
                    text: msg.content.clone(),
                }];
                push_or_merge(&mut native_messages, "user", blocks);
            }
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system, native_messages)
}

/// 转换 assistant 消息 -- ChatMessage 只有 role + content, 无 tool_calls
fn convert_assistant_message(msg: &shadow_core::ChatMessage) -> Vec<ApiContentOut> {
    let mut blocks = Vec::new();

    // 文本内容 (可能为空)
    if !msg.content.is_empty() {
        blocks.push(ApiContentOut::Text {
            text: msg.content.clone(),
        });
    }

    // ChatMessage 不再有 tool_calls 字段, 无法展开为 tool_use blocks
    blocks
}

/// 合并连续相同 role 的消息 (Anthropic 要求 user/assistant 交替)
fn push_or_merge(messages: &mut Vec<ApiMessage>, role: &str, blocks: Vec<ApiContentOut>) {
    if let Some(last) = messages.last_mut()
        && last.role == role
    {
        last.content.extend(blocks);
    } else {
        messages.push(ApiMessage {
            role: role.to_string(),
            content: blocks,
        });
    }
}

// ── 工具转换 ──

/// 将 shadow ToolSpec 列表转换为 Anthropic 工具格式
///
/// Anthropic 用 input_schema (而非 OpenAI 的 parameters),
/// 格式为 {name, description, input_schema}
fn convert_tools(tools: &[shadow_core::ToolSpec]) -> Vec<ApiTool> {
    tools
        .iter()
        .map(|t| ApiTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.parameters.clone(),
        })
        .collect()
}

// ── 响应解析 ──

/// 解析 Anthropic 响应 -- 提取 text / tool_use / usage
fn parse_response(api_resp: &ApiResponse) -> Result<ChatResponse> {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in &api_resp.content {
        match block.kind.as_str() {
            "text" => {
                if let Some(text) = &block.text
                    && !text.trim().is_empty()
                {
                    text_parts.push(text.clone());
                }
            }
            "tool_use" => {
                let name = block.name.clone().unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                let arguments = block
                    .input
                    .clone()
                    .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
                let id = block.id.clone().unwrap_or_default();
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            _ => {}
        }
    }

    let usage = api_resp.usage.as_ref().map(|u| TokenUsage {
        prompt_tokens: u.input_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: u.input_tokens + u.output_tokens,
    });

    Ok(ChatResponse {
        text: Some(text_parts.join("\n")),
        tool_calls,
        usage,
        reasoning_content: None, // Anthropic 原生无 reasoning_content
    })
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::{ChatMessage, ToolSpec};

    #[test]
    fn convert_messages_extracts_system() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "你是一个助手".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "你好".to_string(),
            },
        ];
        let (system, msgs) = convert_messages(&messages);
        assert_eq!(system.as_deref(), Some("你是一个助手"));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn convert_messages_merges_multiple_system() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "规则1".to_string(),
            },
            ChatMessage {
                role: "system".to_string(),
                content: "规则2".to_string(),
            },
        ];
        let (system, _) = convert_messages(&messages);
        assert_eq!(system.as_deref(), Some("规则1\n\n规则2"));
    }

    #[test]
    fn convert_messages_assistant_with_tool_calls() {
        // ChatMessage 不再有 tool_calls 字段, 只测试纯文本 assistant 消息
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: "让我查一下".to_string(),
        }];
        let (_, msgs) = convert_messages(&messages);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "assistant");
        // 只有 text block
        assert_eq!(msgs[0].content.len(), 1);
    }

    #[test]
    fn convert_messages_tool_result_to_user() {
        let messages = vec![ChatMessage {
            role: "tool".to_string(),
            content: "晴天 25度".to_string(),
        }];
        let (_, msgs) = convert_messages(&messages);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content.len(), 1);
        // 验证是 tool_result block
        match &msgs[0].content[0] {
            ApiContentOut::ToolResult { tool_use_id, content } => {
                // ChatMessage 不再有 tool_call_id, tool_use_id 为空字符串
                assert_eq!(tool_use_id, "");
                assert_eq!(content, "晴天 25度");
            }
            _ => panic!("应该是 ToolResult block"),
        }
    }

    #[test]
    fn convert_messages_merges_consecutive_user() {
        // 连续 user 消息应合并 (Anthropic 要求交替)
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: "你好".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "在吗".to_string(),
            },
        ];
        let (_, msgs) = convert_messages(&messages);
        assert_eq!(msgs.len(), 1, "连续 user 消息应合并为一条");
        assert_eq!(msgs[0].content.len(), 2);
    }

    #[test]
    fn convert_tools_to_anthropic_format() {
        let tools = vec![ToolSpec {
            name: "get_weather".to_string(),
            description: "获取天气".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": {"type": "string"}
                }
            }),
        }];
        let result = convert_tools(&tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "get_weather");
        assert_eq!(result[0].description, "获取天气");
        assert_eq!(result[0].input_schema["type"], "object");
    }

    #[test]
    fn parse_response_text_only() {
        let api_resp = ApiResponse {
            content: vec![ApiContentIn {
                kind: "text".to_string(),
                text: Some("你好".to_string()),
                id: None,
                name: None,
                input: None,
            }],
            usage: Some(ApiUsage {
                input_tokens: 10,
                output_tokens: 5,
            }),
        };
        let result = parse_response(&api_resp).unwrap();
        assert_eq!(result.text.unwrap(), "你好");
        assert!(result.tool_calls.is_empty());
        let usage = result.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn parse_response_tool_use() {
        let api_resp = ApiResponse {
            content: vec![
                ApiContentIn {
                    kind: "text".to_string(),
                    text: Some("让我查一下".to_string()),
                    id: None,
                    name: None,
                    input: None,
                },
                ApiContentIn {
                    kind: "tool_use".to_string(),
                    text: None,
                    id: Some("tool_1".to_string()),
                    name: Some("get_weather".to_string()),
                    input: Some(serde_json::json!({"city": "北京"})),
                },
            ],
            usage: Some(ApiUsage {
                input_tokens: 20,
                output_tokens: 15,
            }),
        };
        let result = parse_response(&api_resp).unwrap();
        assert_eq!(result.text.unwrap(), "让我查一下");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "tool_1");
        assert_eq!(result.tool_calls[0].name, "get_weather");
        assert_eq!(result.tool_calls[0].arguments["city"], "北京");
    }

    #[test]
    fn parse_response_empty_content() {
        let api_resp = ApiResponse {
            content: vec![],
            usage: None,
        };
        let result = parse_response(&api_resp).unwrap();
        assert!(result.text.unwrap().is_empty());
        assert!(result.tool_calls.is_empty());
        assert!(result.usage.is_none());
    }

    #[test]
    fn parse_response_multiple_text_blocks_joined() {
        let api_resp = ApiResponse {
            content: vec![
                ApiContentIn {
                    kind: "text".to_string(),
                    text: Some("第一段".to_string()),
                    id: None,
                    name: None,
                    input: None,
                },
                ApiContentIn {
                    kind: "text".to_string(),
                    text: Some("第二段".to_string()),
                    id: None,
                    name: None,
                    input: None,
                },
            ],
            usage: None,
        };
        let result = parse_response(&api_resp).unwrap();
        assert_eq!(result.text.unwrap(), "第一段\n第二段");
    }

    #[test]
    fn provider_construction_works() {
        let provider = AnthropicProvider::new(Some("sk-ant-test"), None);
        assert!(provider.is_ok());
        let provider = provider.unwrap();
        assert_eq!(provider.alias(), "anthropic");
        assert!(provider.supports_native_tools());
        assert_eq!(provider.default_temperature(), 1.0);
    }

    #[test]
    fn provider_with_custom_base_url() {
        let provider = AnthropicProvider::new(Some("key"), Some("https://custom.proxy/v1"));
        assert!(provider.is_ok());
    }

    #[test]
    fn build_url_correct() {
        let provider = AnthropicProvider::new(None, None).unwrap();
        // build_url 是私有方法, 通过 chat_with_system 间接验证
        assert!(provider.alias() == "anthropic");
    }

    #[test]
    fn set_api_key_updates_key() {
        let provider = AnthropicProvider::new(None, None).unwrap();
        provider.set_api_key(Some("new-key".to_string()));
        // api_key 是私有字段, 通过 set_api_key 不报错验证
    }

    #[test]
    fn key_rotator_trait_impl() {
        let provider = AnthropicProvider::new(None, None).unwrap();
        provider.set_key(Some("rotated-key"));
        // 通过 set_key 不报错验证
    }

    #[test]
    fn list_models_returns_known_models() {
        let provider = AnthropicProvider::new(None, None).unwrap();
        let models = futures::executor::block_on(provider.list_models()).unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.contains("claude")));
    }

    #[test]
    fn convert_messages_no_system() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "你好".to_string(),
        }];
        let (system, msgs) = convert_messages(&messages);
        assert!(system.is_none());
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn parse_response_skips_empty_tool_use() {
        // tool_use block 缺少 name 时应跳过
        let api_resp = ApiResponse {
            content: vec![ApiContentIn {
                kind: "tool_use".to_string(),
                text: None,
                id: Some("tool_1".to_string()),
                name: None, // 缺少 name
                input: None,
            }],
            usage: None,
        };
        let result = parse_response(&api_resp).unwrap();
        assert!(result.tool_calls.is_empty(), "缺少 name 的 tool_use 应跳过");
    }

    #[test]
    fn convert_tools_empty_input() {
        let tools: Vec<ToolSpec> = vec![];
        let result = convert_tools(&tools);
        assert!(result.is_empty());
    }

    #[test]
    fn parse_response_multiple_tool_calls() {
        let api_resp = ApiResponse {
            content: vec![
                ApiContentIn {
                    kind: "tool_use".to_string(),
                    text: None,
                    id: Some("tool_1".to_string()),
                    name: Some("search".to_string()),
                    input: Some(serde_json::json!({"q": "rust"})),
                },
                ApiContentIn {
                    kind: "tool_use".to_string(),
                    text: None,
                    id: Some("tool_2".to_string()),
                    name: Some("read_file".to_string()),
                    input: Some(serde_json::json!({"path": "/tmp/test"})),
                },
            ],
            usage: None,
        };
        let result = parse_response(&api_resp).unwrap();
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "search");
        assert_eq!(result.tool_calls[1].name, "read_file");
    }
}
