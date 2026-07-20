//! 进程内广播 -- SSE / 实时订阅

use parking_lot::RwLock;
use serde_json::Value;
use std::sync::OnceLock;
use tokio::sync::broadcast;

pub type LogSender = broadcast::Sender<Value>;

static BROADCAST: OnceLock<RwLock<Option<LogSender>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<LogSender>> {
    BROADCAST.get_or_init(|| RwLock::new(None))
}

/// 安装广播发送端
pub fn set_broadcast_hook(sender: LogSender) {
    *slot().write() = Some(sender);
}
pub fn clear_broadcast_hook() {
    *slot().write() = None;
}

/// 获取当前广播发送端
#[must_use]
pub fn current_broadcast_hook() -> Option<LogSender> {
    slot().read().clone()
}

/// 订阅广播
#[must_use]
pub fn subscribe() -> Option<broadcast::Receiver<Value>> {
    slot().read().as_ref().map(|s: &LogSender| s.subscribe())
}


pub fn subscribe_or_install() -> broadcast::Receiver<Value>{
    {
        let read = slot().read();
        if let Some(sender) = read.as_ref() {
            return sender.subscribe();
        }
    }
    let (tx,rx) = broadcast::channel((65536));
    set_broadcast_hook(tx);
    rx

}