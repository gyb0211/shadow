//! 影子核心层 -- trait 定义与共享类型
//!
//! 所有其他 crate 依赖此 crate, 此 crate 不依赖任何其他内部 crate。
//! 这是微内核架构的 "ABI 层"。

pub mod attribution;
pub mod channel;
pub mod memory;
pub mod observer;
pub mod provider;
pub mod tool;

pub use attribution::{Attributable, Role};
pub use channel::{Channel, ChannelMessage, SendMessage};
pub use memory::{Memory, MemoryEntry, NoneMemory};
pub use observer::{Observer, ObserverEvent, NoopObserver};
pub use provider::{ModelProvider, ChatMessage, ChatRequest, ChatResponse, TokenUsage, ToolCall};
pub use tool::{Tool, ToolResult, ToolSpec};

/// 代理自主级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyLevel {
    /// 完全自主, 无需审批
    Full,
    /// 受监督, 敏感操作需审批
    Supervised,
    /// 只读, 写操作被拒绝
    ReadOnly,
}

impl Default for AutonomyLevel {
    fn default() -> Self {
        Self::Supervised
    }
}
