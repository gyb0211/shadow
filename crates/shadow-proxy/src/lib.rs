//! Shadow Proxy -- A2A + ACP broker + registry
//!
//! 一个程序, 四重身份:
//! - 对主 agent: 一个 Tool (delegate) -- Embedded 模式
//! - 对本地 agent: ACP broker (spawn + stdio JSON-RPC)
//! - 对远程 agent: A2A broker (HTTP JSON-RPC)
//! - 对所有 agent: Registry (注册 + 发现)
//!
//! 三种 transport:
//! - Embedded: ProxyTool 直接调 TaskRouter (零开销)
//! - HTTP: axum RESTful API (远程 agent)
//! - stdio: JSON-RPC over stdin/stdout (CLI/IDE 集成)

pub mod acp_client;
pub mod a2a_client;
pub mod card;
pub mod core;
pub mod discovery;
pub mod http_transport;
pub mod local;
pub mod proxy_tool;
pub mod registry;
pub mod router;
pub mod stdio_transport;
pub mod task;
pub mod transport;

pub use acp_client::AcpClient;
pub use a2a_client::A2aClient;
pub use card::{AgentCard, AgentSkill, TransportKind};
pub use core::ProxyCore;
pub use discovery::{discover_agents, DiscoveredAgent, DiscoverySource};
pub use http_transport::HttpTransport;
pub use local::LocalAgent;
pub use proxy_tool::ProxyTool;
pub use registry::AgentRegistry;
pub use router::TaskRouter;
pub use stdio_transport::StdioTransport;
pub use task::{Task, TaskStatus};
pub use transport::AgentTransport;

/// Proxy 传输模式
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyMode {
    /// 内嵌模式 -- 作为库直接调用 (无网络)
    Embedded,
    /// HTTP server -- 远程 agent 通过 HTTP 调用
    Http,
    /// stdio server -- JSON-RPC over stdin/stdout (CLI/IDE)
    Stdio,
}
