//! 影子核心 trait 层 -- 微内核 ABI
//!
//! 所有其他 crate 依赖此 crate, 此 crate 不依赖任何其他内部 crate。
//! 这是微内核架构的 "ABI 层"。
//!
//! 核心导出:
//!   - [`ModelProvider`]       LLM 推理后端
//!   - [`Memory`]         长期记忆存储
//!   - [`Tool`]           Agent 可调用工具
//!   - [`Channel`]        消息平台渠道
//!   - [`Observer`]       指标观察者
//!   - [`AgentRuntime`]   Agent 主循环 (新)
//!   - [`SessionStore`]   会话持久化 (新)
//!   - [`Attributable`]   归因系统

pub mod channel;
pub mod kennel;
pub mod runtime;
pub mod session_store;
pub mod workspace;
pub mod platform;

pub use channel::{Channel, ChannelMessage, SendMessage};
pub use kennel::attribution::*;
pub use kennel::memory::{
    ExportFilter, Memory, MemoryCategory, MemoryEntry, MemoryKind, MemoryStats, MemoryStrategy,
    ProceduralMessage, StoreOptions,
};
pub use kennel::observer::{Observer, ObserverEvent};
pub use kennel::provider::{
    AuthStyle, ChatMessage, ChatRequest, ChatResponse, ModelInfo, ModelProvider,
    ModelProviderRuntimeOptions, ProviderCapabilities, StreamChunk, StreamError, StreamEvent,
    StreamOptions, TokenUsage, ToolCall, ToolsPayload,
};
pub use kennel::tool::{Tool, ToolResult, ToolSpec};
pub use session_store::{JsonlSessionStore, Session, SessionMetadata, SessionStore};
pub use workspace::Workspace;
use crate::kennel::provider::NativeThinkingParams;

/// 代理自主级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AutonomyLevel {
    /// 完全自主, 无需审批
    Full,
    /// 受监督, 敏感操作需审批
    #[default]
    Supervised,
    /// 只读, 写操作被拒绝
    ReadOnly,
}

tokio::task_local! {
    pub static TOOL_LOOP_THREAD_ID: Option<String>;
    pub static TOOL_CHOICE_OVERRIDE:  Option<String>;
    pub static TOOL_LOOP_SESSION_KEY:  Option<String>;
    pub static NARIVE_THINKING_OVERRIDE:  Option<NativeThinkingParams>;
}
