//! 配置 schema -- 顶层 Config + 各配置段

pub use crate::multi::alias_agent::AliasedAgentConfig;
pub use crate::multi::risk_profile::RiskProfileConfig;
pub use crate::multi::runtime_profile::RuntimeProfileConfig;
pub use crate::multi::skill_bundle::SkillBundleConfig;

pub use crate::model_provider::*;

use crate::ReliableConfig;
use crate::channel::{LarkReceiveMode, StreamMode};
use crate::multi::alias_agent::MemoryBackendKind;
use crate::observability::ObservabilityBackend;
use crate::peer_group::PeerGroupConfig;
use crate::providers::{ModelProviderRef, ModelProviders, Providers};
use anyhow::Context;
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::future::poll_fn;
use std::path::{Path, PathBuf};
use tokio::fs;

/// 顶层配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct Config {
    /// schema 版本号 -- 用于未来迁移。新配置默认 = CURRENT_SCHEMA_VERSION。
    #[serde(default)]
    pub schema_version: u32,

    #[serde(skip)]
    pub config_path: PathBuf,
    #[serde(skip)]
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

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_routes: Vec<ModelRouteConfig>,

    #[serde(default)]
    pub runtime: RuntimeConfig,

    #[serde(default)]
    pub reliability: ReliableConfig,

    #[serde(default)]
    pub scheduler: SchedulerConfig,

    #[serde(default = "default_memory_backend")]
    pub memory_backend: String,

    #[serde(default)]
    pub memory: MemoryConfig,

    #[serde(default)]
    pub storage: StorageConfig,

    #[serde(default)]
    pub observability: ObservabilityConfig,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedding_routes: Vec<EmbeddingRouteConfig>,

    #[serde(default, alias = "channels_config")]
    pub channels: ChannelsConfig,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub peer_groups: HashMap<String, PeerGroupConfig>,

    /// Jira 集成配置
    #[serde(default)]
    pub jira: JiraConfig,
}

/// Jira 集成配置 (`[jira]`)
///
/// 支持 Jira Server/DC（Basic Auth: 用户名+密码）
/// 和 Jira Cloud（Basic Auth: email+api_token）
///
/// 密码/API Token 从环境变量 `JIRA_PASSWORD` 读取，不存储在配置文件中
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JiraConfig {
    /// 是否启用 Jira 工具
    pub enabled: bool,
    /// Jira 实例地址（如 http://jira.wb-intra.com）
    pub base_url: String,
    /// 用户名（Server/DC 认证）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Jira Cloud email（Cloud 认证，设置后用 API v3）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// 允许的操作列表
    #[serde(default = "default_jira_allowed_actions")]
    pub allowed_actions: Vec<String>,
    /// 请求超时（秒）
    #[serde(default = "default_jira_timeout_secs")]
    pub timeout_secs: u64,
}

impl JiraConfig {
    /// 从环境变量 JIRA_PASSWORD 读取密码
    ///
    /// 不存储在配置文件中，避免明文泄露
    pub fn password(&self) -> Option<String> {
        std::env::var("JIRA_PASSWORD").ok().filter(|s| !s.is_empty())
    }
}

fn default_jira_allowed_actions() -> Vec<String> {
    vec!["get_ticket".to_string()]
}

fn default_jira_timeout_secs() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsConfig {
    #[serde(default = "default_true")]
    pub cli: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub lark: HashMap<String, LarkConfig>,
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            cli: true,
            lark: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LarkConfig {
    #[serde(default)]
    pub enabled: bool,
    pub app_id: String,
    pub app_secret: String,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub mention_only: bool,
    #[serde(default)]
    pub use_feishu: bool,
    #[serde(default)]
    pub receive_mode: LarkReceiveMode,

    #[serde(default)]
    pub port: Option<u16>,

    #[serde(default)]
    pub proxy_url: Option<String>,

    #[serde(default = "default_channel_approval_timeout_secs")]
    pub approval_timeout_secs: u64,

    #[serde(default)]
    pub per_user_session: bool,

    #[serde(default)]
    pub ack_reactions: Option<bool>,

    #[serde(default)]
    pub stream_mode: StreamMode,

    #[serde(default = "default_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
}

fn default_draft_update_interval_ms() -> u64 {
    1000
}

fn default_multi_message_delay_ms() -> u64 {
    800
}

fn default_channel_approval_timeout_secs() -> u64 {
    300
}

fn default_matrix_draft_update_interval_ms() -> u64 {
    1500
}

fn default_memory_backend() -> String {
    "sqlite".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 0,
            config_path: Default::default(),
            data_dir: Default::default(),
            agents: HashMap::new(),
            risk_profiles: HashMap::new(),
            runtime_profiles: Default::default(),
            skill_bundles: Default::default(),
            providers: Providers::default(),
            model_routes: vec![],
            runtime: Default::default(),
            reliability: Default::default(),
            scheduler: Default::default(),
            memory_backend: "sqlite".to_string(),
            memory: Default::default(),
            storage: Default::default(),
            observability: Default::default(),
            embedding_routes: vec![],
            channels: ChannelsConfig::default(),
            peer_groups: Default::default(),
            jira: Default::default(),
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

    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home).join(".shadow"));
    }

    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home.join(".shadow"))
}

async fn resolve_runtime_config_dirs(
    default_shadow_dir: &PathBuf,
    default_data_dir: &PathBuf,
) -> anyhow::Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    if let Ok(custom_config_dir) = std::env::var("SHADOW_CONFIG_DIR") {}
    if let Ok(custom_data_dir) = std::env::var("SHADOW_DATA_DIR")
        && !custom_data_dir.trim().is_empty()
    {
        // let expanded = expand_tilde_path(&custom_data_dir);
        // let (shadow_dir, data_dir) = resolve_config_dir_for_data(&expanded);
        // return Ok(shadow_dir, data_dir, ConfigResolutionSource::EnvDataDir);
    }

    if let Ok(custom_workspace) = std::env::var("SHADOW_WORKSPACE")
        && !custom_workspace.trim().is_empty()
    {
        // let expanded = expand_tilde_path(&custom_data_dir);
        // let (shadow_dir, data_dir) = resolve_config_dir_for_data(&expanded);
        // return Ok(shadow_dir, data_dir, ConfigResolutionSource::EnvWorkspaceLegacy);
    }

    if cfg!(target_os = "macos")
        && let Ok(exe) = std::env::current_exe()
        && let Some(homebrew_config_dir) = try_resolve_macos_homebrew_config_dir(&exe).await
    {
        return Ok((
            homebrew_config_dir.clone(),
            homebrew_config_dir.join("workspace"),
            ConfigResolutionSource::HomebrewConfigDir,
        ));
    }

    Ok((
        default_shadow_dir.to_path_buf(),
        default_data_dir.to_path_buf(),
        ConfigResolutionSource::DefaultConfigDir,
    ))
}

async fn try_resolve_macos_homebrew_config_dir(exe: &PathBuf) -> Option<PathBuf> {
    let parts = exe.iter().collect::<Vec<_>>();
    let prefix = match parts.as_slice() {
        [prefix @ .., cellar, formula, _version, bin, exe_name]
            if os_str_eq(cellar, "Cellar")
                && os_str_eq(formula, "shadow")
                && os_str_eq(bin, "bin")
                && os_str_eq(exe_name, "shadow") =>
        {
            prefix.iter().collect::<PathBuf>()
        }
        [prefix @ .., opt, formula, bin, exe_name]
            if os_str_eq(opt, "opt")
                && os_str_eq(formula, "shadow")
                && os_str_eq(bin, "bin")
                && os_str_eq(exe_name, "shadow") =>
        {
            let prefix = prefix.iter().collect::<PathBuf>();
            if !prefix.as_os_str().is_empty()
                && fs::metadata(prefix.join("Cellar"))
                    .await
                    .is_ok_and(|metadata| metadata.is_dir())
            {
                prefix
            } else {
                return None;
            }
        }
        [prefix @ .., bin, exe_name] if os_str_eq(bin, "bin") && os_str_eq(exe_name, "shadow") => {
            let prefix = prefix.iter().collect::<PathBuf>();
            if !prefix.as_os_str().is_empty()
                && fs::metadata(prefix.join("Cellar"))
                    .await
                    .is_ok_and(|metadata| metadata.is_dir())
            {
                prefix
            } else {
                return None;
            }
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
    pub fn channel_external_peers(&self, channel_type: &str, alias: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for group in self.peer_groups.values() {
            let group_matches = match group.channel.split_once('.') {
                Some((ty, al)) => ty == channel_type && al == alias,
                None => group.channel == channel_type,
            };

            if !group_matches {
                continue;
            }

            for peer in &group.external_peers {
                let username = peer.as_str().to_string();
                if seen.insert(username.clone()) {
                    out.push(username)
                }
            }
        }
        out
    }

    pub fn agent(&self, agent_alias: &str) -> Option<&AliasedAgentConfig> {
        self.agents.get(agent_alias)
    }

    pub fn model_provider_for_agent(&self, agent_alias: &str) -> Option<&ModelProviderConfig> {
        let agent = self.agents.get(agent_alias)?;

        let (type_key, alias_key) = agent.model_provider.split_once(".")?;

        self.providers.models.find(type_key, alias_key)
    }

    /// 获取 TTS API Key（复用 MiniMax model provider 的 api_key）
    pub fn tts_api_key(&self) -> Option<String> {
        // 从第一个 agent 的 model provider 中获取 api_key
        let agent = self.agents.values().next()?;
        let (type_key, alias_key) = agent.model_provider.split_once(".")?;
        let provider = self.providers.models.find(type_key, alias_key)?;
        provider.api_key.clone().filter(|k| !k.is_empty())
    }

    /// 解析 agent 的 TTS provider 配置
    ///
    /// agent.tts_provider = "minimax.my_voice" → 查找 [providers.tts.minimax.my_voice]
    pub fn tts_provider_for_agent(&self, agent_alias: &str) -> Option<&crate::providers::TtsProviderConfig> {
        let agent = self.agents.get(agent_alias)?;
        let tts_ref = agent.tts_provider.as_str();
        if tts_ref.is_empty() {
            return None;
        }
        // 解析 "minimax.my_voice" → family="minimax", alias="my_voice"
        let (family, alias) = tts_ref.split_once(".")?;
        match family {
            "minimax" => self.providers.tts.minimax.get(alias),
            _ => None,
        }
    }

    pub fn resolved_model_provider_for_agent(
        &self,
        agent_alias: &str,
    ) -> Option<(&str, &str, &ModelProviderConfig)> {
        let agent = self.agents.get(agent_alias)?;
        let (type_key, alias_key) = agent.model_provider.split_once(".")?;
        self.providers
            .models
            .iter_entries()
            .find(|(ty, alias, _)| *ty == type_key && *alias == alias_key)
    }

    pub fn resolved_agent_config(&self, agent_alias: &str) -> Option<AliasedAgentConfig> {
        let mut cfg = self.agents.get(agent_alias)?.clone();
        let runtime_profile_cfg = self.runtime_profile_for_agent(agent_alias);
        let mut resolved = ResolvedRuntime {
            max_tool_iterations: runtime_profile_cfg
                .map(|c| c.max_tool_iterations)
                .filter(|&v| v > 0)
                .unwrap_or(10),
        };
        if let Some(profile) = runtime_profile_cfg {}
        cfg.resolved = resolved;
        Some(cfg)
    }

    pub fn agent_workspace_dir(&self, agent_alias: &str) -> PathBuf {
        if let Some(cfg) = self.agents.get(agent_alias)
            && let Some(custom) = cfg.workspace.path.as_ref()
        {
            return custom.clone();
        }

        self.install_root_dir()
            .join("agents")
            .join(agent_alias)
            .join("workspace")
    }
    pub fn install_root_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn runtime_profile_for_agent(&self, agent_alias: &str) -> Option<&RuntimeProfileConfig> {
        self.runtime_profiles.get(agent_alias)
    }

    pub fn risk_profile_for_agent(&self, agent_alias: &str) -> Option<&RiskProfileConfig> {
        self.risk_profiles.get(agent_alias)
    }

    pub async fn load_or_init() -> anyhow::Result<Self> {
        let (default_shadow_dir, default_workspace_dir) = default_config_and_data_dirs()?;
        let (shadow_dir, _legacy_workspace_dir, resolution_source) =
            resolve_runtime_config_dirs(&default_shadow_dir, &default_workspace_dir).await?;

        let config_path = shadow_dir.join("config.toml");
        let data_dir = shadow_dir.join("data");
        fs::create_dir_all(&data_dir).await.with_context(|| {
            format!(
                "Failed to create data directory: {}",
                data_dir.display().to_string()
            )
        })?;

        let workspace_dir = data_dir;

        let shared_dir = shadow_dir.join("shared");
        fs::create_dir_all(&shared_dir).await.with_context(|| {
            format!(
                "Failed to create shared directory: {}",
                shared_dir.display().to_string()
            )
        })?;

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read config file")?;

            // 先运行 migration 链 (v1→v2 ...), 若发生迁移则回写文件
            let contents = match crate::migration::migrate_str(&contents) {
                Ok(Some(migrated)) => {
                    fs::write(&config_path, &migrated).await?;
                    migrated
                }
                Ok(None) => contents,
                Err(e) => {
                    tracing::warn!("Config migration failed, using raw content: {e}");
                    contents
                }
            };

            let mut config: Config = toml::from_str(&contents).unwrap_or_else(|e| {
                tracing::error!(
                    "Failed to parse config.toml as TOML: {e}, falling back to default"
                );
                Config::default()
            });

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
            // 首次运行：确保目录存在并设置 config_path/data_dir，
            // 否则后续 save() 会因为 config_path 为空而失败。
            let mut config = Config::default();
            config.config_path = config_path.clone();
            config.data_dir = workspace_dir;
            Ok(config)
        }
    }

    /// 将当前配置序列化为 TOML 并写回 config_path.
    ///
    /// `config_path` 和 `data_dir` 不会写入文件 (serde skip).
    /// 调用前需通过 `load_or_init` 或 Default 设置 `config_path`.
    pub async fn save(&self) -> anyhow::Result<()> {
        let toml_str = toml::to_string_pretty(self)?;
        fs::write(&self.config_path, toml_str).await?;
        Ok(())
    }

    pub fn resolve_active_storage(&self) -> ActiveStorage<'_> {
        let backend = self.memory.backend.trim();
        if backend.is_empty() || backend.eq_ignore_ascii_case("none") {
            return ActiveStorage::None;
        }

        let (kind, alias) = backend.split_once(".").unwrap_or((backend, "default"));
        match kind {
            "sqlite" => self
                .storage
                .sqlite
                .get(alias)
                .map(ActiveStorage::Sqlite)
                .unwrap_or(ActiveStorage::None),

            _ => ActiveStorage::None,
        }
    }

    #[must_use]
    pub fn shared_workspace_dir(&self) -> std::path::PathBuf {
        self.install_root_dir().join("shared")
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ActiveStorage<'a> {
    None,
    Sqlite(&'a SqliteStorageConfig),
}
impl ActiveStorage<'_> {
    pub fn kind(&self) -> &'static str {
        match self {
            ActiveStorage::None => "none",
            ActiveStorage::Sqlite(_) => "sqlite",
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SqliteStorageConfig {
    pub path: Option<String>,
    pub open_timeout_secs: Option<u64>,
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

fn default_true() -> bool {
    true
}
fn default_scheduler_enabled() -> bool {
    true
}
fn default_max_tasks() -> usize {
    64
}
fn default_max_concurrent() -> usize {
    4
}
fn default_max_run_history() -> u32 {
    50
}

#[derive(Debug, Clone)]
pub struct ResolvedRuntime {
    pub max_tool_iterations: usize,
}

impl Default for ResolvedRuntime {
    fn default() -> Self {
        Self {
            max_tool_iterations: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub backend: ObservabilityBackend,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            backend: ObservabilityBackend::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub backend: String,
    pub auto_save: bool,
    pub hygiene_enabled: bool,
    pub archive_after_days: u32,
    pub purge_after_days: u32,
    pub conversation_retention_days: u32,
    pub core_retention_days: u32,
    pub daily_retention_days: u32,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_dimensions: usize,
    pub embedding_api_key: Option<String>,
    pub vector_weight: f64,
    pub keyword_weight: f64,
    pub min_relevance_score: f64,
    pub embedding_cache_size: usize,
    pub search_mode: SearchMode,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: "".to_string(),
            auto_save: false,
            hygiene_enabled: false,
            archive_after_days: 0,
            purge_after_days: 0,
            conversation_retention_days: 0,
            core_retention_days: 0,
            daily_retention_days: 0,
            embedding_provider: "".to_string(),
            embedding_model: "".to_string(),
            embedding_dimensions: 0,
            embedding_api_key: None,
            vector_weight: 0.0,
            keyword_weight: 0.0,
            min_relevance_score: 0.0,
            embedding_cache_size: 0,
            search_mode: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    sqlite: HashMap<String, SqliteStorageConfig>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            sqlite: HashMap::new(),
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRouteConfig {
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub model_provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub dimensions: Option<usize>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Bm25,
    Embedding,
    #[default]
    Hubrid,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    #[default]
    Native,
    Docker,
    Cloudflare,
}

impl RuntimeKind {
    #[must_use]
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Docker => "docker",
            Self::Cloudflare => "cloudflare",
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Runtime kind: native | docker | cloudflare.
    #[serde(default, deserialize_with = "deserialize_enum_lenient")]
    pub kind: RuntimeKind,

    /// Docker runtime settings (used when `kind = "docker"`).
    #[serde(default)]
    pub docker: DockerRuntimeConfig,

    /// Shell binary the native runtime uses for command execution.
    ///
    /// Applies only to `runtime.kind = "native"`; other runtimes ignore it.
    /// When unset or `null`, the system default `sh` is used. The shell is
    /// invoked as `<shell> -c "<command>"`, so it must be a POSIX-compatible
    /// shell binary.
    ///
    /// Accepted forms (Unix):
    /// - a bare command name resolved via `PATH` (e.g. `"bash"`), or
    /// - an absolute path (e.g. `"/bin/bash"`, `"/usr/bin/zsh"`).
    ///
    /// The value is validated when the native runtime is constructed, so a bad
    /// value is reported up front rather than failing on the first shell
    /// command. Rejected: empty/whitespace; a relative path with separators
    /// (e.g. `"./sh"`, `"bin/sh"` — use a bare `PATH` name or an absolute path
    /// instead); a bare name not found on `PATH`; and a path that does not
    /// exist or is not executable.
    ///
    /// **Ignored on Windows and Android** (and not validated there): Windows
    /// always uses `cmd.exe`, and Android always uses `/system/bin/sh`
    /// (its shell is not on `PATH` for spawned processes).
    ///
    /// **Examples:**
    /// ```toml
    /// [runtime]
    /// shell = "bash"           # resolves via PATH
    /// shell = "/bin/zsh"       # absolute path
    /// ```
    #[serde(default)]
    pub shell: Option<String>,

    /// Global reasoning override for model_providers that expose explicit controls.
    /// - `None`: model_provider default behavior
    /// - `Some(true)`: request reasoning/thinking when supported
    /// - `Some(false)`: disable reasoning/thinking when supported
    #[serde(default)]
    pub reasoning_enabled: Option<bool>,
    /// Optional reasoning effort for model_providers that expose a level control.
    #[serde(default, deserialize_with = "deserialize_reasoning_effort_opt")]
    pub reasoning_effort: Option<String>,
}

/// Docker runtime configuration (`[runtime.docker]` section).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerRuntimeConfig {
    /// Runtime image used to execute shell commands.
    #[serde(default = "default_docker_image")]
    pub image: String,

    /// Docker network mode (`none`, `bridge`, etc.).
    #[serde(default = "default_docker_network")]
    pub network: String,

    /// Optional memory limit in MB (`None` = no explicit limit).
    #[serde(default = "default_docker_memory_limit_mb")]
    pub memory_limit_mb: Option<u64>,

    /// Optional CPU limit (`None` = no explicit limit).
    #[serde(default = "default_docker_cpu_limit")]
    pub cpu_limit: Option<f64>,

    /// Mount root filesystem as read-only.
    #[serde(default = "default_true")]
    pub read_only_rootfs: bool,

    /// Mount configured workspace into `/workspace`.
    #[serde(default = "default_true")]
    pub mount_workspace: bool,

    /// Optional workspace root allowlist for Docker mount validation.
    #[serde(default)]
    pub allowed_workspace_roots: Vec<String>,
}

fn default_docker_image() -> String {
    "alpine:3.20".into()
}

fn default_docker_network() -> String {
    "none".into()
}

fn default_docker_memory_limit_mb() -> Option<u64> {
    Some(512)
}

fn default_docker_cpu_limit() -> Option<f64> {
    Some(1.0)
}

impl Default for DockerRuntimeConfig {
    fn default() -> Self {
        Self {
            image: default_docker_image(),
            network: default_docker_network(),
            memory_limit_mb: default_docker_memory_limit_mb(),
            cpu_limit: default_docker_cpu_limit(),
            read_only_rootfs: true,
            mount_workspace: true,
            allowed_workspace_roots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelRouteConfig {
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub model_provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

fn deserialize_reasoning_effort_opt<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<String> = Option::deserialize(deserializer)?;
    value
        .map(|raw| normalize_reasoning_effort(&raw).map_err(serde::de::Error::custom))
        .transpose()
}

fn deserialize_enum_lenient<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(T::deserialize(raw).unwrap_or_default())
}

/// Deserialize an `Option<String>` that maps an empty literal `""` to
/// `None`. Used by `JiraConfig::email` so a config that round-tripped
/// `email = ""` to disk (the legacy `email: String` had no
/// `skip_serializing_if`) doesn't deserialize as `Some("")` and silently
/// break Basic auth — the email-required validation was removed when
/// Server/DC Bearer-token support landed, so this is the last line of
/// defense.
fn deserialize_optional_email_skip_empty<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<String> = Option::deserialize(deserializer)?;
    Ok(value.filter(|s| !s.trim().is_empty()))
}

fn normalize_reasoning_effort(value: &str) -> std::result::Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "minimal" | "low" | "medium" | "high" | "xhigh" => Ok(normalized),
        _ => Err(format!(
            "reasoning_effort {value:?} is invalid (expected one of: minimal, low, medium, high, xhigh)"
        )),
    }
}

pub trait FamilyEndpoint {
    fn endpoint_uri(&self) -> Option<&'static str> {
        None
    }
}

macro_rules! impl_default_family_endpoint {
    ($($t:ty), + $(,)?) => {
        $(impl FamilyEndpoint for $t {})+
    };

}
impl_default_family_endpoint! {
CustomModelProviderConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tom_roundtrip_minimal() {
        // Providers 有 models 字段, 所以 TOML 路径是 [providers.models.custom.<alias>]
        let toml_str = r#"
schema_version = 2

[providers.models.custom.default]
api_key = "sk-test"
model = "gpt-4o-mini"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.schema_version, 2);
        let entry = config.providers.models.custom.get("default").unwrap();
        assert_eq!(entry.base.api_key.as_deref(), Some("sk-test"));
        assert_eq!(entry.base.model.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn tom_lark_channel() {
        let toml_str = r#"
schema_version = 2

[channels.lark.mybot]
app_id = "cli_xxx"
app_secret = "secret"
use_feishu = true
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let lark = config.channels.lark.get("mybot").unwrap();
        assert_eq!(lark.app_id, "cli_xxx");
        assert_eq!(lark.app_secret, "secret");
        assert!(lark.use_feishu);
    }

    #[test]
    fn tom_serialize_skips_runtime_paths() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).unwrap();
        assert!(!toml_str.contains("config_path"));
        assert!(!toml_str.contains("data_dir"));
    }
}
