//! 观察者 trait -- 指标和追踪
use async_trait::async_trait;
use chrono::Duration;
use serde::{Deserialize, Serialize};
use std::any::Any;

use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessageSnapshot {
    pub input: Vec<MessageSnapshot>,
    pub output_text: Option<String>,
    pub output_tool_calls: Vec<ToolCallSnapshot>,
    pub system_instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSnapshot {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSnapshot {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// 观察者事件 -- #[non_exhaustive] 保证外部实现对新变体优雅降级
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ObserverEvent {
    // 启动一个agent循环
    AgentStart {
        model_provider: String,
        model: String,
        channel: Option<String>,
        agent_alias: Option<String>,
        turn_id: Option<String>,
    },
    /// LLM 请求开始
    LlmRequest {
        model_provider: String,
        model: String,
        message_count: usize,
        channel: Option<String>,
        agent_alias: Option<String>,
        turn_id: Option<String>,
    },
    /// LLM 响应完成
    LlmResponse {
        model_provider: String,
        model: String,
        duration: Duration,
        success: bool,
        error_message: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        messages: Option<LlmMessageSnapshot>,
        channel: Option<String>,
        agent_alias: Option<String>,
        turn_id: Option<String>,
    },
    /// agent会话结束 消耗了多少token 使用了多少money
    AgentEnd {
        model_provider: String,
        model: String,
        duration: Duration,
        token_used: Option<TurnTokenUsage>,
        cost_usd: Option<f64>,
        channel: Option<String>,
        agent_alias: Option<String>,
        turn_id: Option<String>,
    },

    /// 工具调用之前
    ToolCallStart {
        tool: String,
        tool_call_id: Option<String>,
        arguments: Option<String>,
        channel: Option<String>,
        agent_alias: Option<String>,
        turn_id: Option<String>,
    },

    /// 工具调用结果
    ToolCall {
        tool: String,
        tool_call_id: Option<String>,
        duration: Duration,
        success: bool,
        arguments: Option<String>,
        result: Option<String>,
        channel: Option<String>,
        agent_alias: Option<String>,
        turn_id: Option<String>,
    },

    /// 记忆召回
    MemoryRecall {
        query_summary: String,
        duration: Duration,
        num_entries: usize,
        backend: String,
        success: bool,
    },

    /// 记忆存储
    MemoryStore {
        category: String,
        backend: String,
        duration: Duration,
        success: bool,
    },
    /// RAG检索完成
    RagRetrieve {
        query_summary: String,
        duration: Duration,
        num_chunks: usize,
        num_boards: usize,
    },
    /// 一问一答 完成
    TurnComplete,
    ///从通道中 发送 或者收到一个消息
    ChannelMessage {
        channel: String,
        /// 方向 `inbound` or `outbound`
        direction: String,
    },

    /// 心跳
    HeartbeatTick,
    /// 响应命中缓存 减少了一次llm call
    CacheHit {
        /// 缓存类型 hot（memory） warm (sqlite)
        cache_type: String,
        /// 能减少多少token消耗
        tokens_saved: u64,
    },
    CacheMiss {
        /// `response`
        cache_type: String,
    },
    /// 错误
    Error { message: String },

    /// 一次发布开始
    DeploymentStart { deploy_id: String },

    /// 一次发布完成
    DeploymentComplete {
        deploy_id: String,
        commit_sha: String,
    },
    /// 一次发布失败
    DeploymentFail { deploy_id: String, reason: String },

    /// 恢复完成
    RecoveryComplete { deploy_id: String },

    /// 历史记录裁剪(丢多少 留多少) 并且能提示用户
    HistoryTrimmed {
        dropped_messages: usize,
        kept_turns: usize,
        reason: String,
        channel: Option<String>,
        agent_alias: Option<String>,
        turn_id: Option<String>,
    },
}

/// 运行时数据指标
#[derive(Debug, Clone)]
pub enum ObserverMetric {
    /// 单次llm或工具请求时长
    RequestLatency(Duration),
    /// 一次llm call 消耗的token
    TokenUsed(u64),
    /// 当前活跃的session数量
    ActiveSessions(u64),
    /// 当前消息队列的长度
    QueueDepth(u64),
    /// 一次需求的交付时间
    DeploymentLeadTime(Duration),
    /// 从部署失败中恢复所花费的时间
    RecoveryTime(Duration),
}

/// 观察者 trait
///
/// 后端实现: Log / Prometheus / OTel (未来)
#[async_trait]
pub trait Observer: Send + Sync + 'static {
    /// 记录事件
    fn record_event(&self, event: &ObserverEvent);

    fn record_metric(&self, metric: &ObserverMetric);

    /// 刷新缓冲
    fn flush(&self) {}

    fn name(&self) -> &str;

    /// as_any 用于 downcast
    fn as_any(&self) -> &dyn Any;
}

impl<T: Observer + ?Sized> Observer for Arc<T> {
    fn record_event(&self, event: &ObserverEvent) {
        self.as_ref().record_event(event)
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        self.as_ref().record_metric(metric)
    }

    fn flush(&self) {
        self.as_ref().flush()
    }

    fn name(&self) -> &str {
        self.as_ref().name()
    }

    fn as_any(&self) -> &dyn Any {
        self.as_ref().as_any()
    }
}
