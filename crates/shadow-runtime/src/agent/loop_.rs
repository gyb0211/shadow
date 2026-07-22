use crate::agent::AgentAttribution;
use crate::security::SecurityPolicy;
use anyhow::Context;
use shadow_config::multi::alias_agent::MemoryBackendKind;
use shadow_config::observability::ObservabilityBackend;
use shadow_config::schema::AliasedAgentConfig;
use shadow_config::{Config, platform};
use shadow_core::{Memory, Observer};
use shadow_core::runtime::RuntimePlatformAdapter;
use shadow_log::{attribution_span, Action, Event};
use shadow_memory::create_memory_for_agent;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use tracing::{info_span, Instrument};
use shadow_config::policy::SecurityPolicy;
use crate::{observability, tools};
use crate::tools::outcome::ModelSwitchCallback;
//Global Model Switch request state

static MODEL_SWITCH_REQUEST:LazyLock<Arc<Mutex<Option<(String,String)>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(None)));
pub fn get_model_switch_state() -> ModelSwitchCallback {
    Arc::clone(&MODEL_SWITCH_REQUEST)
}

#[derive(Default)]
pub struct AgentRuntimeOverrides {
    pub security: Option<Arc<SecurityPolicy>>,
    pub memory: Option<Arc<dyn Memory>>,
    pub is_subagent: bool,
}

pub async fn run(
    config: Config,
    agent_alias: &str,
    message: Option<String>,
    temperature: Option<f64>,
    interactive: bool,
    session_state_file: Option<PathBuf>,
    allowed_tools: Option<Vec<String>>,
    overrides: AgentRuntimeOverrides,
) -> anyhow::Result<String> {
    let agent: AliasedAgentConfig = resolve_agent_for_turn(&config, agent_alias)?;

    let risk_profile = config
        .risk_profile_for_agent(agent_alias)
        .with_context(|| {
            format!(
                "agents.{agent_alias}.risk_profile does not name a configured risk_profiles entry."
            )
        })?
        .clone();

    let memory_composite = {
        match agent.memory.backend {
            MemoryBackendKind::None => "none".to_string(),
            MemoryBackendKind::Markdown => format!("markdown.{agent_alias}"),
            _ => {
                let raw: &str = config.memory_backend.trim();
                if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                    "none".to_string()
                } else {
                    let (kind, alias) = raw.split_once(".").unwrap_or((raw, "default"));
                    format!("{kind}.{alias}")
                }
            }
        }
    };

    let __alias = agent_alias.to_string();
    let __attribution_span = attribution_span!(&AgentAttribution(__alias.as_str()));
    let __scope_span = info_span!(
        target: "log_internal_scope",
        "shadow_scope",
        risk_profile = %agent.risk_profile,
        runtime_profile = %agent.runtime_profile,
        memory_namespace = %memory_composite,
    );
    


    let __body = async move {
        let agent_alias: &str = __alias.as_str();
        // let eff_max_history_messages = agent.resolved.max_history_messages;
        let base_obs = observability::create_observer(&config.observability);
        let obs: Arc<dyn Observer> = Arc::from(base_obs);
        let turn_id = uuid::Uuid::new_v4().to_string();
        let channel_name = if interactive{"cli"}else{"daemon"};
        // let __flush_guard = interactive.then(|| ) todo
        //  核心矛盾：
        //
        //     OTLP batch exporter 是异步批量上报的 — 它不会每条 span/log 一产生就立刻发走，而是攒一批、按间隔周期性 flush。这对常驻进程没问题，但 CLI 一次性执行的场景下，进程跑完就退出了，background interval 还没来得及触发，缓冲区里的遥测数据就丢了。
        //
        //     逐行拆解：
        //
        //     1. interactive.then(|| ...)
        //        interactive 是个 bool。.then() 是 Bool 的方法 — 为 true 就执行闭包拿 Some(guard)，为 false 返回 None。所以守护逻辑只在交互式/CLI 单次执行时启用。
        //
        //     2. observability::FlushGuard::new(observer.clone())
        //        创建一个 FlushGuard，持有 observer 的克隆引用。这个 guard 的关键作用在它的 Drop trait — 当 guard 离开作用域被销毁时，Drop 实现会强制 flush 一次，把缓冲区里还没上报的遥测数据推出去。
        //
        //     3. let _flush_guard = ...
        //        绑定到 _flush_guard。下划线前缀表示"我不会读这个变量"，但变量本身仍然存活在整个函数 body 的作用域内。这是 Rust 里 RAII guard 的标准写法 — 你不需要主动调用它，它的存在本身就是作用，销毁时自动触发 flush。
        //
        //     为什么用 guard 而不是手动调 flush：
        //
        //     函数体内可能有多个 return 路径，包括 ? 操作符产生的提前返回（错误传播）。如果手动在每个 return 前 flush，很容易漏。guard 借助 Rust 的 Drop 语义保证 — 无论从哪条路径离开这个作用域，Drop 都会执行，遥测一定被推走。
        //
        //     最后一句注释说明了另一类调用方 — daemon、cron、subagent 这些长期运行的场景传 interactive=false，不创建 guard，因为它们靠周期性 export 自然就能把数据送出去，不需要进程退出时的强制 flush。
        //
        //     一句话总结：用 RAII guard 保证 CLI 快速退出场景下遥测不丢，用 interactive 标志区分常驻进程和一次性执行两种生命周期模式。
        if interactive
            && matches!(
                config.observability.backend,
                ObservabilityBackend::Prometheus
            )
        {
           shadow_log::record!(
               WARN,
               shadow_log::Event::new(module_path!(), shadow_log::Action::Note).with_outcome(shadow_log::EventOutcome::Unknown),
               "Observability backend is Prometheus (pull/scrape model): a one-shot CLI process \
               exists before any scraper can pull, so its telemetry will not be collected.\
               Prometheus is intended for long-running(daemon) deployment."
           );
        }

        let runtime: Arc<dyn RuntimePlatformAdapter> =
            Arc::from(platform::create_runtime(&config.runtime)?);
        // todo check is subagent
        let is_subagent_caller = overrides.is_subagent;
        
        let security = match overrides.security {
            None => Arc::new(SecurityPolicy::for_agent(&config, agent_alias)?),
            Some(sec) => sec,
        };

        let agent_provider_resolved = config
            .resolved_model_provider_for_agent(agent_alias)
            .map(|(ty, alias, cfg)| (ty, alias.to_string(), cfg.clone()));

        let agent_model_provider = agent_provider_resolved.as_ref().map(|(_, _, cfg)| cfg);

        let mem = match overrides.memory {
            Some(m) => m,
            None => {
                create_memory_for_agent(
                    &config,
                    agent_alias,
                    agent_model_provider.and_then(|e| e.api_key.as_deref()),
                )
                .await?
            }
        };
        
        shadow_log::record!(
            INFO,
            Event::new(module_path!(), Action::Load),
            "Memory initialized"
        );
        
        // todo 串口相关
        
        // todo sop
        
        let all_tools_result = tools::all_tools_with_runtime(
            
        )
        
        
        return Ok("exit".to_string());
    };
    __body.instrument(__scope_span).instrument(__attribution_span).await
}

fn resolve_agent_for_turn(
    config: &Config,
    agent_alias: &str,
) -> anyhow::Result<AliasedAgentConfig> {
    let agent = config
        .resolved_agent_config(agent_alias)
        .with_context(|| format!("agents.{agent_alias} is not configured."))?;
    Ok(agent)
}
