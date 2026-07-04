//! Agent 传输层 trait -- 所有 agent 通信的统一抽象

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::card::AgentCard;

/// Agent 传输层 -- 统一的 agent 通信接口
///
/// 三种实现:
/// - LocalAgent: 进程内函数调用
/// - AcpClient: ACP 子进程 (stdio JSON-RPC)
/// - A2aClient: A2A 远程 (HTTP JSON-RPC)
#[async_trait]
pub trait AgentTransport: Send + Sync {
    /// 发送任务, 等待结果 (同步)
    async fn chat(&self, prompt: &str) -> Result<String>;

    /// 发送任务, 流式返回
    async fn chat_stream(&self, prompt: &str) -> BoxStream<'_, Result<String>>;

    /// agent 能力声明
    fn card(&self) -> &AgentCard;

    /// transport 类型 (便捷方法)
    fn transport_kind(&self) -> &str {
        match self.card().transport {
            crate::card::TransportKind::Local => "local",
            crate::card::TransportKind::Acp => "acp",
            crate::card::TransportKind::A2a => "a2a",
        }
    }
}
