//! 配置 schema -- 顶层 Config + 各配置段

pub use crate::multi::alias_agent::AliasedAgentConfig;
pub use crate::multi::risk_profile::RiskProfileConfig;
pub use crate::multi::runtime_profile::RuntimeProfileConfig;
pub use crate::multi::skill_bundle::SkillBundleConfig;

pub use crate::model_provider::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::future::poll_fn;
use std::path::PathBuf;
use anyhow::Context;
use directories::UserDirs;
use tokio::fs;
use crate::providers::{ModelProviderRef, ModelProviders, Providers};

/// 顶层配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct Config {
    /// schema 版本号 -- 用于未来迁移。新配置默认 = CURRENT_SCHEMA_VERSION。
    #[serde(default)]
    pub schema_version: u32,

    pub config_path: PathBuf,
    pub data_dir: PathBuf,

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

    #[serde(default)]
    pub scheduler: SchedulerConfig,
}

impl Default for Config {
    fn default() -> Self {
        let home = UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
        let shadow_home = home.join(".shadow");
        let mut agents = HashMap::new();
        agents.insert("assistant".to_string() , AliasedAgentConfig{
            enabled: true,
            model_provider: ModelProviderRef::new("custom.default"),
            risk_profile: Default::default(),
            runtime_profile: Default::default(),
        });
        let mut pdf = Providers::default();
        pdf.models = ModelProviders::default();
        pdf.models.custom = HashMap::new();
        pdf.models.custom.insert("default".to_string(), CustomModelProviderConfig{
            base: ModelProviderConfig {
                api_key: Some("sk-cp-18TqfpOSbQSSNkZNvjO01R3euRRLhj7zreKCW1ssrFficr3yWC3rBzylPD6Nnw2V450mmZu9Q5s6p0CsqAInQOq1r3ZBzYpl_UUG_PkNiVGl4s5OduwRiFE".to_string()),
                kind: None,
                url: Some("https://api.minimaxi.com/v1".to_string()),
                model: Some("MiniMax-M2.7".to_string()),
                temperature: None,
                timeout_secs: None,
                extra_headers: Default::default(),
                response_max_tokens: None,
                native_tools: None,
                think: None,
                context_window: None,
            },
        });
        Self {
            schema_version: 0,
            config_path: Default::default(),
            data_dir: Default::default(),
            agents,
            risk_profiles: Default::default(),
            runtime_profiles: Default::default(),
            skill_bundles: Default::default(),
            providers: pdf,
            scheduler: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigResolutionSource {
    EnvConfigDir,
    EnvDataDir,
    EnvWorkspaceLegacy,
    DefaultConfigDir,
    HomebrewConfigDir,
}

impl ConfigResolutionSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::EnvConfigDir => "SHADOW_CONFIG_DIR",
            Self::EnvDataDir => "SHADOW_DATA_DIR",
            Self::EnvWorkspaceLegacy => "SHADOW_WORKSPACE",
            Self::DefaultConfigDir => "default",
            Self::HomebrewConfigDir => "homebrew",
        }
    }
}


fn default_config_and_data_dirs() -> anyhow::Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("data")))
}


fn default_config_dir() -> anyhow::Result<PathBuf> {
    if let Ok(custom) = std::env::var("SHADOW_CONFIG_DIR") {
        let custom = custom.trim();
        if !custom.is_empty() {
            // 扩大shell可运行范围
            return Ok(expand_tilde_path(custom));
        }
    }

    if let Ok(home) = std::env::var("HOME") && !home.is_empty() {
        return Ok(PathBuf::from(home).join(".shadow"));
    }

    let home = UserDirs::new().map(|u| u.home_dir().to_path_buf()).context("Could not find home directory")?;
    Ok(home.join(".shadow"))
}

async fn resolve_runtime_config_dirs(default_shadow_dir: &PathBuf, default_data_dir: &PathBuf) -> anyhow::Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    if let Ok(custom_config_dir) = std::env::var("SHADOW_CONFIG_DIR") {}
    if let Ok(custom_data_dir) = std::env::var("SHADOW_DATA_DIR") && !custom_data_dir.trim().is_empty() {
        // let expanded = expand_tilde_path(&custom_data_dir);
        // let (shadow_dir, data_dir) = resolve_config_dir_for_data(&expanded);
        // return Ok(shadow_dir, data_dir, ConfigResolutionSource::EnvDataDir);
    }

    if let Ok(custom_workspace) = std::env::var("SHADOW_WORKSPACE") && !custom_workspace.trim().is_empty() {
        // let expanded = expand_tilde_path(&custom_data_dir);
        // let (shadow_dir, data_dir) = resolve_config_dir_for_data(&expanded);
        // return Ok(shadow_dir, data_dir, ConfigResolutionSource::EnvWorkspaceLegacy);
    }


    if cfg!(target_os = "macos") && let Ok(exe) = std::env::current_exe()
        && let Some(homebrew_config_dir) = try_resolve_macos_homebrew_config_dir(&exe).await {
        return Ok(
            (homebrew_config_dir.clone(),
             homebrew_config_dir.join("workspace"), ConfigResolutionSource::HomebrewConfigDir)
        );
    }

    Ok((default_data_dir.to_path_buf(), default_data_dir.to_path_buf(), ConfigResolutionSource::DefaultConfigDir))
}

async fn try_resolve_macos_homebrew_config_dir(exe: &PathBuf) -> Option<PathBuf> {
    let parts = exe.iter().collect::<Vec<_>>();
    let prefix = match parts.as_slice() {
        [prefix @ .., cellar, formula, _version, bin, exe_name]
        if os_str_eq(cellar, "Cellar") && os_str_eq(formula, "shadow") && os_str_eq(bin, "bin") && os_str_eq(exe_name, "shadow") => prefix.iter().collect::<PathBuf>(),
        [prefix @ .., opt, formula, bin, exe_name]
        if os_str_eq(opt, "opt") && os_str_eq(formula, "shadow") && os_str_eq(bin, "bin") && os_str_eq(exe_name, "shadow") => {
            let prefix = prefix.iter().collect::<PathBuf>();
            if !prefix.as_os_str().is_empty() && fs::metadata(prefix.join("Cellar")).await.is_ok_and(|metadata| metadata.is_dir()) {
                prefix
            } else { return None; }
        }
        [prefix @ .., bin, exe_name]
        if os_str_eq(bin, "bin") && os_str_eq(exe_name, "shadow") => {
            let prefix = prefix.iter().collect::<PathBuf>();
            if !prefix.as_os_str().is_empty() && fs::metadata(prefix.join("Cellar")).await.is_ok_and(|metadata| metadata.is_dir()) {
                prefix
            } else { return None; }
        }
        _ => {
            return None;
        }
    };

    Some(prefix.join("var").join("shadow"))
}

fn os_str_eq(cellar: &&OsStr, os: &str) -> bool {
    *cellar == std::ffi::OsStr::new(os)
}

fn expand_tilde_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    let expanded_str = expanded.as_ref();

    if expanded_str.starts_with('~') {
        if let Some(user_dirs) = UserDirs::new() {
            let home = user_dirs.home_dir();
            if let Some(rest) = expanded_str.strip_prefix('~') {
                return home.join(rest.trim_start_matches(['/', '\\']));
            }
        }
    }

    PathBuf::from(expanded_str)
}

impl Config {
    pub fn agent(&self, agent_alias: &str) -> Option<&AliasedAgentConfig> {
        self.agents.get(agent_alias)
    }

    pub fn model_provider_for_agent(&self, agent_alias: &str) -> Option<&ModelProviderConfig> {
        let agent = self.agents.get(agent_alias)?;

        let (type_key, alias_key) = agent.model_provider.split_once(".")?;

        self.providers.models.find(type_key, alias_key)
    }

    pub fn resolved_model_provider_for_agent(&self, agent_alias: &String) -> Option<(&str, &str, &ModelProviderConfig)> {
        let agent = self.agents.get(agent_alias)?;
        let (type_key, alias_key) = agent.model_provider.split_once(".")?;
        self.providers.models.iter_entries().find(|(ty, alias, _)| *ty == type_key && *alias == alias_key)
    }

    pub async fn load_or_init() -> anyhow::Result<Self> {
        let (default_shadow_dir, default_workspace_dir) = default_config_and_data_dirs()?;
        let (shadow_dir, _legacy_workspace_dir, resolution_source) = resolve_runtime_config_dirs(&default_shadow_dir, &default_workspace_dir).await?;

        let config_path = shadow_dir.join("config.toml");
        let data_dir = shadow_dir.join("data");
        fs::create_dir_all(&data_dir).await.with_context(|| format!("Failed to create data directory: {}", data_dir.display().to_string()))?;

        let workspace_dir = data_dir;

        let shared_dir = shadow_dir.join("shared");
        fs::create_dir_all(&shared_dir).await.with_context(|| format!("Failed to create shared directory: {}", shared_dir.display().to_string()))?;

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path).await.context("Failed to read config file")?;
            let mut config = serde_json::from_str::<Config>(&contents).unwrap_or(Config::default());


            if let Some(default_profile) = config.risk_profiles.get_mut("default") {
                // default_profile.ensure_default_auto_approve();

            }

            config.config_path = config_path.clone();

            config.data_dir = workspace_dir;
            // todo skill

            // todo secret

            // todo env


            // todo validate


            Ok(config)
        } else {
            Ok(Config::default())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct SchedulerConfig {
    #[serde(default = "default_scheduler_enabled")]
    pub enabled: bool,
    #[serde(default = "default_max_tasks")]
    pub max_tasks: usize,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default = "default_true")]
    pub catch_up_on_startup: bool,
    #[serde(default = "default_max_run_history")]
    pub max_run_history: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: default_scheduler_enabled(),
            max_tasks: default_max_tasks(),
            max_concurrent: default_max_concurrent(),
            catch_up_on_startup: false,
            max_run_history: default_max_run_history(),
        }
    }
}

fn default_true() -> bool { true }
fn default_scheduler_enabled() -> bool { true }
fn default_max_tasks() -> usize { 64 }
fn default_max_concurrent() -> usize { 4 }
fn default_max_run_history() -> u32 { 50 }
