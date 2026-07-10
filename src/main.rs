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
use shadow_providers::ProviderDispatch;
use std::io::{BufRead, StdinLock};

const STDIN_LINE_CAP: usize = 1024 * 1024;

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
    fn as_directive(self) -> &'static str {
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
    Run {
        #[arg(long)]
        suite: Option<String>,

        mode: Option<String>,
        // format: commands::eval::OutputFormat,
    },
}

/// 检测 stdin 是否为 TTY
fn is_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(long_about = "\
    Start the AI agent loop.

    Examples:
        shadow agent -a assistant                   # interactive session
        shadow agent -a assistant -m \"Hello\"      # single chat
    ")]
    Agent {
        #[arg(short = 'a', long)]
        agent: String,
        #[arg(short, long)]
        message: Option<String>,
        #[arg(short = 'p', long)]
        model_provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(short, long)]
        temperature: Option<f64>,
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
    let mut config = Box::pin(shadow_config::Config::load_or_init()).await?;
    #[cfg(not(feature = "runtime"))]
    match cli.command {
        Commands::Agent {
            agent: agent_alias,
            message,
            model_provider,
            model,
            temperature,
        } => {
            if config.agent(&agent_alias).is_none() {
                anyhow::bail!(
                    "`shadow agent --agent {agent_alias}` is not configured (no [agents.{agent_alias}] entry)"
                )
            }
            let agent_entry = config.model_provider_for_agent(&agent_alias);
            let final_temperature = temperature
                .unwrap_or_else(|| agent_entry.and_then(|e| e.temperature).unwrap_or(0.7));
            if let Some(p) = &model_provider {
                let (type_key, alias_key) = p.split_once('.').unwrap_or((p.as_str(), &agent_alias));
                let entry = config
                    .providers
                    .models
                    .ensure(type_key, alias_key)
                    .ok_or_else(|| {
                        anyhow::Error::msg(format!(
                            "Unknown model_provider family: {type_key}. \
                        Configure a provider via `shadow quickstart` or /config editor.
                        "
                        ))
                    })?;

                if let Some(m) = &model {
                    entry.model = Some(m.clone())
                }

                entry.temperature = Some(final_temperature);
                if let Some(agent_cfg) = config.agents.get_mut(&agent_alias) {
                    agent_cfg.model_provider = format!("{type_key}.{alias_key}").into();
                }
            } else if config.model_provider_for_agent(&agent_alias).is_none() {
                anyhow::bail!(
                    "No model model_provider configured for agent {agent_alias}.\n
                    Pass --model-provider <type> or run `shadow quickstart` to configured one."
                );
            }

            let (provider_name, resolved_entry) = config
                .resolved_model_provider_for_agent(&agent_alias)
                .map(|(ty, _alias, entry)| (ty, Some(entry)))
                .unwrap_or(("openai", None));

            let model_provider = shadow_providers::create_model_provider(
                provider_name,
                resolved_entry.and_then(|e| e.api_key.as_deref()),
                resolved_entry.and_then(|e| e.url.as_deref()),
            )?;

            let model_name = resolved_entry
                .and_then(|e| e.model.as_deref())
                .unwrap_or("default");

            match message {
                Some(msg) => {
                    let response = shadow_providers::ProviderDispatch::from_ref(&*model_provider)
                        .simple_chat(&msg, model_name, Some(final_temperature))
                        .await?;
                    println!("{response}");
                }
                None => {
                    /// Interactive mode
                    loop {
                        eprint!(">");
                        let line = {
                            let stdin = std::io::stdin().lock();
                            match read_capped_line(stdin, STDIN_LINE_CAP) {
                                Ok(c) => match c {
                                    CappedLine::Line(s) => s,
                                    CappedLine::Truncated => {
                                        eprintln!(
                                            "\nWarning: input line exceeds {} bytes and was discarded.",
                                            STDIN_LINE_CAP
                                        );
                                        continue;
                                    }
                                    CappedLine::Eof => break,
                                },
                                Err(e) => {
                                    eprintln!("\nError reading input: {e}\n");
                                    break;
                                }
                            }
                        };

                        let response = ProviderDispatch::from_ref(&*model_provider)
                            .simple_chat(line.trim(), model_name, Some(final_temperature))
                            .await?;
                        println!("{response}");
                    }
                }
            }

            return Ok(());
        }
        _ => {
            anyhow::bail!(
                "This command requires the full runtime. Rebuild with default features:\n  cargo build --release"
            );
        }
    }

    // todo delivery 投递相关

    #[cfg(feature = "runtime")]
    match cli.command {
        Commands::Agent {
            agent: agent_alias,
            message,
            model_provider,
            model,
            temperature,
        } => {
            let final_temperature = temperature.or_else(|| {
                config.model_provider_for_agent(&agent_alias)
                    .and_then(|c| c.temperature)
            });

            if config.agent(&agent_alias).is_none() {
                anyhow::bail!(
                    "`shadow agent --agent {agent_alias}` is not configured (no [agents.{agent_alias}] entry)"
                )
            }

            // todo cli-channel 暂时不接入
            
            // todo 其他channel 也暂时不接入
            
            Box::pin(agent::run())


        }
    }

    Ok(())
}

#[derive(Debug)]
enum CappedLine {
    Line(String),
    Truncated,
    Eof,
}

fn read_capped_line<R: std::io::BufRead>(reader: R, cap: usize) -> std::io::Result<CappedLine> {
    let mut raw = Vec::new();
    let mut limited = reader.take((cap + 1) as u64);
    std::io::BufRead::read_until(&mut limited, b'\n', &mut raw)?;
    let truncated = raw.len() > cap;

    if truncated {
        let mut inner = limited.into_inner();
        discard_until_newline(&mut inner)?;
        return Ok(CappedLine::Truncated);
    } else if raw.last() == Some(&b'\n') {
        raw.pop();
    }

    if raw.is_empty() {
        return Ok(CappedLine::Eof);
    }

    Ok(CappedLine::Line(String::from_utf8_lossy(&raw).into_owned()))
}

fn discard_until_newline<R: std::io::BufRead>(reader: &mut R) -> std::io::Result<()> {
    loop {
        let buf = reader.fill_buf()?;
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            reader.consume(pos + 1);
            return Ok(());
        }

        let len = buf.len();
        if len == 0 {
            return Ok(());
        }

        reader.consume(len)
    }
}
