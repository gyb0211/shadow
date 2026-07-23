use crate::tools::AllToolsResult;
use shadow_config::Config;
use shadow_config::policy::SecurityPolicy;
use shadow_core::Tool;
use shadow_core::runtime::RuntimePlatformAdapter;
use std::ops::Deref;
use std::sync::Arc;

pub struct ScopedToolRegistry(Vec<Box<dyn Tool>>);

impl Deref for ScopedToolRegistry {
    type Target = [Box<dyn Tool>];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ScopedToolRegistry {
    pub fn into_inner(self) -> Vec<Box<dyn Tool>> {
        self.0
    }
}

pub struct ScopedAssembly<'a> {
    pub config: &'a Config,
    pub agent_alias: &'a str,
    pub security: &'a Arc<SecurityPolicy>,
    pub built: AllToolsResult,
    pub runtime: Arc<dyn RuntimePlatformAdapter>,
    pub caller_allowed: Option<&'a [String]>,
    pub exclude_memory: bool,
}

pub struct ScopedAssembled {
    pub registry: ScopedToolRegistry,
}

impl ScopedAssembled {
    pub async fn assemble(spec: ScopedAssembly<'_>) -> Self {
        let ScopedAssembly {
            config,
            agent_alias,
            security,
            built,
            runtime,
            caller_allowed,
            exclude_memory,
        } = spec;

        let AllToolsResult {
            tools: mut tool_registry,
            unfiltered_tool_arcs,
        } = built;

        let before_filter = tool_registry.len();
        apply_policy_tool_filter(&mut tool_registry, Some(security.as_ref()), caller_allowed);

        // todo exclude memory

        if let Some(excluded) = security.excluded_tools.as_deref() {
            tool_registry.retain(|t| !excluded.contains(&t.name().to_string()));
            // if excluded.iter().any(|ex| ex == "tool_search") {
            //
            // }
        }

        ScopedAssembled{
            registry: ScopedToolRegistry(tool_registry),
        }
    }
}

fn apply_policy_tool_filter(
    tools: &mut Vec<Box<dyn Tool>>,
    policy: Option<&SecurityPolicy>,
    caller_allowed: Option<&[String]>,
) {
    tools.retain(|t| {
        let name = t.name();
        let policy_ok = policy.is_none_or(|p| p.is_tool_allowed(name));
        let caller_ok = caller_allowed.is_none_or(|list| list.contains(&name.to_string()));
        policy_ok && caller_ok
    })
}
