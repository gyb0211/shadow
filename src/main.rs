//! shadow -- 影子 CLI 入口
//!
//! ZeroClaw 架构的精简复刻

use anyhow::Result;
use clap::{Parser, Subcommand};

/// 影子 -- ZeroClaw 架构精简复刻
#[derive(Parser)]
#[command(name = "shadow")]
#[command(version)]
#[command(about = "影子 -- trait 驱动的 AI agent 运行时")]
struct Cli {
    /// 全局: 详细日志
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 启动交互式对话
    Chat {
        /// 单次消息 (不进入交互模式)
        #[arg(short, long)]
        message: Option<String>,
    },

    /// 配置管理
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// 记忆管理
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
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

    // 安装日志
    shadow_log::install_subscriber(cli.verbose);

    // 加载配置
    let config = shadow_config::load_or_init()?;

    match cli.command {
        Commands::Chat { message } => {
            // 创建 provider
            let provider = shadow_providers::create_provider(
                &config.provider.provider_type,
                config.provider.api_key.as_deref(),
                config.provider.base_url.as_deref(),
            )?;

            // 创建 memory
            let memory = shadow_memory::create_memory(&config.memory.backend, &shadow_config::config_dir())?;

            // 创建 agent
            let agent_config = shadow_runtime::agent::AgentConfig {
                alias: config.agent.alias.clone(),
                model_provider_type: config.provider.provider_type.clone(),
                model: config.agent.model.clone(),
                temperature: config.agent.temperature,
                autonomy: match config.agent.autonomy.as_str() {
                    "full" => agent_core::AutonomyLevel::Full,
                    "read_only" => agent_core::AutonomyLevel::ReadOnly,
                    _ => agent_core::AutonomyLevel::Supervised,
                },
                workspace_dir: shadow_config::config_dir(),
            };

            let agent = shadow_runtime::agent::Agent::builder()
                .alias(&agent_config.alias)
                .provider(provider)
                .memory(memory)
                .config(agent_config)
                .build()?;

            if let Some(msg) = message {
                // 单次对话
                let response = agent.chat(&msg).await?;
                println!("{response}");
            } else {
                // 交互式对话
                println!("影子 v{} -- 输入 /quit 退出", env!("CARGO_PKG_VERSION"));
                println!("---");

                let stdin = std::io::stdin();
                let mut line = String::new();
                loop {
                    print!("> ");
                    std::io::Write::flush(&mut std::io::stdout())?;
                    line.clear();
                    if stdin.read_line(&mut line)? == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "/quit" || trimmed == "/exit" {
                        break;
                    }
                    if trimmed == "/clear" {
                        agent.clear_history();
                        println!("[历史已清空]");
                        continue;
                    }

                    match agent.chat(trimmed).await {
                        Ok(response) => println!("\n{response}\n"),
                        Err(e) => eprintln!("[错误] {e}"),
                    }
                }
            }
        }

        Commands::Config { action } => match action {
            ConfigAction::List => {
                let content = serde_json::to_string_pretty(&config)?;
                println!("{content}");
            }
            ConfigAction::Set { key, value } => {
                println!("设置 {key} = {value}");
                // TODO: 实现配置写入
                println!("(配置写入功能开发中)");
            }
            ConfigAction::Path => {
                println!("{}", shadow_config::config_path().display());
            }
        },

        Commands::Memory { action } => match action {
            MemoryAction::List => {
                let memory = shadow_memory::create_memory("markdown", &shadow_config::config_dir())?;
                let entries = memory.list().await?;
                if entries.is_empty() {
                    println!("(无记忆)");
                } else {
                    for entry in entries {
                        println!("  {} : {}", entry.key, &entry.content[..entry.content.len().min(60)]);
                    }
                }
            }
            MemoryAction::Get { key } => {
                let memory = shadow_memory::create_memory("markdown", &shadow_config::config_dir())?;
                match memory.get(&key).await? {
                    Some(entry) => println!("{}: {}", entry.key, entry.content),
                    None => println!("(未找到: {key})"),
                }
            }
            MemoryAction::Forget { key } => {
                let memory = shadow_memory::create_memory("markdown", &shadow_config::config_dir())?;
                memory.forget(&key).await?;
                println!("(已删除: {key})");
            }
            MemoryAction::Clear => {
                println!("(清空记忆功能开发中)");
            }
        },
    }

    Ok(())
}
