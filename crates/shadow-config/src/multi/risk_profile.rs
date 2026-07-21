use crate::autonomy::{AutonomyLevel, DelegationPolicy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskProfileConfig {
    pub level: AutonomyLevel,
    pub workspace_only: bool,
    pub allowed_commands: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,

    pub shell_env_passthrough: Vec<String>,
    pub auto_approve: Vec<String>,

    pub always_ask: Vec<String>,

    #[serde(alias = "allowed_path", alias = "allowed_paths")]
    pub allowed_roots: Vec<String>,

    #[serde(default)]
    pub delegation_policy: DelegationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_route: Option<crate::autonomy::ApprovalRoute>,
    pub allowed_tools: Vec<String>,
    pub excluded_tools: Vec<String>,
    pub sandbox_enabled: Option<bool>,
    pub sandbox_backend: Option<String>,
    pub firejail_args: Vec<String>,
}

impl Default for RiskProfileConfig {
    fn default() -> Self {
        Self {
            level: AutonomyLevel::Supervised,
            workspace_only: true,
            allowed_commands: vec![],
            forbidden_paths: vec![],
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            shell_env_passthrough: vec![],
            auto_approve: vec![],
            always_ask: vec![],
            allowed_roots: vec![],
            delegation_policy: DelegationPolicy::default(),
            approval_route: None,
            allowed_tools: vec![],
            excluded_tools: vec![],
            sandbox_enabled: None,
            sandbox_backend: None,
            firejail_args: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SandboxBackend {
    #[default]
    Auto,
    Landlock,
    Firejail,
    Bubblewrap,
    Docker,
    #[serde(alias = "sandbox-exec")]
    SandboxExec,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub backend: SandboxBackend,
    #[serde(default)]
    pub firejail_args: Vec<String>,
}

impl RiskProfileConfig {
    pub fn sandbox_config(&self) -> SandboxConfig {
        let backend = self.sandbox_backend.as_deref().map(str::trim).filter(|s| !s.is_empty())
            .map(parse_sandbox_backend).unwrap_or_default();
        SandboxConfig{
            enabled: self.sandbox_enabled,
            backend,
            firejail_args: self.firejail_args.clone(),
        }
    }
}


fn parse_sandbox_backend(name: &str) -> SandboxBackend {
    match name { 
        "auto" => SandboxBackend::Auto,
        "landlock" => SandboxBackend::Landlock,
        "firejail" => SandboxBackend::Firejail,
        "bubblewrap" => SandboxBackend::Bubblewrap,
        "docker" => SandboxBackend::Docker,
        "sandbox-exec" | "sandboxexec" | "seatbelt" => SandboxBackend::SandboxExec,
        "none" => SandboxBackend::None,
        _ => SandboxBackend::default(),
    }
}
