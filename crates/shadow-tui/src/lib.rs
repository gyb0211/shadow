//! shadow TUI -- ratatui dashboard

pub mod app;
pub mod event;
pub mod observer;
pub mod runner;
pub mod terminal;
pub mod theme;
pub mod views;
pub mod widgets;

pub use app::AppState;
pub use event::AppEvent;
pub use observer::UiObserver;
pub use runner::run_loop;

use anyhow::Result;
use shadow_config::Config;
use std::sync::Arc;
use tokio::sync::mpsc;

/// 启动 TUI. 默认进入 Chat view.
///
/// 构建流程:
/// 1. 从 config 解析 provider + 创建 memory
/// 2. 用 UiObserver (转发事件到 mpsc) 构建 Agent
/// 3. 把 Arc<Agent> + mpsc::Sender 注入 AppState
/// 4. 进入主循环
pub async fn run_tui(config: Config) -> Result<()> {
    // 初始化主题 -- 检测终端背景色 (Dark/Light)
    crate::theme::init();

    let (tx, rx) = mpsc::channel::<AppEvent>(256);
    let observer = UiObserver::arc(tx.clone());

    let mut state = AppState::new();
    state.agent_alias = config.agent.alias.clone();
    state.model_name = config.agent.model.clone();
    match build_agent(&config, observer) {
        Ok(agent) => {
            // 加载历史会话 (从 session store 恢复)
            if let Err(e) = agent.load_history().await {
                eprintln!("加载历史会话失败: {e}");
            }
            // 从 agent 历史初始化聊天消息
            {
                let history = agent.history.lock();
                state.chat.messages = history.clone();
            }
            state.agent = Some(Arc::new(agent));
            state.tx = Some(tx);
        }
        Err(e) => {
            // agent 构建失败 (如 provider 未配置): TUI 仍可启动, Enter 会提示错误
            state.last_error = Some(format!("agent 未就绪: {e}"));
        }
    }

    run_loop(state, rx).await?;
    Ok(())
}

/// 从 config 构建 Agent (镜像 main.rs::chat_via_agent 的构建逻辑)
fn build_agent(
    config: &Config,
    observer: Arc<dyn shadow_core::Observer>,
) -> Result<shadow_runtime::agent::Agent> {
    use shadow_core::AutonomyLevel;

    let resolved = shadow_config::resolve_provider(
        &config.providers.families,
        &config.agent.model_provider,
    )?;

    let model = resolved.effective_model(&config.agent.model).to_string();
    let temperature = resolved.effective_temperature();

    let provider = shadow_providers::create_provider(
        &resolved.alias,
        &resolved.family,
        resolved.entry.api_key.as_deref(),
        resolved.effective_base_url(),
        shadow_core::ModelProviderRuntimeOptions::default(),
    )?;

    let workspace = shadow_config::config_dir();
    let memory = shadow_memory::create_memory(&config.memory.backend, &workspace)?;

    let agent_config = shadow_runtime::agent::AgentConfig {
        alias: config.agent.alias.clone(),
        model_provider_type: resolved.family.clone(),
        model,
        temperature: Some(temperature),
        autonomy: match config.agent.autonomy.as_str() {
            "full" => AutonomyLevel::Full,
            "read_only" => AutonomyLevel::ReadOnly,
            _ => AutonomyLevel::Supervised,
        },
        workspace_dir: shadow_config::config_dir(),
        max_iterations: config.agent.max_iterations,
        max_history: config.agent.max_history,
        system_prompt: config.agent.system_prompt.clone(),
    };

    let tools = shadow_runtime::tools::default_tools();

    // 创建会话存储 (JSONL 文件持久化)
    let session_store: Arc<dyn shadow_core::SessionStore> = Arc::new(
        shadow_core::JsonlSessionStore::new(shadow_config::config_dir()),
    );

    let agent = shadow_runtime::agent::Agent::builder()
        .alias(&agent_config.alias)
        .provider(provider)
        .memory(memory)
        .observer(observer)
        .tools(tools)
        .config(agent_config)
        .session_store(session_store)
        .build()?;

    Ok(agent)
}
