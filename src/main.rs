//! shadow -- 影子 CLI 入口
//!
//! 两种构建模式:
//!   1. kernel-only (--no-default-features): config + log + provider + memory
//!      shadow chat   -- 直连 provider, 无 agent loop
//!      shadow config -- 配置管理
//!      shadow memory -- 记忆管理
//!
//!   2. 完整版 (默认, --features runtime): kernel + Agent loop
//!      shadow chat   -- 通过 Agent, 带历史/observer/工具
//!      shadow config -- 配置管理
//!      shadow memory -- 记忆管理

use anyhow::Result;
use clap::{Parser, Subcommand};
use shadow::config;
use shadow_config::Config;
use shadow_log::Action;

/// 影子 -- trait 驱动的 AI agent 运行时
#[derive(Parser)]
#[command(name = "shadow")]
#[command(version)]
#[command(about = format!("影子 -- trait 驱动的 AI agent 运行时"))]
struct Cli {
    #[arg(long, global = true)]
    config_dir: Option<String>,

    #[arg(long, global = true, value_enum)]
    log_level: Option<LogLevel>,

    /// 全局: 详细日志
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}
impl LogLevel {
    fn as_directive(self) -> &'static str{
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

// todo shadow eval cmd
#[derive(Subcommand, Debug)]
enum EvalCommands {
    Run{
        #[arg(long)]
        suite: Option<String>,

        mode: Option<String>,

        // format: commands::eval::OutputFormat,
    }
}

/// 检测 stdin 是否为 TTY
fn is_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(long_about="\
    Start the AI agent loop.

    Examples:
        shadow agent -a assistant                   # interactive session
        shadow agent -a assistant -m \"Hello\"      # single chat
    ")]
    Agent {
        #[arg(short='a', long)]
        agent: String,
        #[arg(short, long)]
        message: Option<String>,
        #[arg(short='p', long)]
        model_provider: Option<String>,
        #[arg( long)]
        model: Option<String>,
        #[arg(short, long)]
        temperature: Option<f64>,
    }
}

#[derive(Subcommand)]
enum ConfigAction {
    /// 列出配置
    List,
    /// 设置配置项
    Set { key: String, value: String },
    /// 查看配置文件路径
    Path,
}

#[derive(Subcommand)]
enum MemoryAction {
    /// 列出所有记忆
    List,
    /// 查看某条记忆
    Get { key: String },
    /// 删除记忆
    Forget { key: String },
    /// 清空所有记忆
    Clear,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    //
    // // Workspace -- 集中所有路径布局 (替代散落的 config_dir() 调用)
    // let workspace = shadow_core::Workspace::open(shadow_config::config_dir());
    // workspace.ensure_layout()?;
    // let workspace_root = workspace.root();
    //
    // // 初始化日志写入器 (JSONL 持久化)
    // shadow_log::init_from_config(workspace_root, 10_000);

    // 安装日志 subscriber (终端 + LogCaptureLayer)
    shadow_log::install_subscriber(cli.verbose);

    // 加载配置
    let mut config = Box::pin( shadow_config::Config::load_or_init()).await?;

    match cli.command {
        Commands::Agent {
            agent: agent_alias,
            message,
            model_provider,
            model,
            temperature
        } => {
            if config.agent(&agent_alias).is_none(){
                anyhow::bail!("`shadow agent --agent {agent_alias}` is not configured (no [agents.{agent_alias}] entry)")
            }
            let agent_entry = config.model_provider_for_agent(&agent_alias);
            let final_temperature = temperature.unwrap_or_else(|| agent_entry.and_then(|e| e.temperature).unwrap_or(0.7));
            if let Some(p) = model_provider {

            }else if config.model_provider_for_agent(&agent_alias).is_none(){
                anyhow::bail!(
                    "No model model_provider configured for agent {agent_alias}.\
                    Pass --model-provider <type> or run `shadow quickstart` to configured one."
                );
            }

            let (provider_name, resolved_entry) = config.resolved_model_provider_for_agent(&agent_alias)
                .map(|(ty, _alias, entry)|(ty, Some(entry))).unwrap_or(("openai", None));

            let model_provider = shadow_providers::create_model_provider(provider_name, resolved_entry.and_then(|e| e.api_key.as_deref()))?;

            let model_name = resolved_entry.and_then(|e| e.model.as_deref()).unwrap_or("default");

            match message {
                Some(msg) => {
                    let response = shadow_providers::ProviderDispatch::from_ref(&*model_provider)
                        .simple_chat(&msg, model_name, Some(final_temperature)).await?;
                    println!("{response}");
                }
                None => {

                }
            }

            return Ok(())

        }
    }

    Ok(())
}

