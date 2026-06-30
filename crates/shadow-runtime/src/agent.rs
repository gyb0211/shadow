//! Agent -- 核心代理, 持有 provider/tool/memory/observer
//!
//! 借鉴 ZeroClaw 的 Agent 设计, 但大幅精简:
//! - ZeroClaw: 30+ 字段, 7874 行
//! - Shadow: ~10 字段, 目标 ~300 行

use agent_core::{
    Attributable, AutonomyLevel, ChatMessage, ChatRequest, ChatResponse, Memory, ModelProvider,
    Observer, ObserverEvent, Role, Tool, ToolResult,
};
use anyhow::Result;
use parking_lot::Mutex;
use std::sync::Arc;

/// Agent 配置
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub alias: String,
    pub model_provider_type: String,
    pub model: String,
    pub temperature: Option<f64>,
    pub autonomy: AutonomyLevel,
    pub workspace_dir: std::path::PathBuf,
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

    /// 单轮对话
    pub async fn chat(&self, user_message: &str) -> Result<String> {
        // 记录 LLM 请求
        self.observer
            .record_event(&ObserverEvent::LlmRequest { model: self.config.model.clone(), message_count: 1 });

        // 构建消息
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: "你是一个有用的 AI 助手.".to_string(),
            tool_call_id: None,
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
        });

        // 构建请求
        let request = ChatRequest {
            messages,
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

        // 保存到历史
        let mut history = self.history.lock();
        history.push(ChatMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
            tool_call_id: None,
        });
        history.push(ChatMessage {
            role: "assistant".to_string(),
            content: response.content.clone(),
            tool_call_id: None,
        });
        drop(history);

        Ok(response.content)
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
