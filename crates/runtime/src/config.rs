//! 配置 -- TOML 读写 + 密钥管理
//!
//! 借鉴 ZeroClaw 的 Config 设计, 但大幅精简:
//! - ZeroClaw: 61,918 行, 36 文件
//! - Shadow: 目标 ~200 行, 单文件

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 顶层配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub agent: AgentSection,
    pub provider: ProviderSection,
    pub memory: MemorySection,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agent: AgentSection::default(),
            provider: ProviderSection::default(),
            memory: MemorySection::default(),
        }
    }
}

/// [agent] 配置段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSection {
    pub alias: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    pub autonomy: String,
}

impl Default for AgentSection {
    fn default() -> Self {
        Self {
            alias: "default".to_string(),
            model: "gpt-4o-mini".to_string(),
            temperature: Some(0.7),
            autonomy: "supervised".to_string(),
        }
    }
}

/// [provider] 配置段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSection {
    #[serde(rename = "type")]
    pub provider_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl Default for ProviderSection {
    fn default() -> Self {
        Self {
            provider_type: "openai".to_string(),
            api_key: None,
            base_url: None,
        }
    }
}

/// [memory] 配置段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySection {
    pub backend: String,
}

impl Default for MemorySection {
    fn default() -> Self {
        Self {
            backend: "none".to_string(),
        }
    }
}

/// 配置目录 -- ~/.shadow/
#[must_use]
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SHADOW_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(home) = dirs_home() {
        return home.join(".shadow");
    }
    PathBuf::from(".shadow")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
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
