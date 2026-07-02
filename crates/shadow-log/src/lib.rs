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
///
/// TUI 模式下 stderr 被 AlternateScreen 隐藏, 但 JSONL 文件仍写入.
/// LogCaptureLayer 使用独立过滤器 (shadow_log_event=info),
/// 不受全局 verbose/warn 级别限制, 确保 record! 事件始终持久化.
pub fn install_subscriber(verbose: bool) {
    use tracing_subscriber::prelude::*;

    // LogCaptureLayer -- 独立 per-layer filter, 始终捕获 record! / attribution
    let capture = LogCaptureLayer.with_filter(
        tracing_subscriber::filter::Targets::new()
            .with_target("shadow_log_event", tracing::Level::INFO)
            .with_target("shadow_log_attribution", tracing::Level::INFO),
    );

    // 终端层 (stderr) 的过滤器 -- 受 verbose / RUST_LOG 控制
    let base = if verbose { "debug" } else { "warn" };
    let fmt_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(base));

    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .with_filter(fmt_filter);

    // 关键: fmt_filter 只挂在 fmt 层, 不挂全局. 这样 LogCaptureLayer 的 per-layer
    // filter 才能真正独立工作 -- 无论 RUST_LOG/warn 默认值如何, record! 事件始终
    // 被捕获并写入 JSONL. (之前 fmt_filter 挂在 registry 级别, 导致 INFO 级
    // record! 被 warn 默认值先拦截, 永远到不了 capture layer -- 这就是日志写不
    // 出来的根因.)
    tracing_subscriber::registry()
        .with(fmt)
        .with(capture)
        .init();
}
