use std::collections::HashMap;
use std::sync::Arc;
use shadow_config::{AliasedAgentConfig, Config, RiskProfileConfig};
use shadow_config::policy::SecurityPolicy;
use shadow_core::{Memory, Tool};
use shadow_core::runtime::RuntimePlatformAdapter;
use crate::tools::registry::ToolRegistry;

pub mod attribution;
pub mod registry;
pub mod cron;

/// 创建默认工具集 -- 返回所有内置工具, 用装饰器包装敏感工具
pub fn default_tools(memory: Option<Arc<dyn Memory>>) -> ToolRegistry {
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    default_tools_with_workspace(memory, workspace)
}

/// 创建默认工具集 (指定工作目录, 用于路径安全检查)
pub fn default_tools_with_workspace(
    memory: Option<Arc<dyn Memory>>,
    workspace: std::path::PathBuf,
) -> ToolRegistry {
    ToolRegistry::new()
}

pub struct AllToolsResult {
    pub tools: Vec<Box<dyn Tool>>,
    pub unfiltered_tool_arcs: Vec<Arc<dyn Tool>>,
}

pub fn all_tools_with_runtime(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    risk_profile: RiskProfileConfig,
    agent_alias: &str,
    runtime: Arc<dyn RuntimePlatformAdapter>,
    memory: Arc<dyn Memory>,
    workspace_dir: &std::path::Path,
    agents: &HashMap<String, AliasedAgentConfig>,
    fallback_api_key: Option<&str>,
    root_config: &Config,
    is_subagent_caller: bool,
    live_config: Option<Arc<parking_lot::RwLock<Config>>>,
) -> AllToolsResult {
    
    let has_shell_access = runtime.has_shell_access();
    let persistent_writes = runtime.has_filesystem_access();
    let runtime_kind = root_config.runtime.kind.as_wire();
    
    let sandbox_cfg = risk_profile.sandbox_config();
    
    let sandbox = create_sandbox(&sandbox_cfg, runtime_kind, Some(&security.workspace_dir));
    
    
}
