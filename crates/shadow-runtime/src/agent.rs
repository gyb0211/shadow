//! Agent -- 核心代理, 持有 provider/tool/memory/observer
//!
//! 借鉴 ZeroClaw 的 Agent 设计, 但大幅精简:
//! - ZeroClaw: 30+ 字段, 7874 行
//! - Shadow: ~10 字段, 目标 ~300 行

use agent_core::{
    Attributable, AutonomyLevel, ChatMessage, ChatRequest, Memory, ModelProvider,
    Observer, ObserverEvent, Role, Tool, ToolCall, ToolResult,
};
use anyhow::Result;
use parking_lot::Mutex;
use shadow_log::Action;
use std::sync::Arc;

/// 工具调用最大循环次数 -- 默认值, 可被 AgentConfig.max_iterations 覆盖
const DEFAULT_MAX_ITERATIONS: usize = 10;

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
        }
    }
}

/// Agent -- 核心代理
pub struct Agent {
    pub alias: String,
    pub provider: Arc<dyn ModelProvider>,
    pub tools: Vec<Box<dyn Tool>>,
    pub memory: Arc<dyn Memory>,
    pub observer: Arc<dyn Observer>,
    pub config: AgentConfig,
    pub history: Mutex<Vec<ChatMessage>>,
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

    /// 单轮对话 (含工具调用循环)
    ///
    /// 流程:
    /// 1. 构建消息 (system + history + user)
    /// 2. 调用 LLM
    /// 3. 若响应包含 tool_calls, 执行工具, 将结果追加到消息, 回到步骤 2
    /// 4. 若无 tool_calls, 保存历史并返回最终内容
    pub async fn chat(&self, user_message: &str) -> Result<String> {
        // 记录会话开始
        shadow_log::record!(INFO, Action::Start, "agent chat 开始");

        // 构建初始消息
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: "你是一个有用的 AI 助手. 你可以使用工具来完成任务.".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
        }];

        // 加载历史
        let history = self.history.lock();
        messages.extend(history.iter().cloned());
        drop(history);

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

        // 保存到历史 (只保存 user + 最终 assistant, 不保存中间 tool 消息)
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
        drop(history);

        shadow_log::record!(INFO, Action::Complete, "agent chat 完成");

        Ok(final_content)
    }

    /// 执行单个工具调用
    async fn execute_tool_call(&self, tool_call: &ToolCall) -> ToolResult {
        // 查找匹配的工具
        let tool = self.tools.iter().find(|t| t.name() == tool_call.name);

        match tool {
            Some(t) => {
                // 检查自主级别
                if self.config.autonomy == AutonomyLevel::ReadOnly {
                    return ToolResult::err("只读模式: 工具执行被拒绝");
                }

                // 执行工具
                match t.execute(tool_call.arguments.clone()).await {
                    Ok(result) => result,
                    Err(e) => ToolResult::err(format!("工具执行异常: {e}")),
                }
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
    provider: Option<Arc<dyn ModelProvider>>,
    tools: Option<Vec<Box<dyn Tool>>>,
    memory: Option<Arc<dyn Memory>>,
    observer: Option<Arc<dyn Observer>>,
    config: Option<AgentConfig>,
}

impl AgentBuilder {
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }
    pub fn provider(mut self, provider: Arc<dyn ModelProvider>) -> Self {
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

    /// 构建 Agent
    pub fn build(self) -> Result<Agent> {
        let config = self.config.unwrap_or_default();
        let alias = self.alias.unwrap_or_else(|| config.alias.clone());
        let provider = self
            .provider
            .ok_or_else(|| anyhow::anyhow!("缺少 provider, 请通过 .provider() 设置"))?;
        let memory = self
            .memory
            .unwrap_or_else(|| Arc::new(agent_core::NoneMemory));
        let observer = self
            .observer
            .unwrap_or_else(|| Arc::new(agent_core::NoopObserver));
        let tools = self.tools.unwrap_or_default();

        Ok(Agent {
            alias,
            provider,
            tools,
            memory,
            observer,
            config,
            history: Mutex::new(Vec::new()),
        })
    }
}
