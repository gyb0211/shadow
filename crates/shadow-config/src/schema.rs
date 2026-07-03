//! 配置 schema -- 顶层 Config + 各配置段

use crate::provider::ProviderEntry;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 顶层配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// schema 版本号 -- 用于未来迁移。新配置默认 = CURRENT_SCHEMA_VERSION。
    #[serde(default)]
    pub schema_version: u32,

    #[serde(default)]
    pub agent: AgentSection,

    /// 多 provider 支持: providers.<family>.<alias>
    ///
    /// family 可以是 openai/anthropic/ollama/custom 等任意字符串。
    /// 每个 family 下可以有多个 alias (如 default, minimax1, glm2)。
    #[serde(default)]
    pub providers: ProvidersConfig,

    #[serde(default)]
    pub memory: MemorySection,

    /// Router 配置 (可选) -- 配置后启用跨 provider 路由与 fallback
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router: Option<crate::provider::RouterConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: crate::migration::CURRENT_SCHEMA_VERSION,
            agent: AgentSection::default(),
            providers: ProvidersConfig::default(),
            memory: MemorySection::default(),
            router: None,
        }
    }
}

/// [agent] 配置段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSection {
    /// agent 别名
    pub alias: String,

    /// provider 引用 -- "family.alias" 格式 (如 "openai.default", "custom.minimax1")
    /// 简写 "openai" 等价于 "openai.default"
    #[serde(default = "default_model_provider")]
    pub model_provider: String,

    /// 默认模型 -- 当 provider 条目未设 model 时使用
    #[serde(default = "default_model")]
    pub model: String,

    /// 采样温度 (0.0-2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// 自主级别: full / supervised / read_only
    #[serde(default = "default_autonomy")]
    pub autonomy: String,

    /// 工具调用最大循环次数 (默认 10)
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,

    /// 对话历史最大条数 (超过时自动截断旧消息, 默认 50)
    #[serde(default = "default_max_history")]
    pub max_history: usize,

    /// 自定义 system prompt (不设则用默认)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

fn default_model_provider() -> String {
    "openai.default".to_string()
}

fn default_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_autonomy() -> String {
    "supervised".to_string()
}

fn default_max_iterations() -> usize {
    10
}

fn default_max_history() -> usize {
    50
}

impl Default for AgentSection {
    fn default() -> Self {
        Self {
            alias: "default".to_string(),
            model_provider: default_model_provider(),
            model: default_model(),
            temperature: Some(0.7),
            autonomy: default_autonomy(),
            max_iterations: default_max_iterations(),
            max_history: default_max_history(),
            system_prompt: None,
        }
    }
}

/// [providers] 配置段 -- 多 provider 支持
///
/// TOML 形态:
/// ```toml
/// [providers.openai.default]
/// api_keys = ["sk-xxx"]      # 多 key 轮换; 也接受 api_key = "sk-xxx"
/// model = "gpt-4o-mini"
///
/// [providers.openai.default.reliable]
/// max_retries = 3
/// initial_backoff_ms = 1000
/// requests_per_minute = 60
///
/// [providers.custom.minimax1]
/// api_key = "xxx"
/// base_url = "https://api.minimax.chat/v1"
/// model = "abab6.5s-chat"
/// ```
///
/// 用 flatten HashMap 实现, family 和 alias 都是任意字符串。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    /// family -> alias -> ProviderEntry
    ///
    /// 使用 flatten 让 TOML 直接序列化为 `[providers.<family>.<alias>]` 块。
    #[serde(default, skip_serializing_if = "HashMap::is_empty", flatten)]
    pub families: HashMap<String, HashMap<String, ProviderEntry>>,
}

impl ProvidersConfig {
    /// 查找 provider 条目
    pub fn find(&self, family: &str, alias: &str) -> Option<&ProviderEntry> {
        self.families.get(family)?.get(alias)
    }

    /// 查找或创建 provider 条目 (用于 config set)
    pub fn find_or_create(&mut self, family: &str, alias: &str) -> &mut ProviderEntry {
        self.families
            .entry(family.to_string())
            .or_default()
            .entry(alias.to_string())
            .or_default()
    }

    /// 列出所有 (family, alias) 对
    pub fn list(&self) -> Vec<(String, String)> {
        crate::provider::list_providers(&self.families)
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }
}

/// [memory] 配置段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySection {
    /// 后端类型: none / markdown
    #[serde(default = "default_memory_backend")]
    pub backend: String,
}

fn default_memory_backend() -> String {
    "none".to_string()
}

impl Default for MemorySection {
    fn default() -> Self {
        Self {
            backend: default_memory_backend(),
        }
    }
}

// ── 路径与加载 ──

/// 配置目录 -- ~/.shadow/
#[must_use]
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SHADOW_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".shadow");
    }
    PathBuf::from(".shadow")
}

/// 配置文件路径
#[must_use]
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// 加载配置(使用 `SHADOW_CONFIG_DIR` / `~/.shadow`)。不存在则创建默认。
pub fn load_or_init() -> Result<Config> {
    load_from(&config_dir())
}

/// 从指定目录加载配置。目录既是配置文件所在,也是密钥文件 `.secret_key` 所在。
pub fn load_from(dir: &std::path::Path) -> Result<Config> {
    let path = dir.join("config.toml");
    let store = crate::secrets::SecretStore::new(dir, true)?;
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        // 迁移到当前 schema 版本(若需要)
        let migrated = crate::migration::migrate_str(&content)?;
        let toml_str = migrated.as_deref().unwrap_or(&content);
        let mut config: Config = toml::from_str(toml_str).unwrap_or_default();
        config.decrypt_secrets(&store);
        Ok(config)
    } else {
        let config = Config::default();
        save_to(&config, dir)?;
        Ok(config)
    }
}

/// 保存配置(使用 `SHADOW_CONFIG_DIR` / `~/.shadow`)。
pub fn save(config: &Config) -> Result<()> {
    save_to(config, &config_dir())
}

/// 保存配置到指定目录。原子写(tempfile + rename)+ 文件权限 0600 + 加密 api_key。
pub fn save_to(config: &Config, dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let store = crate::secrets::SecretStore::new(dir, true)?;
    // 克隆一份,加密后写盘 -- 不污染调用方的内存 config
    let mut to_write = config.clone();
    to_write.encrypt_secrets(&store);

    let path = dir.join("config.toml");
    let content = toml::to_string_pretty(&to_write)?;

    // 原子写: 在同目录建临时文件,写完 fsync 后 persist(rename)
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, content.as_bytes())?;
    restrict_permissions(tmp.as_file())?;
    tmp.as_file().sync_all()?;
    tmp.persist(&path).map_err(|e| anyhow::anyhow!("persist failed: {e}"))?;
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(file: &std::fs::File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| anyhow::anyhow!("set permissions: {e}"))
}

#[cfg(not(unix))]
fn restrict_permissions(_file: &std::fs::File) -> Result<()> {
    Ok(())
}

// ── Config 的密钥加密辅助 ──

impl Config {
    /// 加密所有 provider 的 api_keys(写盘前调用)。裸值透传 (失败则保留原值)。
    fn encrypt_secrets(&mut self, store: &crate::secrets::SecretStore) {
        for entry in self.iter_provider_entries_mut() {
            for key in entry.api_keys.iter_mut() {
                if let Ok(enc) = store.encrypt(key) {
                    if !enc.is_empty() {
                        *key = enc;
                    }
                }
            }
        }
    }

    /// 解密所有 provider 的 api_keys(读盘后调用)。裸值透传,兼容旧明文配置。
    fn decrypt_secrets(&mut self, store: &crate::secrets::SecretStore) {
        for entry in self.iter_provider_entries_mut() {
            for key in entry.api_keys.iter_mut() {
                if let Ok(dec) = store.decrypt(key) {
                    if !dec.is_empty() {
                        *key = dec;
                    }
                }
            }
        }
    }

    fn iter_provider_entries_mut(&mut self) -> impl Iterator<Item = &mut ProviderEntry> {
        self.providers.families.values_mut().flat_map(|m| m.values_mut())
    }
}
