//! shadow-proxy -- 独立的代理中转二进制
//!
//! 三种模式:
//!   shadow-proxy              → HTTP server (默认, 远程 agent 调用)
//!   shadow-proxy --stdio      → stdio JSON-RPC (CLI/IDE 子进程)
//!   shadow-proxy --embedded   → 内嵌测试模式 (立即返回, 调试用)
//!
//! 用法:
//!   shadow-proxy -p 9090
//!   shadow-proxy --stdio
//!   shadow-proxy --bind 0.0.0.0 -p 8080

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "shadow-proxy")]
#[command(version)]
#[command(about = "影子代理中转 -- A2A + ACP broker + registry")]
struct Cli {
    /// 绑定地址 (HTTP 模式)
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,

    /// 端口 (HTTP 模式)
    #[arg(short, long, default_value_t = 9091)]
    port: u16,

    /// 使用 stdio 模式 (JSON-RPC over stdin/stdout)
    #[arg(long, default_value_t = false)]
    stdio: bool,

    /// 初始化时注册的本地 agent 名称 (逗号分隔)
    #[arg(long, value_delimiter = ',')]
    agents: Option<Vec<String>>,

    /// 详细日志
    #[arg(short, long)]
    verbose: bool,

    /// 启动时自动发现本地 agent (PATH 探测 + 端口探测)
    #[arg(long, default_value_t = true)]
    discover: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // 初始化日志
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(format!("shadow_proxy={log_level},shadow_log={log_level}"))
        .init();

    let registry = Arc::new(shadow_proxy::AgentRegistry::new());
    let router = Arc::new(shadow_proxy::TaskRouter::new(Arc::clone(&registry)));

    // 注册自身 + 配置的本地 agent
    registry.register(shadow_proxy::AgentCard::local("proxy", vec!["broker".into()]))?;
    if let Some(agents) = &cli.agents {
        for name in agents {
            registry.register(shadow_proxy::AgentCard::local(name, vec!["chat".into()]))?;
            println!("  已注册本地 agent: {name}");
        }
    }

    // 自动发现
    if cli.discover {
        println!("正在发现本地 agent...");
        let discovered = shadow_proxy::discover_agents().await;
        for d in &discovered {
            match d.source {
                shadow_proxy::DiscoverySource::Path => {
                    println!("  [PATH] 发现 {} ({:?})", d.card.name, d.card.transport);
                }
                shadow_proxy::DiscoverySource::Port(port) => {
                    println!("  [端口 {}] 发现 {} ({:?})", port, d.card.name, d.card.transport);
                }
                shadow_proxy::DiscoverySource::Config => {
                    println!("  [配置] 发现 {}", d.card.name);
                }
            }
            registry.register(d.card.clone())?;
        }
        println!("  共发现 {} 个 agent", discovered.len());
    }

    let core = shadow_proxy::ProxyCore::new(router);

    if cli.stdio {
        // stdio 模式: JSON-RPC over stdin/stdout
        let transport = shadow_proxy::StdioTransport::new(core);
        transport.serve().await?;
    } else {
        // HTTP 模式: axum RESTful API
        let server = shadow_proxy::HttpTransport::new(core, &cli.bind, cli.port);
        println!("Shadow Proxy 启动中 (HTTP)...");
        println!("  监听: http://{}:{}", cli.bind, cli.port);
        println!("  Agent 注册: POST http://{}:{}/agents/register", cli.bind, cli.port);
        println!("  任务派发: POST http://{}:{}/tasks", cli.bind, cli.port);
        println!("  Agent 列表: GET  http://{}:{}/agents", cli.bind, cli.port);
        println!("  健康检查: GET  http://{}:{}/health", cli.bind, cli.port);
        println!("  发现卡片: GET  http://{}:{}/.well-known/agent-card.json", cli.bind, cli.port);
        println!("按 Ctrl+C 退出");
        server.serve().await?;
    }

    Ok(())
}
