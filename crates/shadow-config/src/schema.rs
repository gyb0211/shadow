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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: crate::migration::CURRENT_SCHEMA_VERSION,
            agent: AgentSection::default(),
            providers: ProvidersConfig::default(),
            memory: MemorySection::default(),
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
/// api_key = "sk-xxx"
/// model = "gpt-4o-mini"
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
pub(crate) fn load_from(dir: &std::path::Path) -> Result<Config> {
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
pub(crate) fn save_to(config: &Config, dir: &std::path::Path) -> Result<()> {
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
    /// 加密所有 provider 的 api_key(写盘前调用)。
    fn encrypt_secrets(&mut self, store: &crate::secrets::SecretStore) {
        for entry in self.iter_provider_entries_mut() {
            if let Some(k) = entry.api_key.take() {
                entry.api_key = store.encrypt(&k).ok().filter(|s| !s.is_empty());
            }
        }
    }

    /// 解密所有 provider 的 api_key(读盘后调用)。裸值透传,兼容旧明文配置。
    fn decrypt_secrets(&mut self, store: &crate::secrets::SecretStore) {
        for entry in self.iter_provider_entries_mut() {
            if let Some(k) = entry.api_key.take() {
                entry.api_key = store.decrypt(&k).ok().filter(|s| !s.is_empty());
            }
        }
    }

    fn iter_provider_entries_mut(&mut self) -> impl Iterator<Item = &mut ProviderEntry> {
        self.providers.families.values_mut().flat_map(|m| m.values_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── load/save + 加密集成 ──

    #[test]
    fn save_writes_encrypted_api_key_to_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut config = Config::default();
        config
            .providers
            .find_or_create("openai", "default")
            .api_key = Some("sk-plaintext".into());
        save_to(&config, dir).unwrap();
        let raw = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(
            raw.contains("enc2:"),
            "api_key must be encrypted on disk, got: {raw}"
        );
        assert!(
            !raw.contains("sk-plaintext"),
            "plaintext must NOT reach disk"
        );
    }

    #[test]
    fn save_then_load_preserves_api_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut config = Config::default();
        config
            .providers
            .find_or_create("openai", "default")
            .api_key = Some("sk-roundtrip".into());
        save_to(&config, dir).unwrap();
        let loaded = load_from(dir).unwrap();
        assert_eq!(
            loaded.providers.find("openai", "default").unwrap().api_key.as_deref(),
            Some("sk-roundtrip")
        );
    }

    #[test]
    fn load_decrypts_encrypted_api_key() {
        // 先在一个 dir 里 save 一个带 api_key 的配置,拿到 enc2: 形态,
        // 再清空内存重新 load,验证 decrypt 生效。
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut config = Config::default();
        config
            .providers
            .find_or_create("anthropic", "default")
            .api_key = Some("sk-ant-secret".into());
        save_to(&config, dir).unwrap();
        // 磁盘上 api_key 行必须是加密形态(值带 enc2: 前缀)
        let raw = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let api_key_line = raw
            .lines()
            .find(|l| l.contains("api_key"))
            .expect("config must have an api_key line");
        assert!(
            api_key_line.contains("enc2:"),
            "api_key on disk must be encrypted, got: {api_key_line}"
        );
        // 重新 load 应解密回明文
        let loaded = load_from(dir).unwrap();
        assert_eq!(
            loaded.providers.find("anthropic", "default").unwrap().api_key.as_deref(),
            Some("sk-ant-secret")
        );
    }

    #[test]
    fn load_migrates_unversioned_config() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // 手写一个没有 schema_version 的旧配置
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("config.toml"),
            r#"
[agent]
alias = "default"
"#,
        )
        .unwrap();
        let loaded = load_from(dir).unwrap();
        assert_eq!(
            loaded.schema_version,
            crate::migration::CURRENT_SCHEMA_VERSION
        );
    }

    #[test]
    fn save_load_empty_api_key_stays_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut config = Config::default();
        config.providers.find_or_create("openai", "default").api_key = None;
        save_to(&config, dir).unwrap();
        let loaded = load_from(dir).unwrap();
        assert!(loaded.providers.find("openai", "default").unwrap().api_key.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_config_file_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let config = Config::default();
        save_to(&config, dir).unwrap();
        let mode = std::fs::metadata(dir.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    // ── 原有单元测试 ──

    #[test]
    fn default_config_serializes_to_toml() {
        let config = Config::default();
        let toml = toml::to_string_pretty(&config).unwrap();
        assert!(toml.contains("[agent]"));
        assert!(toml.contains("model_provider = \"openai.default\""));
    }

    #[test]
    fn multi_provider_round_trip() {
        let mut config = Config::default();

        // 添加 openai.default
        let openai_entry = config.providers.find_or_create("openai", "default");
        openai_entry.api_key = Some("sk-xxx".to_string());
        openai_entry.model = Some("gpt-4o-mini".to_string());

        // 添加 custom.minimax1
        let minimax_entry = config.providers.find_or_create("custom", "minimax1");
        minimax_entry.api_key = Some("minimax-key".to_string());
        minimax_entry.base_url = Some("https://api.minimax.chat/v1".to_string());
        minimax_entry.model = Some("abab6.5s-chat".to_string());

        // 添加 custom.glm2
        let glm_entry = config.providers.find_or_create("custom", "glm2");
        glm_entry.api_key = Some("glm-key".to_string());
        glm_entry.base_url = Some("https://open.bigmodel.cn/api/paas/v4".to_string());
        glm_entry.model = Some("glm-4-flash".to_string());

        // 序列化
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("[providers.openai.default]"));
        assert!(toml_str.contains("[providers.custom.minimax1]"));
        assert!(toml_str.contains("[providers.custom.glm2]"));

        // 反序列化
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            parsed.providers.find("openai", "default").unwrap().api_key,
            Some("sk-xxx".to_string())
        );
        assert_eq!(
            parsed
                .providers
                .find("custom", "minimax1")
                .unwrap()
                .model,
            Some("abab6.5s-chat".to_string())
        );
        assert_eq!(
            parsed
                .providers
                .find("custom", "glm2")
                .unwrap()
                .base_url,
            Some("https://open.bigmodel.cn/api/paas/v4".to_string())
        );
    }

    #[test]
    fn providers_list_sorted() {
        let mut config = Config::default();
        config.providers.find_or_create("custom", "glm2");
        config.providers.find_or_create("custom", "minimax1");
        config.providers.find_or_create("openai", "default");

        let list = config.providers.list();
        assert_eq!(list, vec![
            ("custom".to_string(), "glm2".to_string()),
            ("custom".to_string(), "minimax1".to_string()),
            ("openai".to_string(), "default".to_string()),
        ]);
    }

    #[test]
    fn default_config_has_current_schema_version() {
        let config = Config::default();
        assert_eq!(config.schema_version, crate::migration::CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn schema_version_round_trips_through_toml() {
        let config = Config {
            schema_version: 7,
            ..Config::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("schema_version = 7"));
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.schema_version, 7);
    }

    #[test]
    fn toml_with_multiple_providers() {
        let toml_str = r#"
[agent]
alias = "default"
model_provider = "custom.minimax1"
model = "abab6.5s-chat"
temperature = 0.7
autonomy = "supervised"

[providers.openai.default]
api_key = "sk-xxx"
model = "gpt-4o-mini"

[providers.custom.minimax1]
api_key = "minimax-key"
base_url = "https://api.minimax.chat/v1"
model = "abab6.5s-chat"

[providers.custom.glm2]
api_key = "glm-key"
base_url = "https://open.bigmodel.cn/api/paas/v4"
model = "glm-4-flash"

[memory]
backend = "none"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.agent.model_provider, "custom.minimax1");
        assert_eq!(config.providers.list().len(), 3);
    }
}
