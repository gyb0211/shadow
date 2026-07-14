use anyhow::Context;
use shadow_config::{platform, Config};
use shadow_config::multi::alias_agent::MemoryBackendKind;
use shadow_config::observability::ObservabilityBackend;
use shadow_config::schema::AliasedAgentConfig;
use std::path::PathBuf;
use std::sync::Arc;
use shadow_core::Memory;
use shadow_core::runtime::RuntimePlatformAdapter;
use shadow_memory::create_memory_for_agent;
use crate::security::SecurityPolicy;

#[derive(Default)]
pub struct AgentRuntimeOverrides {
    pub security: Option<Arc<SecurityPolicy>>,
    pub memory: Option<Arc<dyn  Memory>>,
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

    let base = async move {
        let agent_alias = __alias.as_str();

        let eff_max_tool_iterations = agent.resolved.max_tool_iterations;
        let channel_name = if interactive { "cli" } else { "daemon" };

        // let __flush_guard = interactive.then(|| )
        if interactive
            && matches!(
                config.observability.backend,
                ObservabilityBackend::Prometheus
            )
        {
            // todo record
        }

        let runtime: Arc<dyn RuntimePlatformAdapter> = Arc::from(platform::create_runtime(&config.runtime)?);
        // todo check is subagent
        let is_subagent_caller = false;

        let agent_provider_resolved = config
            .resolved_model_provider_for_agent(agent_alias)
            .map(|(ty, alias, cfg)| (ty, alias.to_string(), cfg.clone()));

        let agent_model_provider = agent_provider_resolved.as_ref().map(|(_, _, cfg)| cfg);
        
        let mem = match overrides.memory {
            Some(m) => m,
            None => {
                create_memory_for_agent(
                    &config, agent_alias,
                    agent_model_provider.and_then(|e| e.api_key.as_deref())
                ).await?
            }
  
        };

        return Ok("exit".to_string());
    };
    base.await
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
