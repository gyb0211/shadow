use serde::{Deserialize, Serialize};

fn default_true()->bool{true}
fn default_false()->bool{false}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasedAgentConfig{

    #[serde(default="default_true")]
    pub enabled: bool,

 
    #[serde(default)]
    pub model_provider: crate::providers::ModelProviderRef,


    #[serde(default)]
    pub risk_profile:  crate::providers::RiskProfileRef,

 
    #[serde(default)]
    pub runtime_profile:  crate::providers::RuntimeProfileRef,

}

impl AliasedAgentConfig {

}