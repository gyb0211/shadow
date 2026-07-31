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
use shadow_channels::cli::CliChannel;
use shadow_config::Config;
use shadow_log::Action;
use shadow_providers::ProviderDispatch;
use shadow_runtime::agent;
use shadow_runtime::agent::{AgentRuntimeOverrides, CLI_CHANNEL_FN};
use std::io::{BufRead, StdinLock};
use std::path::PathBuf;

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

    /// 飞书/Lark 用户认证 (二维码 + 链接)
    LarkAuth {
        /// 配置中的 Lark 实例名称 (默认使用第一个)
        #[arg(long)]
        name: Option<String>,
    },

    /// 交互式配置引导
    Quickstart {
        /// agent 名称 (不指定则交互选择/创建)
        #[arg(short = 'a', long)]
        agent: Option<String>,
    },

    /// 启动 channel 监听服务（飞书/Lark）
    #[command(long_about = "\
    Start channel listeners for Lark/Feishu.

    Examples:
        shadow serve -a assistant                   # listen all configured lark channels
        shadow serve -a assistant -n moqi           # listen specific lark channel
    ")]
    Serve {
        #[arg(short = 'a', long)]
        agent: String,
        /// Lark channel 名称 (不指定则监听所有配置的)
        #[arg(short = 'n', long)]
        name: Option<String>,
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
    // 安装 rustls crypto provider (WebSocket TLS 需要)
    let _ = rustls::crypto::ring::default_provider().install_default();

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
    shadow_log::install_global_subscriber(None, "debug", cli.verbose);

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
        Commands::LarkAuth { name } => {
            let lark_config = pick_lark_config(&config, name.as_deref())?;
            let domain = if lark_config.use_feishu {
                "feishu"
            } else {
                "lark"
            };
            println!("正在验证飞书 / Lark 凭证...");
            match shadow_channels::lark::probe_bot(
                &lark_config.app_id,
                &lark_config.app_secret,
                domain,
            )
            .await
            {
                Ok(Some(bot_info)) => {
                    let bot_name = bot_info
                        .get("bot_name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("(未命名)");
                    println!("  ✓ 凭证验证成功, 机器人: {bot_name}");
                }
                _ => {
                    anyhow::bail!("凭证验证失败, 请检查 app_id / app_secret");
                }
            }
            return Ok(());
        }
        Commands::Quickstart { agent } => {
            shadow::quickstart::run(&mut config, agent.as_deref()).await?;
            return Ok(());
        }
        _ => {
            anyhow::bail!(
                "This command requires the full runtime. Rebuild with default features:\n  cargo build --release"
            )
        }
    }

    // todo delivery 投递相关
    shadow_runtime::cron::scheduler::registry_delivery_fn(Box::new(
        |config, channel, target, thread_id, output| {
            Box::pin(async move {
                shadow_channels::orchestrator::deliver_announcement(
                    &config, &channel, &target, thread_id, &output,
                )
                .await
            })
        },
    ));

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
                config
                    .model_provider_for_agent(&agent_alias)
                    .and_then(|c| c.temperature)
            });

            if config.agent(&agent_alias).is_none() {
                anyhow::bail!(
                    "`shadow agent --agent {agent_alias}` is not configured (no [agents.{agent_alias}] entry)"
                )
            }

            // todo cli-channel 需要修改
            let _ = CLI_CHANNEL_FN.set(Box::new(|| Box::new(CliChannel::new("cli"))));

            // todo 其他channel 也暂时不接入

            Box::pin(agent::run(
                config,
                &agent_alias,
                message,
                final_temperature,
                true,
                None,
                None,
                AgentRuntimeOverrides::default(),
            ))
            .await
            .map(|_| ())
        }
        Commands::LarkAuth { name } => {
            let lark_config = pick_lark_config(&config, name.as_deref())?;
            let domain = if lark_config.use_feishu {
                "feishu"
            } else {
                "lark"
            };
            println!("正在验证飞书 / Lark 凭证...");
            match shadow_channels::lark::probe_bot(
                &lark_config.app_id,
                &lark_config.app_secret,
                domain,
            )
            .await
            {
                Ok(Some(bot_info)) => {
                    let bot_name = bot_info
                        .get("bot_name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("(未命名)");
                    println!("  ✓ 凭证验证成功, 机器人: {bot_name}");
                }
                _ => {
                    anyhow::bail!("凭证验证失败, 请检查 app_id / app_secret");
                }
            }
            Ok(())
        }
        Commands::Quickstart { agent } => {
            shadow::quickstart::run(&mut config, agent.as_deref()).await?;
            Ok(())
        }
        Commands::Serve { agent, name } => {
            use shadow_channels::lark::LarkChannel;
            use shadow_core::Channel;
            use std::sync::Arc;
            use tokio::sync::mpsc;

            if config.agent(&agent).is_none() {
                anyhow::bail!(
                    "`shadow serve --agent {agent}` is not configured (no [agents.{agent}] entry)"
                )
            }

            // 收集要监听的 lark channels 配置（clone 出来避免生命周期问题）
            let lark_entries: Vec<(String, shadow_config::LarkConfig)> = match name.as_deref() {
                Some(n) => {
                    let cfg = config.channels.lark.get(n).ok_or_else(|| {
                        anyhow::Error::msg(format!(
                            "Lark config '{n}' not found in [channels.lark.{n}]"
                        ))
                    })?;
                    vec![(n.to_string(), cfg.clone())]
                }
                None => {
                    if config.channels.lark.is_empty() {
                        anyhow::bail!("No Lark config found. Configure with:\n  shadow quickstart");
                    }
                    config
                        .channels
                        .lark
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                }
            };

            println!("Starting Lark channel listeners...");

            // 为每个 channel 创建 LarkChannel 实例并启动监听
            let mut handles = vec![];

            for (alias, lark_config) in lark_entries {
                let peer_resolver: Arc<dyn Fn() -> Vec<String> + Send + Sync> =
                    Arc::new(move || {
                        // 允许所有用户（后续可以从 config 里获取 peer group）
                        vec!["*".to_string()]
                    });

                let channel =
                    LarkChannel::from_config(&lark_config, alias.clone(), peer_resolver.clone());

                let (tx, mut rx) = mpsc::channel::<shadow_core::ChannelMessage>(32);

                // 启动 listen 任务
                let channel_clone = Arc::new(channel);
                let listen_handle = {
                    let channel = channel_clone.clone();
                    let alias = alias.clone();
                    tokio::spawn(async move {
                        if let Err(e) = channel.listen(tx).await {
                            eprintln!("Lark channel '{alias}' listen error: {e}");
                        }
                    })
                };

                // 启动消息处理任务
                let config_clone = config.clone();
                let agent_clone = agent.clone();
                let handle_handle = {
                    let channel = channel_clone.clone();
                    let alias = alias.clone();
                    tokio::spawn(async move {
                        while let Some(msg) = rx.recv().await {
                            println!(
                                "[{}] {} -> {}: {}",
                                msg.channel, msg.sender, msg.reply_target, msg.content
                            );

                            // 调用 agent runtime 处理消息
                            let reply = match agent::run(
                                config_clone.clone(),
                                &agent_clone,
                                Some(msg.content.clone()),
                                None,
                                false, // 非交互模式
                                None,
                                None,
                                AgentRuntimeOverrides::default(),
                            )
                            .await
                            {
                                Ok(reply) => reply,
                                Err(e) => {
                                    eprintln!("Agent runtime error: {e}");
                                    format!("Error: {e}")
                                }
                            };

                            // 发送回复
                            let send_msg = shadow_core::SendMessage::new(reply, &msg.reply_target);
                            if let Err(e) = channel.send(&send_msg).await {
                                eprintln!("Failed to send reply: {e}");
                            }
                        }
                    })
                };

                handles.push((listen_handle, handle_handle));
                println!("  ✓ Listening on Lark channel '{alias}'");
            }

            println!("\nPress Ctrl+C to stop.\n");

            // 等待所有任务
            tokio::signal::ctrl_c().await?;
            println!("\nShutting down...");

            // 取消所有任务
            for (listen, handle) in handles {
                listen.abort();
                handle.abort();
            }

            Ok(())
        }
    }
}

/// 从配置中获取 LarkConfig (按名称或取第一个)
fn pick_lark_config<'a>(
    config: &'a Config,
    name: Option<&str>,
) -> anyhow::Result<&'a shadow_config::LarkConfig> {
    match name {
        Some(n) => config.channels.lark.get(n).ok_or_else(|| {
            anyhow::Error::msg(format!(
                "Lark config '{n}' not found in [channels.lark.{n}]"
            ))
        }),
        None => {
            if config.channels.lark.is_empty() {
                anyhow::bail!(
                    "No Lark config found. Configure with:\n  shadow config set channels.lark.default.app_id <id>\n  shadow config set channels.lark.default.app_secret <secret>"
                );
            }
            config
                .channels
                .lark
                .values()
                .next()
                .ok_or_else(|| anyhow::Error::msg("Lark config map is non-empty but returned None"))
        }
    }
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
