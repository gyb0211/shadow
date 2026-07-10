use serde::{Deserialize, Serialize};
use crate::schema::{ResolvedRuntime, RuntimeProfileConfig};

fn default_true() ->bool{true}
fn default_false()->bool{false}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasedAgentConfig{

    #[serde(default="default_true")]
    pub enabled: bool,

    #[serde(default)]
    pub memory: AgentMemoryConfig,

 
    #[serde(default)]
    pub model_provider: crate::providers::ModelProviderRef,


    #[serde(default)]
    pub risk_profile:  crate::providers::RiskProfileRef,

 
    #[serde(default)]
    pub runtime_profile:  crate::providers::RuntimeProfileRef,


    #[serde(skip)]
    pub resolved: ResolvedRuntime,

}


#[derive(Debug, Clone,Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentMemoryConfig{
    pub backend: MemoryBackendKind,
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryBackendKind {
    None,
    #[default]
    Sqlite,
    Postgres,
    Markdown,
    Lucid,
}