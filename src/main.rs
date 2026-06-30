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

/// 影子 -- trait 驱动的 AI agent 运行时
#[derive(Parser)]
#[command(name = "shadow")]
#[command(version)]
#[command(about = format!("影子 -- trait 驱动的 AI agent 运行时{}", mode_label()))]
struct Cli {
    /// 全局: 详细日志
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

/// 返回当前构建模式标签
const fn mode_label() -> &'static str {
    #[cfg(feature = "runtime")]
    {
        " [完整版]"
    }
    #[cfg(not(feature = "runtime"))]
    {
        " [kernel-only]"
    }
}

#[derive(Subcommand)]
enum Commands {
    /// 启动对话 (交互式或单次)
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
            chat_command(config, message).await?;
        }

        Commands::Config { action } => {
            config_command(config, action);
        }

        Commands::Memory { action } => {
            memory_command(config, action).await?;
        }
    }

    Ok(())
}

// ── Chat 命令 ──

async fn chat_command(
    config: shadow_config::Config,
    message: Option<String>,
) -> Result<()> {
    // 解析 provider 引用 (如 "openai.default" 或 "custom.minimax1")
    let resolved = shadow_config::resolve_provider(
        &config.providers.families,
        &config.agent.model_provider,
    )?;

    let model = resolved.effective_model(&config.agent.model).to_string();
    let temperature = resolved.effective_temperature();

    // 创建 provider (kernel 层, 两种模式都可用)
    let provider = shadow_providers::create_provider(
        &resolved.family,
        resolved.entry.api_key.as_deref(),
        resolved.effective_base_url(),
    )?;

    // 创建 memory (kernel 层)
    let workspace = shadow_config::config_dir();
    let memory = shadow_memory::create_memory(&config.memory.backend, &workspace)?;

    #[cfg(feature = "runtime")]
    {
        // 完整版: 通过 Agent (带历史/observer/工具)
        chat_via_agent(provider, memory, &config, &resolved, model, temperature, message).await?;
    }

    #[cfg(not(feature = "runtime"))]
    {
        // kernel-only: 直连 provider, 无 agent loop
        chat_direct(provider, memory, model, temperature, message).await?;
    }

    Ok(())
}

/// kernel-only 模式: 直连 provider, 最简对话
#[cfg(not(feature = "runtime"))]
async fn chat_direct(
    provider: std::sync::Arc<dyn agent_core::ModelProvider>,
    _memory: std::sync::Arc<dyn agent_core::Memory>,
    model: String,
    temperature: f64,
    message: Option<String>,
) -> Result<()> {
    use agent_core::{ChatMessage, ChatRequest, ModelProvider};

    let system = ChatMessage {
        role: "system".to_string(),
        content: "你是一个有用的 AI 助手.".to_string(),
        tool_call_id: None,
    };

    if let Some(msg) = message {
        // 单次对话
        let user = ChatMessage {
            role: "user".to_string(),
            content: msg,
            tool_call_id: None,
        };
        let request = ChatRequest {
            messages: vec![system, user],
            model,
            temperature: Some(temperature),
            max_tokens: None,
            tools: vec![],
        };
        let response = provider.chat(request).await?;
        println!("{}", response.content);
    } else {
        // 交互式对话
        println!("影子 v{} [kernel-only] -- 输入 /quit 退出", env!("CARGO_PKG_VERSION"));
        println!("---");

        let mut history = vec![system];
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
                history.truncate(1); // 保留 system
                println!("[历史已清空]");
                continue;
            }

            history.push(ChatMessage {
                role: "user".to_string(),
                content: trimmed.to_string(),
                tool_call_id: None,
            });

            let request = ChatRequest {
                messages: history.clone(),
                model: model.clone(),
                temperature: Some(temperature),
                max_tokens: None,
                tools: vec![],
            };

            match provider.chat(request).await {
                Ok(response) => {
                    history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: response.content.clone(),
                        tool_call_id: None,
                    });
                    println!("\n{}\n", response.content);
                }
                Err(e) => {
                    eprintln!("[错误] {e}");
                    // 移除刚添加的 user 消息, 避免历史污染
                    history.pop();
                }
            }
        }
    }

    Ok(())
}

/// 完整版: 通过 Agent (带历史/observer/工具)
#[cfg(feature = "runtime")]
async fn chat_via_agent(
    provider: std::sync::Arc<dyn agent_core::ModelProvider>,
    memory: std::sync::Arc<dyn agent_core::Memory>,
    config: &shadow_config::Config,
    resolved: &shadow_config::ResolvedProvider,
    model: String,
    temperature: f64,
    message: Option<String>,
) -> Result<()> {
    let agent_config = shadow_runtime::agent::AgentConfig {
        alias: config.agent.alias.clone(),
        model_provider_type: resolved.family.clone(),
        model,
        temperature: Some(temperature),
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
        println!("影子 v{} [完整版] -- 输入 /quit 退出", env!("CARGO_PKG_VERSION"));
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

    Ok(())
}

// ── Config 命令 ──

fn config_command(config: shadow_config::Config, action: ConfigAction) {
    match action {
        ConfigAction::List => {
            println!("[agent]");
            println!("  alias = \"{}\"", config.agent.alias);
            println!("  model_provider = \"{}\"", config.agent.model_provider);
            println!("  model = \"{}\"", config.agent.model);
            println!("  temperature = {:?}", config.agent.temperature);
            println!("  autonomy = \"{}\"", config.agent.autonomy);
            println!();
            println!("[memory]");
            println!("  backend = \"{}\"", config.memory.backend);
            println!();

            let providers = config.providers.list();
            if providers.is_empty() {
                println!("[providers] (无)");
            } else {
                for (family, alias) in &providers {
                    let entry = config.providers.find(family, alias).unwrap();
                    println!("[providers.{family}.{alias}]");
                    if let Some(ref key) = entry.api_key {
                        let masked = if key.len() > 8 {
                            format!("{}...{}", &key[..4], &key[key.len()-4..])
                        } else {
                            "***".to_string()
                        };
                        println!("  api_key = \"{masked}\"");
                    }
                    if let Some(ref model) = entry.model {
                        println!("  model = \"{model}\"");
                    }
                    if let Some(ref url) = entry.base_url {
                        println!("  base_url = \"{url}\"");
                    }
                    if let Some(temp) = entry.temperature {
                        println!("  temperature = {temp}");
                    }
                    println!();
                }
            }
        }
        ConfigAction::Set { key, value } => {
            println!("设置 {key} = {value}");
            println!("(配置写入功能开发中, 手动编辑: {})", shadow_config::config_path().display());
        }
        ConfigAction::Path => {
            println!("{}", shadow_config::config_path().display());
        }
    }
}

// ── Memory 命令 ──

async fn memory_command(config: shadow_config::Config, action: MemoryAction) -> Result<()> {
    let workspace = shadow_config::config_dir();
    let memory = shadow_memory::create_memory(&config.memory.backend, &workspace)?;

    match action {
        MemoryAction::List => {
            let entries = memory.list().await?;
            if entries.is_empty() {
                println!("(无记忆)");
            } else {
                for entry in entries {
                    let preview = if entry.content.len() > 60 {
                        format!("{}...", &entry.content[..60])
                    } else {
                        entry.content.clone()
                    };
                    println!("  {} : {preview}", entry.key);
                }
            }
        }
        MemoryAction::Get { key } => {
            match memory.get(&key).await? {
                Some(entry) => println!("{}: {}", entry.key, entry.content),
                None => println!("(未找到: {key})"),
            }
        }
        MemoryAction::Forget { key } => {
            memory.forget(&key).await?;
            println!("(已删除: {key})");
        }
        MemoryAction::Clear => {
            let entries = memory.list().await?;
            for entry in &entries {
                memory.forget(&entry.key).await?;
            }
            println!("(已清空 {} 条记忆)", entries.len());
        }
    }

    Ok(())
}
