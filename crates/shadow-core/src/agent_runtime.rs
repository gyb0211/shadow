//! AgentRuntime trait -- Agent 主循环抽象

use crate::attribution::Attributable;
use anyhow::Result;
use async_trait::async_trait;

/// Agent 运行时 trait
///
/// 把 Provider + Memory + Tool 串成可交互的对话循环。
/// 实现方负责历史管理、工具调度、observer 通知等。
/// 通过 [`Attributable`] 参与归因 (Role::Agent), alias 取 agent 别名。
#[async_trait]
pub trait AgentRuntime: Attributable {
    /// 处理一轮用户输入, 返回 assistant 回复
    async fn run(&self, input: &str) -> Result<String>;

    /// 重置会话状态 (清空历史)
    async fn reset(&self) -> Result<()> {
        Ok(())
    }
}
