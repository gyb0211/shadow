use crate::security::create_sandbox;
use crate::tools::cron::add::CronAddTool;
use crate::tools::registry::ToolRegistry;
use async_trait::async_trait;
use serde_json::Value;
use shadow_config::policy::SecurityPolicy;
use shadow_config::{AliasedAgentConfig, Config, RiskProfileConfig};
use shadow_core::runtime::RuntimePlatformAdapter;
use shadow_core::{Attributable, Memory, Role, Tool, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;
use shadow_config::platform::native::NativeRuntime;
use crate::tools::model_switch::ModelSwitchTool;

pub mod attribution;
pub mod cron;
pub mod registry;
mod model_switch;
pub mod outcome;
pub(crate) mod scoped;

pub struct AllToolsResult {
    pub tools: Vec<Box<dyn Tool>>,
    pub unfiltered_tool_arcs: Vec<Arc<dyn Tool>>,
}

pub fn all_tools_with_runtime(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    risk_profile: &RiskProfileConfig,
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

    let mut tools_arcs: Vec<Arc<dyn Tool>> = vec![Arc::new(CronAddTool::new(
        config.clone(),
        security.clone(),
        agent_alias,
    ))];

    if is_subagent_caller {
        // subagent 不可以切换模型
        tools_arcs.retain(|t| t.name() != ModelSwitchTool::NAME);
    }

    if let Some((family, alias, entry)) = root_config.resolved_model_provider_for_agent(agent_alias)
    {
        let llm_task_provider = family.to_string();
        let llm_task_model = entry.model.clone().unwrap_or_else(|| "none".to_string());
        let llm_task_runtime_opts =
            shadow_providers::provider_runtime_options_for_alias(root_config, family, alias);
        tools_arcs.push(Arc::new(LlmTaskTool::new(
            security.clone(),
            llm_task_provider,
            llm_task_model,
            entry.temperature,
            entry.api_key.clone(),
            llm_task_runtime_opts,
        )));
    }

    // todo skill 注入

    //todo 浏览器配置

    // todo 浏览器代理

    // todo http请求

    // todo web fetch

    // todo headless 浏览器

    // todo web search

    // todo nation

    // todo jira

    // todo 项目管理工具

    //todo 网络安全

    // todo 备份
    // todo 数据
    // todo cloud

    // todo google workspace

    // todo claude acp
    // todo codex acp
    // todo gemini acp

    //todo vision 截图 识别图片

    // todo  session tool
    // todo linkedin

    // todo 图片生产

    // todo 文件上传
    // todo 打包上传
    // todo 文件下载
    // todo poll tool

    // todo sop tool
    // todo Composio 组合工具

    // todo emoji

    // todo channel room

    // todo ask user

    // todo router tool

    // todo Microsoft365

    // todo knowledge
    // todo q全局代理认证

    let delegate_global_credential = fallback_api_key.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    });

    let provider_runtime_options =
        shadow_providers::provider_runtime_options_for_agent(root_config, agent_alias);

    let delegate_handle: Option<&str> = if agents.is_empty() {
        None
    } else {
        //  todo 代理tool
        None
    };

    // todo verifiable tool

    // todo pipeline tool

    AllToolsResult {
        unfiltered_tool_arcs: tools_arcs.clone(),
        tools: boxed_registry_from_arcs(tools_arcs),
        // delegate_handle,
        // ask_user_handle,
        // channel_room_handle,
        // reaction_handle,
        // poll_handle: Some(poll_handle),
        // escalate_handle,
    }
}

#[derive(Clone)]
struct ArcDelegationTool {
    inner: Arc<dyn Tool>,
}

impl ArcDelegationTool {
    fn boxed(inner: Arc<dyn Tool>) -> Box<dyn Tool> {
        Box::new(Self { inner })
    }
}

impl Attributable for ArcDelegationTool {
    fn role(&self) -> Role {
        self.inner.role()
    }

    fn alias(&self) -> &str {
        self.inner.alias()
    }
}

#[async_trait]
impl Tool for ArcDelegationTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.inner.execute(args).await
    }
}

fn boxed_registry_from_arcs(tools: Vec<Arc<dyn Tool>>) -> Vec<Box<dyn Tool>>{
    tools.into_iter().map(ArcDelegationTool::boxed).collect()
    // tools.iter().map(|t| ArcDelegationTool::boxed((*t).clone())).collect()
}

pub fn default_tools(security: Arc<SecurityPolicy>)-> Vec<Box<dyn Tool>>{
    default_tools_with_runtime(security, Arc::new(NativeRuntime::new()))
}

pub fn default_tools_with_runtime(security: Arc<SecurityPolicy>, runtime: Arc<dyn RuntimePlatformAdapter>)-> Vec<Box<dyn Tool>>{
    let persist_writes = runtime.has_filesystem_access();
    vec![
        // todo 最基本的几个tool shell file_read file_write file_edit global_search content_search
    ]
}

