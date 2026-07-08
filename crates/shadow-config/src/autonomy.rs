use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    #[default]
    ReadOnly,
    Supervised,
    Full,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct DelegationPolicy {
    model: DelegationMode,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum DelegationMode {
    #[default]
    Forbidden,
    Allow,
}
impl DelegationPolicy {
    pub fn permits(&self) -> bool {
        matches!(self.model, DelegationMode::Allow)
    }
}


fn default_approver_timeout_secs() -> u64 {
    600
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum OnNoApprover{
    ///
    #[default]
    Deny,
    InheritOriginator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct ApprovalRoute {
    /// 指定一个审批渠道
    pub approver_channel: String,
    /// 当审批失败时
    #[serde(default)]
    pub no_on_approver: OnNoApprover,
    /// 超时自动拒绝
    #[serde(default="default_approver_timeout_secs")]
    pub timeout_secs:u64,
}
