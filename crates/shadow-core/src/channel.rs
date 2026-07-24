//! 渠道 trait -- 消息平台集成

use crate::kennel::attribution::{Attributable, Role};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 入站消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub id: String,
    pub sender: String,
    pub content: String,
    pub reply_target: String,
}

/// 出站消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessage {
    pub content: String,
    pub recipient: String,
}

impl SendMessage {
    pub fn new(content: impl Into<String>, recipient: impl Into<String>) -> Self{
        Self{ content: content.into(), recipient: recipient.into() }
        
    }
}



/// 渠道 trait
///
/// 每个消息平台实现此 trait (CLI/Telegram/Discord...)
#[async_trait]
pub trait Channel: Attributable {
    /// 渠道名称
    fn name(&self) -> &str;

    /// 发送消息
    async fn send(&self, message: &SendMessage) -> Result<()>;

    /// 是否支持审批请求
    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()>;
}

