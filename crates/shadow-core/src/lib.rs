//! 影子核心 trait 层 -- 微内核 ABI
//!
//! 所有其他 crate 依赖此 crate, 此 crate 不依赖任何其他内部 crate。
//! 这是微内核架构的 "ABI 层"。
//!
//! 核心导出:
//!   - [`ModelProvider`]  LLM 推理后端
//!   - [`Memory`]         长期记忆存储
//!   - [`Tool`]           Agent 可调用工具
//!   - [`Channel`]        消息平台渠道
//!   - [`Observer`]       指标观察者
//!   - [`AgentRuntime`]   Agent 主循环 (新)
//!   - [`SessionStore`]   会话持久化 (新)
//!   - [`Attributable`]   归因系统

pub mod agent_runtime;
pub mod attribution;
pub mod channel;
pub mod memory;
pub mod observer;
pub mod provider;
pub mod session_store;
pub mod tool;

pub use agent_runtime::AgentRuntime;
pub use attribution::{Attributable, Role};
pub use channel::{Channel, ChannelMessage, CliChannel, SendMessage};
pub use memory::{Memory, MemoryEntry, NoneMemory};
pub use observer::{NoopObserver, Observer, ObserverEvent};
pub use provider::{
    AuthStyle, ChatMessage, ChatRequest, ChatResponse, ModelProvider, ModelProviderRuntimeOptions,
    TokenUsage, ToolCall,
};
pub use session_store::{JsonlSessionStore, Session, SessionStore};
pub use tool::{Tool, ToolAttribution, ToolResult, ToolSpec};

/// 代理自主级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyLevel {
    /// 完全自主, 无需审批
    Full,
    /// 受监督, 敏感操作需审批
    #[default]
    Supervised,
    /// 只读, 写操作被拒绝
    ReadOnly,
}
