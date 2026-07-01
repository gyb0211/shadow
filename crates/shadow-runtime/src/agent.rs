//! Agent -- 核心代理, 持有 provider/tool/memory/observer
//!
//! 借鉴 ZeroClaw 的 Agent 设计, 但大幅精简:
//! - ZeroClaw: 30+ 字段, 7874 行
//! - Shadow: ~10 字段, 目标 ~300 行

use shadow_core::{
    Attributable, AutonomyLevel, ChatMessage, ChatRequest, Memory, Provider,
    Observer, ObserverEvent, Role, Tool, ToolCall, ToolResult,
};
use anyhow::Result;
use parking_lot::Mutex;
use shadow_log::Action;
use std::sync::Arc;

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
    pub tools: Vec<Box<dyn Tool>>,
    pub memory: Arc<dyn Memory>,
    pub observer: Arc<dyn Observer>,
    pub config: AgentConfig,
    pub history: Mutex<Vec<ChatMessage>>,
    /// 工具事件回调 (可选) -- CLI 通过此回调接收工具执行通知
    pub tool_event_callback: Option<Arc<dyn ToolEventCallback>>,
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

    /// 单轮对话 (含工具调用循环)
    ///
    /// 流程:
    /// 1. 截断过长的历史 (上下文窗口管理)
    /// 2. 构建消息 (system + history + user)
    /// 3. 调用 LLM
    /// 4. 若响应包含 tool_calls, 执行工具, 将结果追加到消息, 回到步骤 3
    /// 5. 若无 tool_calls, 保存历史并返回最终内容
    pub async fn chat(&self, user_message: &str) -> Result<String> {
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
            tool_calls: vec![],
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
            tool_calls: vec![],
        });

        // 工具调用循环
        #[allow(unused_assignments)]
        let mut final_content = String::new();
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
                tools: self.tools.iter().map(|t| t.spec()).collect(),
            };

            // 调用 provider
            let start = std::time::Instant::now();
            let response = self.provider.chat(request).await?;
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

                // 添加 assistant 消息到历史
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: response.content.clone(),
                    tool_call_id: None,
                    tool_calls: vec![],
                });
                break;
            }

            // 有工具调用: 先添加 assistant 消息 (含 tool_calls)
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.content.clone(),
                tool_call_id: None,
                tool_calls: response.tool_calls.clone(),
            });

            // 执行每个工具调用
            for tool_call in &response.tool_calls {
                shadow_log::record!(
                    INFO,
                    Action::Invoke,
                    format!("调用工具: {} (id: {})", tool_call.name, tool_call.id)
                );

                let tool_start = std::time::Instant::now();
                let result = self.execute_tool_call(tool_call).await;
                let tool_duration_ms = tool_start.elapsed().as_millis() as u64;

                // 记录工具调用事件
                self.observer.record_event(&ObserverEvent::ToolCall {
                    tool: tool_call.name.clone(),
                    success: result.success,
                    duration_ms: tool_duration_ms,
                    output_preview: {
                        let full = if result.success {
                            result.output.clone()
                        } else {
                            result.error.clone().unwrap_or_default()
                        };
                        chars_preview(&full, 200)
                    },
                });

                // 将工具结果添加到消息
                let tool_content = if result.success {
                    result.output
                } else {
                    format!(
                        "[工具执行失败] {}",
                        result.error.unwrap_or_default()
                    )
                };

                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: tool_content,
                    tool_call_id: Some(tool_call.id.clone()),
                    tool_calls: vec![],
                });
            }

            // 继续循环, 将工具结果发给 LLM
        }

        // 去除 <think>...</think> 思考块 (不输出给用户, 也不存入历史)
        let final_content = strip_think_blocks(&final_content);

        // 保存到历史 (只保存 user + 最终 assistant, 不保存中间 tool 消息)
        // 注: 锁必须在 block 内释放, 避免进入 async 状态机导致 future !Send
        {
            let mut history = self.history.lock();
            history.push(ChatMessage {
                role: "user".to_string(),
                content: user_message.to_string(),
                tool_call_id: None,
                tool_calls: vec![],
            });
            history.push(ChatMessage {
                role: "assistant".to_string(),
                content: final_content.clone(),
                tool_call_id: None,
                tool_calls: vec![],
            });
        }

        shadow_log::record!(INFO, Action::Complete, "agent chat 完成");

        Ok(final_content)
    }

    /// 执行单个工具调用
    ///
    /// 包含以下检查:
    /// 1. 只读模式拒绝
    /// 2. Supervised 模式审批检查 (requires_approval 的工具跳过执行)
    /// 3. 工具超时控制 (tool.timeout() 返回的时长)
    /// 4. 工具事件回调通知
    async fn execute_tool_call(&self, tool_call: &ToolCall) -> ToolResult {
        // 查找匹配的工具
        let tool = self.tools.iter().find(|t| t.name() == tool_call.name);

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

    /// 清空历史
    pub fn clear_history(&self) {
        self.history.lock().clear();
    }
}

/// Agent 构建器
#[derive(Default)]
pub struct AgentBuilder {
    alias: Option<String>,
    provider: Option<Arc<dyn Provider>>,
    tools: Option<Vec<Box<dyn Tool>>>,
    memory: Option<Arc<dyn Memory>>,
    observer: Option<Arc<dyn Observer>>,
    config: Option<AgentConfig>,
    tool_event_callback: Option<Arc<dyn ToolEventCallback>>,
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
    pub fn tools(mut self, tools: Vec<Box<dyn Tool>>) -> Self {
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

        Ok(Agent {
            alias,
            provider,
            tools,
            memory,
            observer,
            config,
            history: Mutex::new(Vec::new()),
            tool_event_callback,
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

/// 去除 `<think>...</think>` 思考块 -- 模型链式推理内容不输出给用户, 也不存入历史
///
/// 处理:
/// - 已闭合的 `<think>...</think>` (可多个)
/// - 未闭合的 `<think>` (流式残余: 从标签处截断到末尾)
fn strip_think_blocks(content: &str) -> String {
    let mut result = content.to_string();
    loop {
        match result.find("<think>") {
            Some(start) => match result[start..].find("</think>") {
                Some(end_rel) => {
                    let end_abs = start + end_rel + "</think>".len();
                    result.replace_range(start..end_abs, "");
                }
                None => {
                    // 未闭合 <think>: 删除到末尾
                    result.truncate(start);
                    break;
                }
            },
            None => break,
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod think_tests {
    use super::*;

    #[test]
    fn strip_closed_think_block() {
        let input = "<think>分析一下</think>答案是 42";
        assert_eq!(strip_think_blocks(input), "答案是 42");
    }

    #[test]
    fn strip_multiple_think_blocks() {
        let input = "<think>第一段</think>你好<think>第二段</think>世界";
        assert_eq!(strip_think_blocks(input), "你好世界");
    }

    #[test]
    fn strip_unclosed_think_block() {
        let input = "答案是 42<think>还没想完";
        assert_eq!(strip_think_blocks(input), "答案是 42");
    }

    #[test]
    fn no_think_block_unchanged() {
        assert_eq!(strip_think_blocks("普通回复"), "普通回复");
    }

    #[test]
    fn multiline_think_block() {
        let input = "<think>\nlet me think...\ndeeply\n</think>\n\n最终答案";
        assert_eq!(strip_think_blocks(input), "最终答案");
    }
}
