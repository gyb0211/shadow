use serde::{Deserialize, Serialize};
use crate::autonomy::AutonomyLevel;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskProfileConfig{
    pub level: AutonomyLevel,
    pub workspace_only:bool,
    pub allowed_commands:Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub require_approval_for_medium_risk:bool,
    pub block_high_risk_commands: bool,
    #[credential_class = "legacy_env_path"]
    pub shell_env_passthrough: Vec<String>,
    pub auto_approve: Vec<String>,
    #[serde(alias = "allowed_path", alias = "allowed_paths")]
    pub allowed_roots:Vec<String>,

    #[serde(default)]
    #[nested]
    pub delegation_policy: DelegationPolicy,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub approval_route: Option<crate::autonomy::ApprovalRoute>,
    pub allow_tools:Vec<String>,
    pub executed_tools:Vec<String>,
    pub sandbox_enabled: Option<bool>,
    pub sandbox_backend:Option<String>,
    pub firejail_args: Vec<String>

}

