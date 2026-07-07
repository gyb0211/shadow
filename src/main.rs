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

fn parse_temperature(s: &str) -> std::result::Result<f64, String>{
    let t = s.parse().map(|e| format!("{e}"))?;
    config::schema::validate_temperature(t)
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
        #[arg(short, long, value_parser=parse_temperature)]
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
                    let response = shadow_providers::ProviderDispatch::from_ref(model_provider)
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

// ── Chat 命令 ──

async fn chat_command(
    workspace_root: &std::path::Path,
    config: shadow_config::Config,
    message: Option<String>,
) -> Result<()> {
    // 解析 provider 引用 (如 "openai.default" 或 "custom.minimax1")
    let resolved =
        shadow_config::resolve_provider(&config.providers.families, &config.agent.model_provider)?;

    let model = resolved.effective_model(&config.agent.model).to_string();
    let temperature = resolved.effective_temperature();

    // 创建 provider -- Reliable 包装 (重试/退避/key 轮换/限流/fallback)
    let alias = format!("{}.{}", resolved.family, resolved.alias);
    let policy = shadow_providers::RetryPolicy {
        max_retries: resolved.entry.reliable.max_retries,
        initial_backoff_ms: resolved.entry.reliable.initial_backoff_ms,
        max_backoff_ms: resolved.entry.reliable.max_backoff_ms,
        jitter_pct: resolved.entry.reliable.jitter_pct,
    };
    let provider = shadow_providers::create_reliable_provider(
        &alias,
        &resolved.family,
        resolved.entry.api_keys.clone(),
        resolved.effective_base_url(),
        resolved.entry.fallback_models.clone(),
        policy,
        resolved.entry.reliable.requests_per_minute,
    )?;

    // 创建 memory (kernel 层, 路径来自 Workspace)
    let memory = shadow_memory::create_memory(&config.memory.backend, workspace_root)?;

    #[cfg(feature = "runtime")]
    {
        // 完整版: 通过 Agent (带历史/observer/工具)
        chat_via_agent(
            workspace_root,
            provider,
            memory,
            &config,
            &resolved,
            model,
            temperature,
            message,
        )
        .await?;
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
    provider: std::sync::Arc<dyn shadow_core::ModelProvider>,
    _memory: std::sync::Arc<dyn shadow_core::Memory>,
    model: String,
    temperature: f64,
    message: Option<String>,
) -> Result<()> {
    use shadow_core::{ChatMessage, ChatRequest, ModelProvider};

    let system = ChatMessage {
        role: "system".to_string(),
        content: "你是一个有用的 AI 助手.".to_string(),
    };

    if let Some(msg) = message {
        // 单次对话
        let user = ChatMessage {
            role: "user".to_string(),
            content: msg,
        };
        let request = ChatRequest {
            messages: vec![system, user],
            model,
            temperature: Some(temperature),
            max_tokens: None,
            tools: vec![],
        };
        let response = provider.chat(request).await?;
        println!("{}", response.text.unwrap_or_default());
    } else {
        // 交互式对话
        println!(
            "影子 v{} [kernel-only] -- 输入 /quit 退出",
            env!("CARGO_PKG_VERSION")
        );
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
                        content: response.text.clone().unwrap_or_default(),
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
#[allow(clippy::too_many_arguments)]
async fn chat_via_agent(
    workspace_root: &std::path::Path,
    provider: std::sync::Arc<dyn shadow_core::ModelProvider>,
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
        context_token_budget: shadow_runtime::agent::DEFAULT_CONTEXT_TOKEN_BUDGET,
        skill_review_enabled: false,
        skill_review_nudge_threshold: 5,
    };

    // 创建观察者 (日志观察者, 捕获事件到 JSONL)
    let observer: std::sync::Arc<dyn shadow_core::Observer> = std::sync::Arc::new(LogObserver);

    // 工具执行回调 -- CLI 实时显示工具调用
    let callback: std::sync::Arc<dyn shadow_runtime::agent::ToolEventCallback> =
        std::sync::Arc::new(|event: &str, detail: &str| match event {
            "tool_start" => eprintln!("  [工具] {detail}"),
            "tool_success" => eprintln!("  [完成] {detail}"),
            "tool_error" => eprintln!("  [失败] {detail}"),
            "tool_timeout" => eprintln!("  [超时] {detail}"),
            "tool_approval_skipped" => eprintln!("  [跳过] {detail}"),
            _ => {}
        });

    // 注册默认工具集 (传入 memory, 注册记忆工具)
    let tools = shadow_runtime::tools::default_tools(Some(std::sync::Arc::clone(&memory)));

    // 创建会话存储 (JSONL 文件持久化, 路径来自 Workspace)
    let session_store: std::sync::Arc<dyn shadow_core::SessionStore> =
        std::sync::Arc::new(shadow_core::JsonlSessionStore::new(workspace_root));

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
        println!(
            "影子 v{} [完整版] -- 输入 /quit 退出",
            env!("CARGO_PKG_VERSION")
        );
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
            ObserverEvent::LlmRequest {
                model,
                message_count,
            } => {
                shadow_log::record!(
                    INFO,
                    Action::Send,
                    format!("LLM 请求: model={}, messages={}", model, message_count)
                );
            }
            ObserverEvent::LlmResponse {
                model,
                duration_ms,
                tokens,
            } => {
                shadow_log::record!(
                    INFO,
                    Action::Receive,
                    format!(
                        "LLM 响应: model={}, duration={}ms, tokens={}",
                        model, duration_ms, tokens
                    )
                );
            }
            ObserverEvent::ToolCall {
                tool,
                success,
                duration_ms,
                output_preview,
            } => {
                let outcome = if *success { "成功" } else { "失败" };
                shadow_log::record!(
                    INFO,
                    Action::Invoke,
                    format!(
                        "工具调用: {} ({}, {}ms)\n{}",
                        tool, outcome, duration_ms, output_preview
                    )
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
                            format!("{}...{}", &key[..4], &key[key.len() - 4..])
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
                    if shadow_config::save(config).is_ok() {
                        println!("✓ 已设置 {key} = {value}");
                        if changed {
                            println!(
                                "  (配置已保存到 {})",
                                shadow_config::config_path().display()
                            );
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
        MemoryAction::Get { key } => match memory.get(&key).await? {
            Some(entry) => println!("{}: {}", entry.key, entry.content),
            None => println!("(未找到: {key})"),
        },
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
        ["agent", "alias"] => {
            config.agent.alias = value.to_string();
            Ok(true)
        }
        ["agent", "model"] => {
            config.agent.model = value.to_string();
            Ok(true)
        }
        ["agent", "model_provider"] => {
            config.agent.model_provider = value.to_string();
            Ok(true)
        }
        ["agent", "temperature"] => {
            let v: f64 = value
                .parse()
                .map_err(|_| "temperature 需要数字".to_string())?;
            config.agent.temperature = Some(v);
            Ok(true)
        }
        ["agent", "autonomy"] => match value {
            "full" | "supervised" | "read_only" => {
                config.agent.autonomy = value.to_string();
                Ok(true)
            }
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
        ["agent", "system_prompt"] => {
            config.agent.system_prompt = Some(value.to_string());
            Ok(true)
        }
        ["memory", "backend"] => match value {
            "none" | "markdown" => {
                config.memory.backend = value.to_string();
                Ok(true)
            }
            _ => Err("backend 必须是: none / markdown".to_string()),
        },
        ["providers", family, alias, field] => {
            let entry = config.providers.find_or_create(family, alias);
            match *field {
                // api_key / api_keys 都接受 -- 单值替换整个列表
                "api_key" | "api_keys" => {
                    entry.api_keys = vec![value.to_string()];
                    Ok(true)
                }
                "model" => {
                    entry.model = Some(value.to_string());
                    Ok(true)
                }
                "base_url" => {
                    entry.base_url = Some(value.to_string());
                    Ok(true)
                }
                "temperature" => {
                    let v: f64 = value.parse().map_err(|_| "需要数字".to_string())?;
                    entry.temperature = Some(v);
                    Ok(true)
                }
                _ => Err(format!(
                    "未知的 provider 字段: {field} (支持: api_key/model/base_url/temperature)"
                )),
            }
        }
        _ => Err(format!("无法识别的配置路径: {key}")),
    }
}

// ── Proxy 命令 ──

#[cfg(feature = "proxy")]
async fn proxy_command(bind: String, port: u16, stdio: bool) -> Result<()> {
    use shadow_proxy::{
        AgentCard, AgentRegistry, HttpTransport, ProxyCore, StdioTransport, TaskRouter,
    };

    let registry = std::sync::Arc::new(AgentRegistry::new());
    let router = std::sync::Arc::new(TaskRouter::new(std::sync::Arc::clone(&registry)));

    // 注册自身为 "main" agent (本地)
    registry.register(AgentCard::local("main", vec!["chat".into()]))?;

    let core = ProxyCore::new(router);

    if stdio {
        // stdio 模式: JSON-RPC over stdin/stdout
        let transport = StdioTransport::new(core);
        transport.serve().await?;
    } else {
        // HTTP 模式: axum RESTful API
        let server = HttpTransport::new(core, &bind, port);
        println!("Shadow Proxy 启动中 (HTTP)...");
        println!("  监听: http://{bind}:{port}");
        println!("  Agent 注册: POST http://{bind}:{port}/agents/register");
        println!("  任务派发: POST http://{bind}:{port}/tasks");
        println!("  Agent 列表: GET  http://{bind}:{port}/agents");
        println!("  健康检查: GET  http://{bind}:{port}/health");
        println!("  发现卡片: GET  http://{bind}:{port}/.well-known/agent-card.json");
        println!("按 Ctrl+C 退出");
        server.serve().await?;
    }

    Ok(())
}
