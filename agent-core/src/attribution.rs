//! 归因系统 -- 每个参与事件的对象实现 Attributable trait
//!
//! 借鉴 ZeroClaw 的 Attributable 设计, 但大幅精简:
//! - ZeroClaw: 14 种 Role + 6 个子枚举 (72 provider kind + 37 channel kind...)
//! - Shadow: 6 种 Role, 不带子枚举, 用字符串注册
//!
//! 设计原则: trait 对象安全, Arc/Box blanket impl, 零成本归因

use std::sync::Arc;

/// 归因 trait -- 回答 "这个操作是谁干的"
pub trait Attributable: Send + Sync {
    /// 角色分类
    fn role(&self) -> Role;
    /// 具体名称 (如 agent 别名, 工具名, 渠道名)
    fn alias(&self) -> &str;
}

/// 角色枚举 -- 精简为 6 种, 不带子类型
/// 具体类型用 alias() 字符串区分, 而非枚举硬编码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 代理 (agent)
    Agent,
    /// 渠道 (telegram/discord/cli...)
    Channel,
    /// 工具 (shell/file/memory...)
    Tool,
    /// 模型提供商 (openai/anthropic/ollama...)
    Provider,
    /// 记忆后端 (sqlite/markdown/none...)
    Memory,
    /// 系统
    System,
}

impl Role {
    /// 角色家族字符串 -- 用于日志 span 命名
    #[must_use]
    pub fn family_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Channel => "channel",
            Self::Tool => "tool",
            Self::Provider => "provider",
            Self::Memory => "memory",
            Self::System => "system",
        }
    }

    /// 日志归因字段名 -- 用于 LogEvent 的 field key
    #[must_use]
    pub fn attribution_field(self) -> &'static str {
        match self {
            Self::Agent => "agent_alias",
            Self::Channel => "channel",
            Self::Tool => "tool",
            Self::Provider => "model_provider",
            Self::Memory => "memory_namespace",
            Self::System => "system",
        }
    }

    /// 默认日志分类
    #[must_use]
    pub fn default_category(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Channel => "channel",
            Self::Tool => "tool",
            Self::Provider => "provider",
            Self::Memory => "memory",
            Self::System => "system",
        }
    }
}

// ── Blanket impl: Arc<T>, Box<T>, &T 自动实现 Attributable ──
// 这样 Box<dyn ModelProvider> 也能直接调 .role() 和 .alias()

impl<T: Attributable + ?Sized> Attributable for Arc<T> {
    fn role(&self) -> Role {
        (**self).role()
    }
    fn alias(&self) -> &str {
        (**self).alias()
    }
}

impl<T: Attributable + ?Sized> Attributable for Box<T> {
    fn role(&self) -> Role {
        (**self).role()
    }
    fn alias(&self) -> &str {
        (**self).alias()
    }
}

impl<T: Attributable + ?Sized> Attributable for &T {
    fn role(&self) -> Role {
        (**self).role()
    }
    fn alias(&self) -> &str {
        (**self).alias()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeAgent;
    impl Attributable for FakeAgent {
        fn role(&self) -> Role { Role::Agent }
        fn alias(&self) -> &str { "test-agent" }
    }

    #[test]
    fn arc_box_ref_all_coerce() {
        let agent = FakeAgent;
        let arc: Arc<FakeAgent> = Arc::new(agent);
        assert_eq!(arc.role(), Role::Agent);
        assert_eq!(arc.alias(), "test-agent");
    }
}
