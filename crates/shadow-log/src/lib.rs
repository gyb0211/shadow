//! 影子统一日志面 -- record! 宏 + JSONL 持久化 + 广播 + 读取器
//!
//! 借鉴 ZeroClaw 的 record! 设计, 但大幅精简:
//! - ZeroClaw: 5,079 行, 13 文件, OTel/ECS 混合 schema, 37 种 Action
//! - Shadow: 精简版, 7 文件, 简化 schema

pub mod broadcast;
pub mod event;
pub mod layer;
pub mod observer_bridge;
pub mod writer;

pub mod r#macro;
pub mod config;
pub mod reader;
pub mod subscriber;
mod tool_io;

pub use broadcast::*;
pub use event::*;
pub use layer::*;
pub use writer::*;

pub use ::tracing::Span;
pub use ::tracing::Instrument;
pub use ::tracing::{debug_span, error_span, info_span, trace_span, warn_span};

pub mod field {
    pub use ::tracing::field::{Empty,FieldSet };
}

pub fn display_chain(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

/// 私有 re-export, 宏展开用, 外部 crate 不可直接访问 tracing
#[doc(hidden)]
pub mod __private {
    pub use ::tracing;
    pub use ::chrono;
    pub use ::serde_json;
    pub use ::uuid;
}

pub fn debug_enabled() -> bool {
    ::tracing::enabled!(
        target: "log_event",
        ::tracing::Level::DEBUG
    )
}

// #[doc(hidden)]
// #[must_use]
// pub fn __private_test_writer_lock() -> impl Drop {
//     crate::writer::WRITER_TEST_LOCK.lock()
// }
// #[doc(hidden)]
// #[must_use]
// pub fn __private_test_hook_lock() -> impl Drop {
//     crate::broadcast::HOOK_TEST_LOCK.lock()
// }


