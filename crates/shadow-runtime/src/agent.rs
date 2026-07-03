//! Agent -- 核心代理, 持有 provider/tool/memory/observer
//!
//! 借鉴 ZeroClaw 的 Agent 设计, 但大幅精简:
//! - ZeroClaw: 30+ 字段, 7874 行
//! - Shadow: ~10 字段, 目标 ~300 行

use shadow_core::{
    Attributable, AutonomyLevel, ChatChunk, ChatMessage, ChatRequest, ChatResponse, Memory,
    Provider, Observer, ObserverEvent, Role, Session, SessionStore, TokenUsage, ToolCall,
    ToolResult,
};
use anyhow::Result;
use futures::stream::BoxStream;
use futures::StreamExt;
use parking_lot::Mutex;
use shadow_log::Action;
use std::sync::Arc;

use crate::tools::ToolRegistry;

/// 工具调用最大循环次数 -- 默认值, 可被 AgentConfig.max_iterations 覆盖
const DEFAULT_MAX_ITERATIONS: usize = 10;

/// 对话历史最大条数 -- 默认值, 可被 AgentConfig.max_history 覆盖
const DEFAULT_MAX_HISTORY: usize = 50;

/// 默认 system prompt
const DEFAULT_SYSTEM_PROMPT: &str = "你是一个有用的 AI 助手. 你可以使用工具来完成任务.";

/// 工具事件回调 trait -- CLI 通过此回调接收工具执行事件通知
///
/// 两个 &str 参数: (事件类型, 详情)
/// 事件类型: "tool_start" / "tool_success" / "tool_error" / "tool_timeout" / "tool_approval_skipped"
/// 详情: 工具名称或错误信息
pub trait ToolEventCallback: Fn(&str, &str) + Send + Sync {}

// 为所有满足 Fn(&str, &str) + Send + Sync 的类型自动实现 ToolEventCallback
impl<T> ToolEventCallback for T where T: Fn(&str, &str) + Send + Sync {}

/// 流式增量类型 -- 区分回答和思考
#[derive(Debug, Clone)]
pub enum StreamDelta {
    /// 回答增量
    Content(String),
    /// 思考增量
    Reasoning(String),
}

/// 流式增量回调 trait -- TUI/CLI 通过此回调接收逐字增量
///
/// 参数: StreamDelta (回答或思考的增量片段)
pub trait StreamDeltaCallback: Fn(StreamDelta) + Send + Sync {}

// 为所有满足 Fn(StreamDelta) + Send + Sync 的类型自动实现 StreamDeltaCallback
impl<T> StreamDeltaCallback for T where T: Fn(StreamDelta) + Send + Sync {}

/// Agent 配置
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub alias: String,
    pub model_provider_type: String,
    pub model: String,
    pub temperature: Option<f64>,
    pub autonomy: AutonomyLevel,
    pub workspace_dir: std::path::PathBuf,
    /// 工具调用最大循环次数 (默认 10)
    pub max_iterations: usize,
    /// 对话历史最大条数 (超过时自动截断旧消息, 默认 50)
    pub max_history: usize,
    /// 自定义 system prompt (None 则使用默认)
    pub system_prompt: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            alias: "default".to_string(),
            model_provider_type: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            temperature: Some(0.7),
            autonomy: AutonomyLevel::default(),
            workspace_dir: std::path::PathBuf::from("."),
            max_iterations: DEFAULT_MAX_ITERATIONS,
            max_history: DEFAULT_MAX_HISTORY,
            system_prompt: None,
        }
    }
}

/// Agent -- 核心代理
pub struct Agent {
    pub alias: String,
    pub provider: Arc<dyn Provider>,
    pub tools: ToolRegistry,
    pub memory: Arc<dyn Memory>,
    pub observer: Arc<dyn Observer>,
    pub config: AgentConfig,
    pub history: Mutex<Vec<ChatMessage>>,
    /// 工具事件回调 (可选) -- CLI 通过此回调接收工具执行通知
    pub tool_event_callback: Option<Arc<dyn ToolEventCallback>>,
    /// 会话存储 (可选) -- 用于持久化对话历史
    pub session_store: Option<Arc<dyn SessionStore>>,
    /// 当前会话 ID (内存中维护, 通过 list() 修改时间动态确定)
    current_session_id: Mutex<Option<String>>,
}

impl Attributable for Agent {
    fn role(&self) -> Role {
        Role::Agent
    }
    fn alias(&self) -> &str {
        &self.alias
    }
}

impl Agent {
    /// 构建器
    pub fn builder() -> AgentBuilder {
        AgentBuilder::default()
    }

    /// 获取当前生效的 system prompt
    /// 优先使用 config.system_prompt, 未设则用默认
    fn system_prompt(&self) -> &str {
        self.config
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_SYSTEM_PROMPT)
    }

    /// 通知工具事件回调 (若已设置)
    fn notify_tool_event(&self, event: &str, detail: &str) {
        if let Some(cb) = &self.tool_event_callback {
            cb(event, detail);
        }
    }

    /// 单轮对话 (含工具调用循环) -- 不带流式回调
    ///
    /// 等价于 `chat_with_stream(user_message, None)`
    pub async fn chat(&self, user_message: &str) -> Result<String> {
        self.chat_with_stream(user_message, None).await
    }

    /// 单轮对话 (含工具调用循环) -- 带流式增量回调
    ///
    /// 流程:
    /// 1. 截断过长的历史 (上下文窗口管理)
    /// 2. 构建消息 (system + history + user)
    /// 3. 调用 LLM (流式 -- 每个 ContentDelta/ReasoningDelta 调用 on_delta 回调)
    /// 4. 若响应包含 tool_calls, 执行工具, 将结果追加到消息, 回到步骤 3
    /// 5. 若无 tool_calls, 保存历史并返回最终内容
    pub async fn chat_with_stream(
        &self,
        user_message: &str,
        on_delta: Option<Arc<dyn StreamDeltaCallback>>,
    ) -> Result<String> {
        // 记录会话开始
        shadow_log::record!(INFO, Action::Start, "agent chat 开始");

        // 上下文窗口管理: 截断过长的历史, 保留最近 max_history 条
        // 注: 锁必须在 block 内释放, 避免进入 async 状态机导致 future !Send
        let truncated = {
            let mut history = self.history.lock();
            if history.len() > self.config.max_history {
                let drain_count = history.len() - self.config.max_history;
                history.drain(0..drain_count);
                Some(drain_count)
            } else {
                None
            }
        };
        if let Some(drain_count) = truncated {
            shadow_log::record!(
                INFO,
                Action::Note,
                format!(
                    "历史超过 max_history({}), 截断 {} 条旧消息",
                    self.config.max_history, drain_count
                )
            );
        }

        // 构建初始消息 -- system prompt (从 config 读取, 未设则用默认)
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: self.system_prompt().to_string(),
            tool_call_id: None,
            ..Default::default()
        }];

        // 加载历史 (锁在 block 内释放, 避免 future !Send)
        {
            let history = self.history.lock();
            messages.extend(history.iter().cloned());
        }

        // 添加用户消息
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
            tool_call_id: None,
            ..Default::default()
        });

        // 工具调用循环
        #[allow(unused_assignments)]
        let mut final_content = String::new();
        let mut final_reasoning: Option<String> = None;
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > self.config.max_iterations {
                shadow_log::record!(
                    WARN,
                    Action::Fail,
                    "工具调用超过最大循环次数, 终止"
                );
                final_content = "工具调用次数超过上限, 终止对话.".to_string();
                break;
            }

            // 记录 LLM 请求
            self.observer.record_event(&ObserverEvent::LlmRequest {
                model: self.config.model.clone(),
                message_count: messages.len(),
            });

            // 构建请求
            let request = ChatRequest {
                messages: messages.clone(),
                model: self.config.model.clone(),
                temperature: self.config.temperature,
                max_tokens: None,
                tools: self.tools.specs(),
            };

            // 调用 provider (流式)
            let start = std::time::Instant::now();
            let stream = self.provider.chat_stream(request).await?;
            let response = consume_stream(stream, on_delta.as_ref()).await?;
            let duration_ms = start.elapsed().as_millis() as u64;

            // 记录 LLM 响应
            self.observer.record_event(&ObserverEvent::LlmResponse {
                model: self.config.model.clone(),
                duration_ms,
                tokens: response.usage.total_tokens,
            });

            // 判断是否有工具调用
            if response.tool_calls.is_empty() {
                // 无工具调用, 对话结束
                final_content = response.content.clone();
                final_reasoning = response.reasoning_content.clone();

                // 添加 assistant 消息到历史
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: response.content.clone(),
                    tool_call_id: None,
                    ..Default::default()
                });
                break;
            }

            // 有工具调用: 先添加 assistant 消息 (含 tool_calls + reasoning_content)
            // 注: reasoning_content 必须回传, 部分 provider 拒绝缺少此字段的 tool-call 历史
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.content.clone(),
                tool_call_id: None,
                tool_calls: response.tool_calls.clone(),
                reasoning_content: response.reasoning_content.clone(),
            });

            // 执行工具调用 -- 检查是否有工具需要审批
            // 如果任何工具 requires_approval, 全部串行执行 (安全第一)
            let any_needs_approval = response.tool_calls.iter().any(|tc| {
                self.tools
                    .find(&tc.name)
                    .map(|t| t.requires_approval())
                    .unwrap_or(false)
            });

            if any_needs_approval {
                // 串行执行 (有工具需要审批, 逐个执行)
                for tool_call in &response.tool_calls {
                    shadow_log::record!(
                        INFO,
                        Action::Invoke,
                        format!("调用工具: {} (id: {})", tool_call.name, tool_call.id)
                    );

                    let tool_start = std::time::Instant::now();
                    let result = self.execute_tool_call(tool_call).await;
                    let tool_duration_ms = tool_start.elapsed().as_millis() as u64;

                    // 记录工具调用事件 (脱敏后)
                    self.record_tool_event(
                        &tool_call.name,
                        &result,
                        tool_duration_ms,
                    );

                    // 将工具结果添加到消息 (脱敏后)
                    let tool_content = if result.success {
                        scrub_credentials(&result.output)
                    } else {
                        format!(
                            "[工具执行失败] {}",
                            scrub_credentials(&result.error.unwrap_or_default())
                        )
                    };

                    messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: tool_content,
                        tool_call_id: Some(tool_call.id.clone()),
                        ..Default::default()
                    });
                }
            } else {
                // 并行执行 (无审批需求, 使用 join_all 并发执行)
                shadow_log::record!(
                    INFO,
                    Action::Invoke,
                    format!("并行执行 {} 个工具", response.tool_calls.len())
                );

                let tool_calls = &response.tool_calls;
                let futures: Vec<_> = tool_calls
                    .iter()
                    .map(|tc| async move {
                        let tool_start = std::time::Instant::now();
                        let result = self.execute_tool_call(tc).await;
                        let tool_duration_ms = tool_start.elapsed().as_millis() as u64;
                        (tc, result, tool_duration_ms)
                    })
                    .collect();
                let results = futures::future::join_all(futures).await;

                // 按顺序处理结果
                for (tool_call, result, tool_duration_ms) in results {
                    // 记录工具调用事件 (脱敏后)
                    self.record_tool_event(
                        &tool_call.name,
                        &result,
                        tool_duration_ms,
                    );

                    // 将工具结果添加到消息 (脱敏后)
                    let tool_content = if result.success {
                        scrub_credentials(&result.output)
                    } else {
                        format!(
                            "[工具执行失败] {}",
                            scrub_credentials(&result.error.unwrap_or_default())
                        )
                    };

                    messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: tool_content,
                        tool_call_id: Some(tool_call.id.clone()),
                        ..Default::default()
                    });
                }
            }

            // 继续循环, 将工具结果发给 LLM
        }

        // 保留原始内容 (含 <think> 标签), 由显示层决定是否过滤
        // reasoning_content (DeepSeek-R1 等 API 独立字段) 也保留
        let final_reasoning = final_reasoning.take();

        // 保存到历史 (只保存 user + 最终 assistant, 不保存中间 tool 消息)
        // 注: 锁必须在 block 内释放, 避免进入 async 状态机导致 future !Send
        {
            let mut history = self.history.lock();
            history.push(ChatMessage {
                role: "user".to_string(),
                content: user_message.to_string(),
                tool_call_id: None,
                ..Default::default()
            });
            history.push(ChatMessage {
                role: "assistant".to_string(),
                content: final_content.clone(),
                tool_call_id: None,
                reasoning_content: final_reasoning.clone(),
                ..Default::default()
            });
        }

        // 保存到 session store (追加新的 user + assistant 消息)
        if let Some(store) = &self.session_store {
            // 获取或生成会话 ID
            let session_id = {
                let mut sid = self.current_session_id.lock();
                if sid.is_none() {
                    *sid = Some(uuid::Uuid::new_v4().to_string());
                }
                sid.clone()
            };
            if let Some(id) = session_id {
                let session = Session {
                    id,
                    messages: vec![
                        ChatMessage {
                            role: "user".to_string(),
                            content: user_message.to_string(),
                            tool_call_id: None,
                            ..Default::default()
                        },
                        ChatMessage {
                            role: "assistant".to_string(),
                            content: final_content.clone(),
                            tool_call_id: None,
                            reasoning_content: final_reasoning.clone(),
                            ..Default::default()
                        },
                    ],
                };
                if let Err(e) = store.save(&session).await {
                    shadow_log::record!(
                        WARN,
                        Action::Fail,
                        format!("保存会话失败: {e}")
                    );
                }
            }
        }

        shadow_log::record!(INFO, Action::Complete, "agent chat 完成");

        Ok(final_content)
    }

    /// 记录工具调用事件到 observer (脱敏后)
    fn record_tool_event(&self, tool_name: &str, result: &ToolResult, duration_ms: u64) {
        let full = if result.success {
            result.output.clone()
        } else {
            result.error.clone().unwrap_or_default()
        };
        // 脱敏后截断预览
        let scrubbed = scrub_credentials(&full);
        let preview = chars_preview(&scrubbed, 200);

        self.observer.record_event(&ObserverEvent::ToolCall {
            tool: tool_name.to_string(),
            success: result.success,
            duration_ms,
            output_preview: preview,
        });
    }

    /// 执行单个工具调用
    ///
    /// 包含以下检查:
    /// 1. 只读模式拒绝
    /// 2. Supervised 模式审批检查 (requires_approval 的工具跳过执行)
    /// 3. 工具超时控制 (tool.timeout() 返回的时长)
    /// 4. 工具事件回调通知
    async fn execute_tool_call(&self, tool_call: &ToolCall) -> ToolResult {
        // 查找匹配的工具 (通过 ToolRegistry)
        let tool = self.tools.find(&tool_call.name);

        match tool {
            Some(t) => {
                // 检查自主级别 -- 只读模式拒绝所有工具
                if self.config.autonomy == AutonomyLevel::ReadOnly {
                    return ToolResult::err("只读模式: 工具执行被拒绝");
                }

                // 审批检查: Supervised 模式下, 需要审批的工具跳过执行
                if self.config.autonomy == AutonomyLevel::Supervised && t.requires_approval() {
                    let msg = format!(
                        "工具 [{}] 需要用户审批 (supervised 模式), 已跳过执行",
                        tool_call.name
                    );
                    println!("{msg}");
                    self.notify_tool_event("tool_approval_skipped", &tool_call.name);
                    return ToolResult::err("需要用户审批 (supervised 模式)");
                }

                // 通知工具开始执行
                self.notify_tool_event("tool_start", &tool_call.name);

                // 获取工具超时配置 -- None 表示不限制
                let timeout = t.timeout();

                // 执行工具 (带超时控制)
                let exec_future = t.execute(tool_call.arguments.clone());

                let result = match timeout {
                    Some(d) => {
                        // 有超时: 用 tokio::time::timeout 包装
                        match tokio::time::timeout(d, exec_future).await {
                            Ok(Ok(result)) => result,
                            Ok(Err(e)) => ToolResult::err(format!("工具执行异常: {e}")),
                            Err(_) => {
                                let msg = format!(
                                    "工具 [{}] 执行超时 ({}ms)",
                                    tool_call.name,
                                    d.as_millis()
                                );
                                println!("{msg}");
                                self.notify_tool_event("tool_timeout", &tool_call.name);
                                ToolResult::err("工具执行超时")
                            }
                        }
                    }
                    None => {
                        // 无超时: 直接执行
                        match exec_future.await {
                            Ok(result) => result,
                            Err(e) => ToolResult::err(format!("工具执行异常: {e}")),
                        }
                    }
                };

                // 通知工具执行结果
                if result.success {
                    self.notify_tool_event("tool_success", &tool_call.name);
                } else {
                    let detail = format!(
                        "{}: {}",
                        tool_call.name,
                        result.error.as_deref().unwrap_or("未知错误")
                    );
                    self.notify_tool_event("tool_error", &detail);
                }

                result
            }
            None => ToolResult::err(format!("未找到工具: {}", tool_call.name)),
        }
    }

    /// 清空历史 (同时删除 session store 中的当前会话)
    pub async fn clear_history(&self) {
        self.history.lock().clear();

        // 同时清除 session store 中的当前会话
        if let Some(store) = &self.session_store {
            let sid = self.current_session_id.lock().take();
            if let Some(id) = sid {
                if let Err(e) = store.delete(&id).await {
                    shadow_log::record!(
                        WARN,
                        Action::Fail,
                        format!("删除会话失败: {e}")
                    );
                }
            }
        }
    }

    /// 从 session store 加载最近的会话历史到 self.history
    ///
    /// 通过 list() 按修改时间降序排序, 取第一个 (最近修改的) 会话.
    /// 如果没有已保存的会话, 不做任何操作.
    pub async fn load_history(&self) -> Result<()> {
        let Some(store) = &self.session_store else {
            return Ok(());
        };

        let sessions = store.list().await?;
        if sessions.is_empty() {
            return Ok(());
        }

        // list() 按修改时间降序排序, 第一个是最新的
        let latest_id = &sessions[0];
        if let Some(session) = store.load(latest_id).await? {
            {
                let mut history = self.history.lock();
                history.clear();
                history.extend(session.messages);
            }
            let mut sid = self.current_session_id.lock();
            *sid = Some(session.id);
        }

        Ok(())
    }

    /// 返回当前会话 ID
    ///
    /// 会话 ID 在 load_history() 时从最新修改的会话确定,
    /// 或在首次 chat() 时生成新的 UUID.
    #[must_use]
    pub fn current_session_id(&self) -> Option<String> {
        self.current_session_id.lock().clone()
    }
}

/// Agent 构建器
#[derive(Default)]
pub struct AgentBuilder {
    alias: Option<String>,
    provider: Option<Arc<dyn Provider>>,
    tools: Option<ToolRegistry>,
    memory: Option<Arc<dyn Memory>>,
    observer: Option<Arc<dyn Observer>>,
    config: Option<AgentConfig>,
    tool_event_callback: Option<Arc<dyn ToolEventCallback>>,
    session_store: Option<Arc<dyn SessionStore>>,
}

impl AgentBuilder {
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }
    pub fn provider(mut self, provider: Arc<dyn Provider>) -> Self {
        self.provider = Some(provider);
        self
    }
    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(tools);
        self
    }
    pub fn memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = Some(memory);
        self
    }
    pub fn observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }
    pub fn config(mut self, config: AgentConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// 设置自定义 system prompt
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        let config = self.config.get_or_insert_with(AgentConfig::default);
        config.system_prompt = Some(prompt.into());
        self
    }

    /// 设置对话历史最大条数
    pub fn max_history(mut self, max: usize) -> Self {
        let config = self.config.get_or_insert_with(AgentConfig::default);
        config.max_history = max;
        self
    }

    /// 设置工具事件回调
    pub fn tool_event_callback(mut self, callback: Arc<dyn ToolEventCallback>) -> Self {
        self.tool_event_callback = Some(callback);
        self
    }

    /// 设置会话存储 (用于持久化对话历史)
    pub fn session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    /// 构建 Agent
    pub fn build(self) -> Result<Agent> {
        let config = self.config.unwrap_or_default();
        let alias = self.alias.unwrap_or_else(|| config.alias.clone());
        let provider = self
            .provider
            .ok_or_else(|| anyhow::anyhow!("缺少 provider, 请通过 .provider() 设置"))?;
        let memory = self
            .memory
            .unwrap_or_else(|| Arc::new(shadow_core::NoneMemory));
        let observer = self
            .observer
            .unwrap_or_else(|| Arc::new(shadow_core::NoopObserver));
        let tools = self.tools.unwrap_or_default();
        let tool_event_callback = self.tool_event_callback;
        let session_store = self.session_store;

        Ok(Agent {
            alias,
            provider,
            tools,
            memory,
            observer,
            config,
            history: Mutex::new(Vec::new()),
            tool_event_callback,
            session_store,
            current_session_id: Mutex::new(None),
        })
    }
}

/// 截断字符串到最多 n 个字符 (按 char, 非 byte), 超出加 "..."
fn chars_preview(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push_str("...");
    }
    out
}

/// 凭证脱敏 -- 替换文本中的 API key / Bearer token / token= 等敏感信息
///
/// 匹配模式:
/// - `sk-xxx` (20+ 字符): OpenAI 风格 API key
/// - `Bearer xxx` (20+ 字符): HTTP Bearer token
/// - `token=xxx` / `token:xxx` (20+ 字符): query/header token
///
/// 替换为对应的 `***` 占位符, 防止敏感信息泄露到日志和 observer 事件中.
fn scrub_credentials(text: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;

    // 正则编译较慢, 使用 OnceLock 缓存
    static RE_SK: OnceLock<Regex> = OnceLock::new();
    static RE_BEARER: OnceLock<Regex> = OnceLock::new();
    static RE_TOKEN: OnceLock<Regex> = OnceLock::new();

    let re_sk = RE_SK.get_or_init(|| {
        Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap()
    });
    let re_bearer = RE_BEARER.get_or_init(|| {
        Regex::new(r"Bearer\s+[a-zA-Z0-9._-]{20,}").unwrap()
    });
    let re_token = RE_TOKEN.get_or_init(|| {
        Regex::new(r"token[=:]\s*[a-zA-Z0-9]{20,}").unwrap()
    });

    let result = re_sk.replace_all(text, "sk-***");
    let result = re_bearer.replace_all(&result, "Bearer ***");
    let result = re_token.replace_all(&result, "token=***");

    result.into_owned()
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    #[test]
    fn scrub_sk_key() {
        let input = "my key is sk-abcdefghijklmnopqrstuvwxyz1234567890";
        let scrubbed = scrub_credentials(input);
        assert!(scrubbed.contains("sk-***"));
        assert!(!scrubbed.contains("sk-abcdefgh"));
    }

    #[test]
    fn scrub_bearer_token() {
        let input = "Authorization: Bearer abcdefghijklmnopqrstuvwxyz1234567890";
        let scrubbed = scrub_credentials(input);
        assert!(scrubbed.contains("Bearer ***"));
    }

    #[test]
    fn scrub_token_equals() {
        let input = "token=abcdefghijklmnopqrstuvwxyz1234567890";
        let scrubbed = scrub_credentials(input);
        assert!(scrubbed.contains("token=***"));
    }

    #[test]
    fn scrub_no_match() {
        let input = "普通文本, 无敏感信息";
        assert_eq!(scrub_credentials(input), input);
    }

    #[test]
    fn scrub_short_key_not_matched() {
        // 短于 20 字符的 key 不匹配
        let input = "sk-short";
        assert_eq!(scrub_credentials(input), input);
    }
}

/// 消费流式 ChatChunk 流, 聚合为完整 ChatResponse
///
/// 对每个 ContentDelta 调用 on_delta 回调 (如果提供),
/// 从 Done chunk 获取完整累积结果.
async fn consume_stream(
    mut stream: BoxStream<'static, Result<ChatChunk>>,
    on_delta: Option<&Arc<dyn StreamDeltaCallback>>,
) -> Result<ChatResponse> {
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    let mut usage = TokenUsage::default();
    let mut reasoning_content: Option<String> = None;

    while let Some(chunk_result) = stream.next().await {
        match chunk_result? {
            ChatChunk::ContentDelta(delta) => {
                if let Some(cb) = on_delta {
                    cb(StreamDelta::Content(delta.clone()));
                }
                content.push_str(&delta);
            }
            ChatChunk::ReasoningDelta(delta) => {
                if let Some(cb) = on_delta {
                    cb(StreamDelta::Reasoning(delta.clone()));
                }
                reasoning_content = Some(match reasoning_content.take() {
                    Some(mut rc) => {
                        rc.push_str(&delta);
                        rc
                    }
                    None => delta,
                });
            }
            ChatChunk::ToolCallDelta { .. } => {
                // 工具调用增量已在 provider 内部累积, Done chunk 中包含完整结果
            }
            ChatChunk::Done {
                content: done_content,
                tool_calls: done_tool_calls,
                usage: done_usage,
                reasoning_content: done_reasoning,
            } => {
                content = done_content;
                tool_calls = done_tool_calls;
                usage = done_usage;
                reasoning_content = done_reasoning;
            }
        }
    }

    Ok(ChatResponse {
        content,
        tool_calls,
        usage,
        reasoning_content,
    })
}

