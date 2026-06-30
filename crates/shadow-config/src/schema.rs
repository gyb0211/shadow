//! 配置 schema -- 顶层 Config + 各配置段

use crate::provider::ProviderEntry;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 顶层配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
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

impl Default for AgentSection {
    fn default() -> Self {
        Self {
            alias: "default".to_string(),
            model_provider: default_model_provider(),
            model: default_model(),
            temperature: Some(0.7),
            autonomy: default_autonomy(),
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

/// 加载配置 -- 不存在则创建默认
pub fn load_or_init() -> Result<Config> {
    let path = config_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&content).unwrap_or_default();
        Ok(config)
    } else {
        let config = Config::default();
        save(&config)?;
        Ok(config)
    }
}

/// 保存配置
pub fn save(config: &Config) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let path = config_path();
    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
