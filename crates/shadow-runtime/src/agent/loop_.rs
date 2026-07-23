use crate::agent::AgentAttribution;
use crate::tools::outcome::ModelSwitchCallback;
use crate::{observability, tools};
use anyhow::Context;
use shadow_config::multi::alias_agent::MemoryBackendKind;
use shadow_config::observability::ObservabilityBackend;
use shadow_config::policy::SecurityPolicy;
use shadow_config::schema::AliasedAgentConfig;
use shadow_config::{Config, ModelProviderConfig, platform, RiskProfileConfig};
use shadow_core::runtime::RuntimePlatformAdapter;
use shadow_core::{Channel, ChatMessage, Memory, ModelProvider, Observer, SendMessage, Tool};
use shadow_log::{Action, Event, attribution_span};
use shadow_memory::create_memory_for_agent;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;
use tracing::{Instrument, info_span};
use crate::agent::system_prompt::build_system_prompt_with_mode_and_autonomy;
use crate::agent::turn::turn::{run_tool_call_loop, ToolLoop};
use crate::tools::scoped::ScopedAssembly;
//Global Model Switch request state

static MODEL_SWITCH_REQUEST: LazyLock<Arc<Mutex<Option<(String, String)>>>> =
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

fn build_system_prompt_for_turn(
    agent_workspace: &std::path::Path,
    model_name: &str,
    tool_descs: &[(&str, &str)],
    risk_profile: &RiskProfileConfig,
    model_provider: &dyn ModelProvider,
    tools_registry: &[Box<dyn Tool>],
    show_tool_calls: bool,
    thinking_prefix: Option<&str>,
) -> anyhow::Result<String> {

    let native_tools = model_provider.supports_native_tools();
    // todo

    let sys_prompt = build_system_prompt_with_mode_and_autonomy(
        agent_workspace, model_name, tool_descs,Some(risk_profile), show_tool_calls
    );

    Ok(sys_prompt)
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
        let channel_name = if interactive { "cli" } else { "daemon" };
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
                shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                    .with_outcome(shadow_log::EventOutcome::Unknown),
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
            Arc::new(config.clone()),
            &security,
            &risk_profile,
            agent_alias,
            runtime.clone(),
            mem.clone(),
            &config.data_dir,
            &config.agents,
            None,
            &config,
            is_subagent_caller,
            None,
        );

        // todo skill

        let tools::scoped::ScopedAssembled {
            registry
        } = tools::scoped::ScopedAssembled::assemble(ScopedAssembly{
            config: &config,
            agent_alias,
            security: &security,
            built: all_tools_result,
            runtime,
            caller_allowed: None,
            exclude_memory: false,
        }).await;

        let tool_registry = registry.into_inner();

        // todo route scoped tool

        let mut provider_name = agent_provider_composite(&config, agent_alias)?;
        let mut model_name = match agent_model_provider.and_then(|e| e.model.as_deref()) {
            None => anyhow::bail!(
                "No model configured for agent {agent_alias}:\
                [providers.models.{provider_name}.<alias>].model is unset and --model was not passed"
            ),
            Some(m) => m.to_string(),
        };

        //todo span

        let provider_runtime_options = match agent_provider_resolved.as_ref() {
            None => shadow_providers::provider_runtime_options_for_agent(&config, agent_alias),
            Some((ty, alias, _)) => {
                shadow_providers::provider_runtime_options_for_alias(&config, ty, alias)
            }
        };

        let (api_key, api_uri) =
            api_key_and_uri_for_provider(&config, &provider_name, agent_model_provider);

        let mut model_provider: Box<dyn ModelProvider> =
            shadow_providers::create_routed_model_provider_with_options(
                &config,
                &provider_name,
                api_key.as_deref(),
                api_uri.as_deref(),
                &config.reliability,
                &config.model_routes,
                &model_name,
                &provider_runtime_options,
            )?;

        let model_switch_callback = get_model_switch_state();
        // todo obs

        // todo 硬件RAG索引

        // todo 开发板
        // todo 国际化

        let mut tool_descs: Vec<(&str, &str)> = vec![];

        // todo skill注入方式
        // if matches!(config.skills.prompt_injection_mode, shadow_config::SkillsPromptInjectionMode::Compact){
        //
        // }
        // 注册替代硬编码
        retain_registered_tool_descriptions(&mut tool_descs, &tool_registry);

        // todo 初始化提示词长度

        // todo 提示词中排出mcp tool

        let agent_workspace = config.agent_workspace_dir(agent_alias);
        let mut system_prompt = build_system_prompt_for_turn(
            &agent_workspace,
            &model_name,
            &tool_descs,
            &risk_profile,
            model_provider.as_ref(),
            &tool_registry,
            true,
            None,
        )?;

        // 审批管理
        let approval_manager = if interactive {
            // Some(ApprovalManager::from_risk_profile(&risk_profile))
            None
        } else {
            None
        };

        // todo cost追踪

        let start = Instant::now();
        let mut final_output = String::new();
        let base_system_prompt = system_prompt.clone();


        if let Some(msg) = message {
            // todo 从message中解析 思考等级

            //todo 思考等级需要调整温度
            let eff_temperature = temperature;

            // todo 需要排出mcp tool,

            system_prompt = build_system_prompt_for_turn(
                &agent_workspace,
                &model_name,
                &tool_descs,
                &risk_profile,
                model_provider.as_ref(),
                &tool_registry,
                true,
                None,
            )?;
            // todo 过滤掉运行时的mcp能力
            let runtime_capability_names = tool_registry.iter().map(|t| t.name()).collect::<Vec<_>>();

            // todo skill 安装建议

            // todo 自动保存记忆

            // todo 记忆上下文构建

            // todo rag limit


            let mut history = vec![
                ChatMessage::system(&system_prompt),
                ChatMessage::user(&msg)
            ];

            // todo mcp tool excluded
            let response = String::new();
            loop {
                if let Some(sys_msg) = history.first_mut() && sys_msg.role == "system" {
                    // todo 注入每轮的提示词
                }

                // todo 思考覆盖 循环成本追踪


                match run_tool_call_loop(
                    ToolLoop {
                        history: &mut history,
                        channel_name,
                        channel_reply_target: None,
                        // cancellation_token: None,
                        // on_delta: None,
                        agent_alias: Some(agent_alias),
                        turn_id: &turn_id,
                    },
                ).await {
                    Ok(resp) => {
                        response = resp;
                        break;
                    }
                    Err(e) => {
                        if let Some((new_model_provider, new_model)) = is_model_switch_requested(&e) {
                            shadow_log::record!(
                              INFO,
                              shadow_log::Event::new(module_path!(), shadow_log::Action::Migrate)
                              .with_category(shadow_log::EventCategory::Provider),
                              &format!(
                                  "Model switch requested, switching from {}.{} to  {}.{}",
                                  provider_name, model_name, new_model_provider,new_model
                              )
                          );
                            let (switch_api_key, switch_uri) = api_key_and_uri_for_provider(&config, &new_model_provider, &new_model);
                            model_provider = shadow_providers::create_routed_model_provider_with_options(
                                &config,
                                &provider_name,
                                api_key,
                                api_uri,
                                &config.reliaiblity,
                                &config.model_routes,
                                &model_name,
                                &shadow_providers::options_for_provider_ref(
                                    &config, &new_model_provider, &shadow_providers::provider_runtime_options_for_agent(
                                        &config, agent_alias,
                                    ),
                                ),
                            )?;

                            provider_name = new_model_provider;
                            model = new_model;

                            clear_model_switch_request();

                            // todo obs
                            continue;
                        }


                        return Err(e);
                    }
                }
            }

            // todo 多步工具调用后 尝试主动创建skill

            final_output = response;
            println!("{final_output}");
            // todo obs

            // todo 尝试改进skill

        } else {
            println!("Shadow Interactive Mode");
            println!("Type /help for commands.\n");
            let cli = CLI_CHANNEL_FN.get().expect(
                "CLI channel factory not registered - call register_cli_channel_fn at startup"
            )();

            // todo 跨轮次历史记录

            vec![ChatMessage::system(&system_prompt)];

            loop {
                print!("> ");
                let _ = std::io::stdout().flush();
                let input = {
                    let stdin = std::io::stdin().lock();
                    match read_capped_line(stdin, MAX_INTERACTIVE_INPUT_BYTES) {
                        Ok(CappedLine::Eof) => break,
                        Ok(CappedLine::Line(s)) => s,
                        Ok(CappedLine::Truncated) => {
                            eprintln!(
                                "\nWarning: input line exceeds {} bytes and was discarded.",
                                MAX_INTERACTIVE_INPUT_BYTES
                            );
                            continue;
                        }
                        Err(e) => {
                            eprintln!("\nError reading input: {e}\n");
                            break;
                        }
                    }
                };

                let user_input = input.trim().to_string();
                if user_input.is_empty() {
                    continue;
                }

                match user_input.as_str() {
                    "/quit" | "/exit" => break,
                    _=>{}
                }

                let response = loop {
                    if let Some(sys_msg) = history.first_mut() && sys_msg.role == "system" {
                        // todo 注入每轮的提示词
                    }

                    // todo 思考覆盖 循环成本追踪


                    match run_tool_call_loop(
                        ToolLoop {
                            history: &mut history,
                            channel_name,
                            channel_reply_target: None,
                            cancellation_token: None,
                            on_delta: None,
                            agent_alias: Some(agent_alias),
                            turn_id: &turn_id,
                        },
                    ).await {
                        Ok(resp) => break resp,
                        Err(e) => {
                            if is_tool_loop_cancelled(&e) {
                                eprintln!("\n\x1b[2m(cancelled)\x1b[0m");
                                break String::new();
                            }
                            if let Some((new_model_provider, new_model)) = is_model_switch_requested(&e) {
                                shadow_log::record!(
                              INFO,
                              shadow_log::Event::new(module_path!(), shadow_log::Action::Migrate)
                              .with_category(shadow_log::EventCategory::Provider),
                              &format!(
                                  "Model switch requested, switching from {}.{} to  {}.{}",
                                  provider_name, model_name, new_model_provider,new_model
                              )
                          );
                                let (switch_api_key, switch_uri) = api_key_and_uri_for_provider(&config, &new_model_provider, &new_model);
                                model_provider = shadow_providers::create_routed_model_provider_with_options(
                                    &config,
                                    &provider_name,
                                    api_key,
                                    api_uri,
                                    &config.reliaiblity,
                                    &config.model_routes,
                                    &model_name,
                                    &shadow_providers::options_for_provider_ref(
                                        &config, &new_model_provider, &shadow_providers::provider_runtime_options_for_agent(
                                            &config, agent_alias,
                                        ),
                                    ),
                                )?;

                                provider_name = new_model_provider;
                                model = new_model;

                                clear_model_switch_request();

                                // todo obs
                                continue;
                            }

                            // todo 超出上下文窗口 进行裁剪
                            eprintln!("\nError: {e}\n");
                            break String::new();
                        }
                    }
                };

                // todo 停止监听 Ctrl+C
                // todo 清理 tx

                final_output = response;

                // todo stream load


                if let Err(e) = Channel::send(
                    &*cli,
                    SendMessage::new(format!("\n{final_output}\n"), "user")
                ).await {
                    eprintln!("\nError sending Cli response: {e}\n")
                }

                //todo obs

                //todo 输出上下文使用

                // todo 裁剪历史消息

                if let Some(sys_msg) = history.first_mut() && sys_msg.role == "system"{
                    sys_msg.content.clone_from(&base_system_prompt);
                }

                // todo save session file


            }
        }

        let duration = start.elapsed();
        // todo token_used

        // todo obs

        return Ok(final_output);
    };
    __body
        .instrument(__scope_span)
        .instrument(__attribution_span)
        .await
}

pub(crate) const MAX_INTERACTIVE_INPUT_BYTES: usize = 1024 * 1024; // 1 MiB

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

pub static CLI_CHANNEL_FN: std::sync::OnceLock<
    Box<dyn Fn() -> Box<dyn Channel> + Send + Sync>,
> = std::sync::OnceLock::new();
fn api_key_and_uri_for_provider(
    config: &Config,
    provider_name: &str,
    fallback: Option<&ModelProviderConfig>,
) -> (Option<String>, Option<String>) {
    if let Some((family, alias)) = provider_name.split_once('.')
        && let Some(entry) = config.providers.models.find(family, alias)
    {
        return (entry.api_key.clone(), entry.uri.clone());
    }
    (
        fallback.and_then(|entry| entry.api_key.clone()),
        fallback.and_then(|entry| entry.uri.clone()),
    )
}

fn retain_registered_tool_descriptions(
    tool_descs: &mut Vec<(&str, &str)>,
    tool_registry: &[Box<dyn Tool>],
) {
    let registered: HashSet<&str> = tool_registry.iter().map(|t| t.name()).collect();
    tool_descs.retain(|(name, _)| registered.contains(name))
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

fn agent_provider_composite(config: &Config, agent_alias: &str) -> Option<String> {
    config
        .resolved_model_provider_for_agent(agent_alias)
        .map(|(ty, alias, _)| format!("{ty}.{alias}"))
}
