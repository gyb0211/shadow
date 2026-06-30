//! 影子统一日志面 -- record! 宏 + JSONL 持久化 + 广播
//!
//! 借鉴 ZeroClaw 的 record! 设计, 但大幅精简:
//! - ZeroClaw: 5,079 行, 13 文件, OTel/ECS 混合 schema, 37 种 Action
//! - Shadow: 目标 ~300 行, 单文件核心, 简化 schema

pub mod broadcast;
pub mod event;
pub mod layer;
pub mod writer;

mod r#macro;

pub use broadcast::{set_broadcast_hook, current_broadcast_hook, subscribe};
pub use event::{LogEvent, Severity, Action, EventCategory, EventOutcome};
pub use layer::LogCaptureLayer;
pub use writer::{init_from_config, record_event, runtime_trace_path};

/// 私有 re-export, 宏展开用, 外部 crate 不可直接访问 tracing
#[doc(hidden)]
pub mod __private {
    pub use ::tracing;
    pub use ::chrono;
    pub use ::serde_json;
    pub use ::uuid;
}

/// 安装全局 subscriber (终端 + LogCaptureLayer)
pub fn install_subscriber(verbose: bool) {
    use tracing_subscriber::prelude::*;

    let capture = LogCaptureLayer;
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact();

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            if verbose {
                tracing_subscriber::EnvFilter::new("debug")
            } else {
                tracing_subscriber::EnvFilter::new("warn")
            }
        });

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt)
        .with(capture)
        .init();
}
