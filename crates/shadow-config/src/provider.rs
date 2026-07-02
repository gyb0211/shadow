//! Provider 配置条目与解析

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 单个 provider 配置条目
///
/// 每个 `[providers.<family>.<alias>]` 块反序列化为此结构。
/// 不同 family (openai/anthropic/custom...) 共享同一结构,
/// 通过 `base_url` 区分端点。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderEntry {
    /// API 密钥
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// 默认模型 ID (如 "gpt-4o-mini", "claude-sonnet-4-20250514")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// API 端点 URL (不设则用 family 默认)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// 采样温度 (0.0-2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// 最大响应 token 数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// 请求超时 (秒)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,

    /// 备选模型列表 (主模型失败时依次尝试)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_models: Vec<String>,
}

/// Provider 引用 -- "family.alias" 格式
///
/// agent.model_provider = "openai.default" 或 "custom.minimax1"
/// 简写 "openai" 等价于 "openai.default"
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRef {
    pub family: String,
    pub alias: String,
}

impl ProviderRef {
    /// 从 "family.alias" 字符串解析
    ///
    /// "openai.default" -> ProviderRef { family: "openai", alias: "default" }
    /// "openai"         -> ProviderRef { family: "openai", alias: "default" }
    /// "custom.minimax1"-> ProviderRef { family: "custom", alias: "minimax1" }
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.split_once('.') {
            Some((family, alias)) => Self {
                family: family.to_string(),
                alias: alias.to_string(),
            },
            None => Self {
                family: s.to_string(),
                alias: "default".to_string(),
            },
        }
    }

    /// 转回 "family.alias" 字符串
    #[must_use]
    pub fn to_dotted(&self) -> String {
        format!("{}.{}", self.family, self.alias)
    }
}

/// 解析后的 provider -- 从配置中查找到的完整信息
#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    /// family 类型 (openai/anthropic/ollama/custom...)
    pub family: String,
    /// 别名 (default/minimax1/glm2...)
    pub alias: String,
    /// 配置条目
    pub entry: ProviderEntry,
}

impl ResolvedProvider {
    /// 获取实际使用的模型 -- 条目配置 > family 默认
    #[must_use]
    pub fn effective_model<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.entry.model.as_deref().unwrap_or(fallback)
    }

    /// 获取实际使用的 base_url -- 条目配置 > family 默认
    #[must_use]
    pub fn effective_base_url(&self) -> Option<&str> {
        self.entry.base_url.as_deref()
    }

    /// 获取实际使用的温度 -- 条目配置 > 默认 0.7
    #[must_use]
    pub fn effective_temperature(&self) -> f64 {
        self.entry.temperature.unwrap_or(0.7)
    }
}

/// 各 family 的默认 base_url
///
/// 当 ProviderEntry.base_url 为 None 时使用
#[must_use]
pub fn default_base_url(family: &str) -> Option<&'static str> {
    match family {
        "openai" => Some("https://api.openai.com/v1"),
        "anthropic" => Some("https://api.anthropic.com"),
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "ollama" => Some("http://localhost:11434/v1"),
        "deepseek" => Some("https://api.deepseek.com"),
        "moonshot" => Some("https://api.moonshot.cn/v1"),
        "qwen" => Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        "minimax" => Some("https://api.minimax.chat/v1"),
        "glm" | "zhipu" => Some("https://open.bigmodel.cn/api/paas/v4"),
        "doubao" => Some("https://ark.cn-beijing.volces.com/api/v3"),
        // custom 系列: 必须在 entry 中显式配置 base_url
        _ => None,
    }
}

/// 从 providers 配置中解析 provider 引用
///
/// 按 "family.alias" 查找, 不存在则返回 Err
pub fn resolve_provider(
    providers: &HashMap<String, HashMap<String, ProviderEntry>>,
    reference: &str,
) -> Result<ResolvedProvider> {
    let pref = ProviderRef::parse(reference);
    let family_map = providers
        .get(&pref.family)
        .ok_or_else(|| anyhow::anyhow!("未知的 provider family: '{}'", pref.family))?;
    let entry = family_map
        .get(&pref.alias)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "provider '{}' 在 family '{}' 中不存在 (可用别名: {})",
                pref.alias,
                pref.family,
                family_map
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?
        .clone();

    Ok(ResolvedProvider {
        family: pref.family,
        alias: pref.alias,
        entry,
    })
}

/// 列出所有已配置的 provider (family, alias) 对
pub fn list_providers(
    providers: &HashMap<String, HashMap<String, ProviderEntry>>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (family, aliases) in providers {
        for alias in aliases.keys() {
            out.push((family.clone(), alias.clone()));
        }
    }
    out.sort();
    out
}

