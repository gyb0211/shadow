//! 配置 schema -- 顶层 Config + 各配置段

pub use crate::multi::alias_agent::AliasedAgentConfig;
pub use crate::multi::risk_profile::RiskProfileConfig;
pub use crate::multi::runtime_profile::RuntimeProfileConfig;
pub use crate::multi::skill_bundle::SkillBundleConfig;

pub use crate::model_provider::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;



/// 顶层配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct Config {
    /// schema 版本号 -- 用于未来迁移。新配置默认 = CURRENT_SCHEMA_VERSION。
    #[serde(default)]
    pub schema_version: u32,

    /// Aliased agents  [agents.<alias>]
    /// 代理映射关系
    #[serde(default)]
    pub agents: HashMap<String, AliasedAgentConfig>,

    #[serde(default)]
    pub risk_profiles: HashMap<String, RiskProfileConfig>,

    #[serde(default)]
    pub runtime_profiles: HashMap<String, RuntimeProfileConfig>,

    #[serde(default)]
    pub skill_bundles: HashMap<String, SkillBundleConfig>,

    #[serde(default)]
    pub providers: crate::providers::Providers,
}

impl Config {
    pub fn agent(&self, agent_alias: &str) -> Option<&AliasedAgentConfig> {
        self.agents.get(agent_alias)
    }

    pub fn model_provider_for_agent(&self, agent_alias: &str) -> Option<&ModelProviderConfig>{
        let agent = self.agents.get(agent_alias)?;

        let (type_key, alias_key) = agent.model_provider.split_once(".")?;

        self.providers.models.find(type_key, alias_key)

    }

    pub fn resolved_model_provider_for_agent(&self, agent_alias: &String) -> Option<(&str, &str, &ModelProviderConfig)> {
        let agent = self.agents.get(agent_alias)?;
        let (type_key, alias_key) = agent.model_provider.split_once(".")?;
        self.providers.models.iter_entries().find(|(ty, alias, _)| *ty == type_key && *alias == alias_key)
    }
}
