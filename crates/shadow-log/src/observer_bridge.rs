//! Observer 桥接 -- 将 LogEvent 投影到 ObserverEvent, 统一日志和观察者通道
//!
//! 参考 ZeroClaw observer_bridge.rs:
//! - LogEvent → ObserverEvent 投影 (只转发 metric 相关字段)
//! - 让 TUI/Prometheus 等后端消费同一份事件流
//! - 无 observer 绑定时 no-op

use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;

use crate::event::LogEvent;

static OBSERVER: OnceLock<RwLock<Option<Arc<dyn LogObserver>>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Arc<dyn LogObserver>>> {
    OBSERVER.get_or_init(|| RwLock::new(None))
}

/// 日志观察者 trait -- 接收投影后的事件
///
/// 与 shadow_core::Observer 不同, 这个 trait 专门接收 LogEvent,
/// 避免循环依赖 (shadow-log 不依赖 shadow-core 的 Observer trait)
pub trait LogObserver: Send + Sync {
    fn on_log_event(&self, event: &LogEvent);
}

/// 安装日志观察者
pub fn set_observer(observer: Arc<dyn LogObserver>) {
    *slot().write() = Some(observer);
}

/// 移除日志观察者
pub fn clear_observer() {
    *slot().write() = None;
}

/// 投影 LogEvent 到已绑定的观察者
/// 无 observer 绑定时 no-op
pub(crate) fn forward(event: &LogEvent) {
    let Some(observer) = slot().read().clone() else {
        return;
    };
    observer.on_log_event(event);
}
