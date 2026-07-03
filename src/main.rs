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
use shadow_log::Action;

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

/// 检测 stdin 是否为 TTY
fn is_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[derive(Subcommand)]
enum Commands {
    /// 启动对话 (交互式或单次)
    Chat {
        /// 单次消息 (不进入交互模式)
        #[arg(short, long)]
        message: Option<String>,

        /// 强制行式 (不走 TUI)
        #[arg(short = 'P', long, default_value_t = false)]
        plain: bool,
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

    // Workspace -- 集中所有路径布局 (替代散落的 config_dir() 调用)
    let workspace = shadow_core::Workspace::open(shadow_config::config_dir());
    workspace.ensure_layout()?;
    let workspace_root = workspace.root();

    // 初始化日志写入器 (JSONL 持久化)
    shadow_log::init_from_config(workspace_root, 10_000);

    // 安装日志 subscriber (终端 + LogCaptureLayer)
    shadow_log::install_subscriber(cli.verbose);

    // 加载配置
    let mut config = shadow_config::load_or_init()?;

    match cli.command {
        Commands::Chat { message, plain } => {
            if message.is_none() && !plain && is_terminal() {
                // TUI 模式 (默认)
                #[cfg(feature = "tui")]
                {
                    shadow_tui::run_tui(config).await?;
                    return Ok(());
                }
                #[cfg(not(feature = "tui"))]
                {
                    chat_command(workspace_root, config, message).await?;
                }
            } else {
                chat_command(workspace_root, config, message).await?;
            }
        }

        Commands::Config { action } => {
            config_command(&mut config, action);
        }

        Commands::Memory { action } => {
            memory_command(workspace_root, config, action).await?;
        }
    }

    Ok(())
}

// ── Chat 命令 ──

async fn chat_command(
    workspace_root: &std::path::Path,
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
        resolved.entry.first_key(),
        resolved.effective_base_url(),
    )?;

    // 创建 memory (kernel 层, 路径来自 Workspace)
    let memory = shadow_memory::create_memory(&config.memory.backend, workspace_root)?;

    #[cfg(feature = "runtime")]
    {
        // 完整版: 通过 Agent (带历史/observer/工具)
        chat_via_agent(workspace_root, provider, memory, &config, &resolved, model, temperature, message).await?;
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
    provider: std::sync::Arc<dyn shadow_core::Provider>,
    _memory: std::sync::Arc<dyn shadow_core::Memory>,
    model: String,
    temperature: f64,
    message: Option<String>,
) -> Result<()> {
    use shadow_core::{ChatMessage, ChatRequest, Provider};

    let system = ChatMessage {
        role: "system".to_string(),
        content: "你是一个有用的 AI 助手.".to_string(),
        tool_call_id: None,
        tool_calls: vec![], reasoning_content: None,
    };

    if let Some(msg) = message {
        // 单次对话
        let user = ChatMessage {
            role: "user".to_string(),
            content: msg,
            tool_call_id: None,
            tool_calls: vec![], reasoning_content: None,
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
                tool_calls: vec![], reasoning_content: None,
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
                        tool_calls: vec![], reasoning_content: None,
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
    workspace_root: &std::path::Path,
    provider: std::sync::Arc<dyn shadow_core::Provider>,
    memory: std::sync::Arc<dyn shadow_core::Memory>,
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
            "full" => shadow_core::AutonomyLevel::Full,
            "read_only" => shadow_core::AutonomyLevel::ReadOnly,
            _ => shadow_core::AutonomyLevel::Supervised,
        },
        workspace_dir: workspace_root.to_path_buf(),
        max_iterations: config.agent.max_iterations,
        max_history: config.agent.max_history,
        system_prompt: config.agent.system_prompt.clone(),
    };

    // 创建观察者 (日志观察者, 捕获事件到 JSONL)
    let observer: std::sync::Arc<dyn shadow_core::Observer> =
        std::sync::Arc::new(LogObserver);

    // 工具执行回调 -- CLI 实时显示工具调用
    let callback: std::sync::Arc<dyn shadow_runtime::agent::ToolEventCallback> =
        std::sync::Arc::new(|event: &str, detail: &str| {
            match event {
                "tool_start" => eprintln!("  [工具] {detail}"),
                "tool_success" => eprintln!("  [完成] {detail}"),
                "tool_error" => eprintln!("  [失败] {detail}"),
                "tool_timeout" => eprintln!("  [超时] {detail}"),
                "tool_approval_skipped" => eprintln!("  [跳过] {detail}"),
                _ => {}
            }
        });

    // 注册默认工具集 (传入 memory, 注册记忆工具)
    let tools = shadow_runtime::tools::default_tools(Some(std::sync::Arc::clone(&memory)));

    // 创建会话存储 (JSONL 文件持久化, 路径来自 Workspace)
    let session_store: std::sync::Arc<dyn shadow_core::SessionStore> = std::sync::Arc::new(
        shadow_core::JsonlSessionStore::new(workspace_root),
    );

    let agent = shadow_runtime::agent::Agent::builder()
        .alias(&agent_config.alias)
        .provider(provider)
        .memory(memory)
        .observer(observer)
        .tools(tools)
        .tool_event_callback(callback)
        .config(agent_config)
        .session_store(session_store)
        .build()?;

    // 加载历史会话 (从 session store 恢复)
    agent.load_history().await?;

    if let Some(msg) = message {
        // 单次对话 (流式输出) -- CLI 默认不显示 think 内容
        let on_delta: std::sync::Arc<dyn shadow_runtime::agent::StreamDeltaCallback> =
            std::sync::Arc::new(|delta: shadow_runtime::agent::StreamDelta| {
                match delta {
                    shadow_runtime::agent::StreamDelta::Content(s) => {
                        print!("{s}");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                    shadow_runtime::agent::StreamDelta::Reasoning(_) => {
                        // CLI 默认不显示思考内容
                    }
                }
            });
        let resp = agent.chat_with_stream(&msg, Some(on_delta)).await?;
        // 最终响应可能仍有 think 标签, 打印清理后的版本
        // (流式 delta 已打印大部分, 这里只补全可能的差异)
        let _ = resp;
        println!();
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
                agent.clear_history().await;
                println!("[历史已清空]");
                continue;
            }

            // 流式输出: 逐字打印 LLM 回复 (CLI 默认不显示 think 内容)
            println!(); // 前置换行
            let on_delta: std::sync::Arc<dyn shadow_runtime::agent::StreamDeltaCallback> =
                std::sync::Arc::new(|delta: shadow_runtime::agent::StreamDelta| {
                    match delta {
                        shadow_runtime::agent::StreamDelta::Content(s) => {
                            print!("{s}");
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                        shadow_runtime::agent::StreamDelta::Reasoning(_) => {
                            // CLI 默认不显示思考内容
                        }
                    }
                });
            match agent.chat_with_stream(trimmed, Some(on_delta)).await {
                Ok(_) => println!("\n"),
                Err(e) => eprintln!("\n[错误] {e}"),
            }
        }
    }

    Ok(())
}

// ── 日志观察者 -- 将 Observer 事件转发到 shadow-log ──

#[cfg(feature = "runtime")]
struct LogObserver;

#[cfg(feature = "runtime")]
impl shadow_core::Attributable for LogObserver {
    fn role(&self) -> shadow_core::Role {
        shadow_core::Role::System
    }
    fn alias(&self) -> &str {
        "log-observer"
    }
}

#[cfg(feature = "runtime")]
#[async_trait::async_trait]
impl shadow_core::Observer for LogObserver {
    fn record_event(&self, event: &shadow_core::ObserverEvent) {
        use shadow_core::ObserverEvent;
        match event {
            ObserverEvent::LlmRequest { model, message_count } => {
                shadow_log::record!(
                    INFO,
                    Action::Send,
                    format!("LLM 请求: model={}, messages={}", model, message_count)
                );
            }
            ObserverEvent::LlmResponse { model, duration_ms, tokens } => {
                shadow_log::record!(
                    INFO,
                    Action::Receive,
                    format!("LLM 响应: model={}, duration={}ms, tokens={}", model, duration_ms, tokens)
                );
            }
            ObserverEvent::ToolCall { tool, success, duration_ms, output_preview } => {
                let outcome = if *success { "成功" } else { "失败" };
                shadow_log::record!(
                    INFO,
                    Action::Invoke,
                    format!("工具调用: {} ({}, {}ms)\n{}", tool, outcome, duration_ms, output_preview)
                );
            }
            ObserverEvent::SessionStart { session_id } => {
                shadow_log::record!(INFO, Action::Start, format!("会话开始: {}", session_id));
            }
            ObserverEvent::SessionEnd { session_id } => {
                shadow_log::record!(INFO, Action::Complete, format!("会话结束: {}", session_id));
            }
            ObserverEvent::Error { message } => {
                shadow_log::record!(ERROR, Action::Fail, format!("错误: {}", message));
            }
            _ => {}
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ── Config 命令 ──

fn config_command(config: &mut shadow_config::Config, action: ConfigAction) {
    match action {
        ConfigAction::List => {
            println!("[agent]");
            println!("  alias = \"{}\"", config.agent.alias);
            println!("  model_provider = \"{}\"", config.agent.model_provider);
            println!("  model = \"{}\"", config.agent.model);
            println!("  temperature = {:?}", config.agent.temperature);
            println!("  autonomy = \"{}\"", config.agent.autonomy);
            println!("  max_iterations = {}", config.agent.max_iterations);
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
                    for (i, key) in entry.api_keys.iter().enumerate() {
                        let masked = if key.len() > 8 {
                            format!("{}...{}", &key[..4], &key[key.len()-4..])
                        } else {
                            "***".to_string()
                        };
                        if entry.api_keys.len() == 1 {
                            println!("  api_key = \"{masked}\"");
                        } else {
                            println!("  api_key[{i}] = \"{masked}\"");
                        }
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
            // 支持 dotted path 设置配置项
            // 例: agent.model, agent.max_iterations, providers.openai.default.api_key
            match config_set(&mut *config, &key, &value) {
                Ok(changed) => {
                    if shadow_config::save(&config).is_ok() {
                        println!("✓ 已设置 {key} = {value}");
                        if changed {
                            println!("  (配置已保存到 {})", shadow_config::config_path().display());
                        }
                    } else {
                        eprintln!("✗ 保存配置失败");
                    }
                }
                Err(e) => {
                    eprintln!("✗ {e}");
                    eprintln!("  支持的路径:");
                    eprintln!("    agent.alias <string>");
                    eprintln!("    agent.model <string>");
                    eprintln!("    agent.model_provider <string>");
                    eprintln!("    agent.temperature <float>");
                    eprintln!("    agent.autonomy <full|supervised|read_only>");
                    eprintln!("    agent.max_iterations <int>");
                    eprintln!("    agent.max_history <int>");
                    eprintln!("    agent.system_prompt <string>");
                    eprintln!("    memory.backend <none|markdown>");
                    eprintln!("    providers.<family>.<alias>.api_key <string>");
                    eprintln!("    providers.<family>.<alias>.model <string>");
                    eprintln!("    providers.<family>.<alias>.base_url <string>");
                }
            }
        }
        ConfigAction::Path => {
            println!("{}", shadow_config::config_path().display());
        }
    }
}

// ── Memory 命令 ──

async fn memory_command(
    workspace_root: &std::path::Path,
    config: shadow_config::Config,
    action: MemoryAction,
) -> Result<()> {
    let memory = shadow_memory::create_memory(&config.memory.backend, workspace_root)?;

    match action {
        MemoryAction::List => {
            let entries = memory.list(None).await?;
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
            let entries = memory.list(None).await?;
            for entry in &entries {
                memory.forget(&entry.key).await?;
            }
            println!("(已清空 {} 条记忆)", entries.len());
        }
    }

    Ok(())
}

// ── config set 实现 ──

/// 解析 dotted path 并设置配置项
fn config_set(config: &mut shadow_config::Config, key: &str, value: &str) -> Result<bool, String> {
    let parts: Vec<&str> = key.split('.').collect();

    match parts.as_slice() {
        ["agent", "alias"] => { config.agent.alias = value.to_string(); Ok(true) }
        ["agent", "model"] => { config.agent.model = value.to_string(); Ok(true) }
        ["agent", "model_provider"] => { config.agent.model_provider = value.to_string(); Ok(true) }
        ["agent", "temperature"] => {
            let v: f64 = value.parse().map_err(|_| "temperature 需要数字".to_string())?;
            config.agent.temperature = Some(v);
            Ok(true)
        }
        ["agent", "autonomy"] => match value {
            "full" | "supervised" | "read_only" => { config.agent.autonomy = value.to_string(); Ok(true) }
            _ => Err("autonomy 必须是: full / supervised / read_only".to_string()),
        },
        ["agent", "max_iterations"] => {
            config.agent.max_iterations = value.parse().map_err(|_| "需要正整数".to_string())?;
            Ok(true)
        }
        ["agent", "max_history"] => {
            config.agent.max_history = value.parse().map_err(|_| "需要正整数".to_string())?;
            Ok(true)
        }
        ["agent", "system_prompt"] => { config.agent.system_prompt = Some(value.to_string()); Ok(true) }
        ["memory", "backend"] => match value {
            "none" | "markdown" => { config.memory.backend = value.to_string(); Ok(true) }
            _ => Err("backend 必须是: none / markdown".to_string()),
        },
        ["providers", family, alias, field] => {
            let entry = config.providers.find_or_create(family, alias);
            match *field {
                // api_key / api_keys 都接受 -- 单值替换整个列表
                "api_key" | "api_keys" => { entry.api_keys = vec![value.to_string()]; Ok(true) }
                "model" => { entry.model = Some(value.to_string()); Ok(true) }
                "base_url" => { entry.base_url = Some(value.to_string()); Ok(true) }
                "temperature" => {
                    let v: f64 = value.parse().map_err(|_| "需要数字".to_string())?;
                    entry.temperature = Some(v);
                    Ok(true)
                }
                _ => Err(format!("未知的 provider 字段: {field} (支持: api_key/model/base_url/temperature)")),
            }
        }
        _ => Err(format!("无法识别的配置路径: {key}")),
    }
}
