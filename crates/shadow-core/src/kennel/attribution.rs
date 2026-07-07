//! 归因系统 -- 每个参与事件的对象实现 Attributable trait
//!
//! 回答 "这个操作是谁干的":
//!   - [`Attributable::role`]    角色家族
//!   - [`Attributable::alias`]   具体名称

use std::sync::Arc;
use strum_macros::IntoStaticStr;

/// 归因 trait -- 回答 "这个操作是谁干的"
pub trait Attributable: Send + Sync {
    /// 角色分类
    fn role(&self) -> Role;
    /// 具体名称 (如 agent 别名, 工具名, provider 类型)
    fn alias(&self) -> &str;
}

impl<T: Attributable + ?Sized> Attributable for std::sync::Arc<T> {
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

/// 不可以使用 self.role() or self.alias()
/// 会触发递归循环 self=&&T -> &T -> &&T -> &T
impl<T: Attributable + ?Sized> Attributable for &T {
    fn role(&self) -> Role {
        (**self).role()
    }

    fn alias(&self) -> &str {
        (**self).alias()
    }
}

/// 角色枚举 -- 7 种, 不带子类型
/// 具体实例用 `alias()` 字符串区分, 而非枚举硬编码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 代理 / AgentRuntime
    Agent,
    /// 渠道 (telegram/discord/cli...)
    Channel(ChannelKind),
    /// 工具 (shell/file/memory...)
    Tool(ToolKind),
    /// 模型提供商 (openai/anthropic/ollama...)
    Provider(ProviderKind),
    /// 记忆后端 (sqlite/markdown/none...)
    Memory(MemoryKind),
    /// 会话存储 (sqlite/file/none...)
    Session,
    /// 系统
    System,
    Swarm,
    Cron(CronKind),
    PeerGroup,
    Skill,
    Mcp,
    Sop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum ChannelKind {
    #[strum(serialize = "acp")]
    AcpChannel,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum ToolKind {
    Shell,
    HttpRequest,
    HttpServer,
    FetchUrl,
    Search,
    Memory,
    SpawnSubAgent,
    SopList,
    SopExecute,
    SopApprove,
    SopAdvance,
    SopStatus,
    SopHistory,
    Wait,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum CronKind {
    Interval,
    At,
    Cron,
    Once,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Model(ModelProviderKind),
    Tts(TtsProviderKind),
    Transcription(TranscriptionProviderKind),
    Tunnel(TunnelProviderKind),
}

impl ProviderKind {
    #[must_use]
    pub fn type_str(self) -> &'static str {
        match self {
            Self::Model(k) => k.into(),
            Self::Tts(k) => k.into(),
            Self::Transcription(k) => k.into(),
            Self::Tunnel(k) => k.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum ModelProviderKind {
    Anthropic,
    Custom,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum TtsProviderKind {
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum TranscriptionProviderKind {
    Google,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum TunnelProviderKind {
    OpenVpn,
    Custom,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum MemoryKind {
    Sqlite,
    Json,
    InMemory,
    Markdown,
    AgentScopedMarkdown,
    AgentScoped,
    Qdrant,
    Postgres,
    Lucid,
    None,
    Plugin,
}

impl Role {
    pub fn composite_prefix(self) -> Option<&'static str> {
        match self {
            Self::Channel(_) => Some("channel"),
            Self::Provider(ProviderKind::Model(_)) => Some("model_provider"),
            Self::Provider(ProviderKind::Tts(_)) => Some("tts_provider"),
            Self::Provider(ProviderKind::Transcription(_)) => Some("transcription_provider"),
            Self::Provider(ProviderKind::Tunnel(_)) => Some("tunnel_provider"),
            _ => None,
        }
    }

    pub fn composite_type(self) -> Option<&'static str> {
        match self {
            Self::Channel(c) => Some(c.into()),
            Self::Provider(p) => Some(p.type_str()),
            _ => None,
        }
    }

    pub fn attribution_field(self) -> Option<&'static str> {
        match self {
            Self::Agent => Some("agent_alias"),
            Self::Tool(_) => Some("tool"),
            Self::Cron(_) => Some("cron_job_id"),
            Self::Memory(_) => Some("memory_namespace"),
            Self::PeerGroup => Some("peer_group"),
            Self::Skill => Some("skill_bundle"),
            Self::Mcp => Some("mcp_bundle"),
            Self::Sop => Some("sop_name"),
            Self::Session => Some("session_key"),
            _ => None,
        }
    }

    pub fn family_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Channel(_) => "channel",
            Self::Tool(_) => "tool",
            Self::Provider(ProviderKind::Model(_)) => "provider.model",
            Self::Provider(ProviderKind::Tts(_)) => "provider.tts",
            Self::Provider(ProviderKind::Transcription(_)) => "provider.transcription",
            Self::Provider(ProviderKind::Tunnel(_)) => "provider.tunnel",
            Self::Memory(_) => "memory",
            Self::Session => "session",
            Self::System => "system",
            Self::Swarm => "swarm",
            Self::Cron(_) => "cron",
            Self::PeerGroup => "peer_group",
            Self::Skill => "skill",
            Self::Mcp => "mcp",
            Self::Sop => "sop",
        }
    }

    pub fn default_category(self) -> &'static str {
        match self {
            Self::Agent | Self::Swarm => "agent",
            Self::Channel(_) => "channel",
            Self::Tool(_) => "tool",
            Self::Provider(_) => "provider",
            Self::Memory(_) => "memory",
            Self::Session => "session",
            Self::Cron(_) => "cron",
            Self::PeerGroup | Self::Skill | Self::Mcp | Self::Sop | Self::System => "system",
        }
    }
}
