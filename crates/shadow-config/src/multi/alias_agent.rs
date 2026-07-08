use serde::{Deserialize, Serialize};

fn default_true()->bool{true}
fn default_false()->bool{false}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasedAgentConfig{

    #[serde(default="default_true")]
    pub enabled: bool,

 
    #[serde(default)]
    pub model_provider: crate::provider::ModelProviderRef,


    #[serde(default)]
    pub risk_profile:  crate::provider::RiskProfileRef,

 
    #[serde(default)]
    pub runtime_profile:  crate::provider::RuntimeProfileRef,

}