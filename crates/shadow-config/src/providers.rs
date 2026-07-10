//! Provider 配置条目与解析

use crate::model_provider::{
    ModelProviderConfig, CustomModelProviderConfig
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[macro_export]
macro_rules! define_provider_ref {
    ($name:ident, $category_doc: literal) => {
        #[doc = concat!("Reference to a configured `[",$category_doc,".<type>.<alias>] entry.`")]
        #[derive(
            Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(v: String) -> Self {
                Self(v)
            }
        }

        impl From<&str> for $name {
            fn from(v: &str) -> Self {
                Self(v.to_string())
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> Self {
                v.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                &self.0 == other
            }
        }
    };
}

define_provider_ref!(ModelProviderRef, "providers.models");
define_provider_ref!(RiskProfileRef, "risk_profiles");
define_provider_ref!(RuntimeProfileRef, "runtime_profiles");

#[macro_export]
macro_rules! for_each_model_provider_slot {
    ($mac: ident) => {
        $mac! {
            (custom, "custom", CustomModelProviderConfig),
        }
    };
}
macro_rules! emit_model_provider_struct {
    // field + field_str + config
    ($(($field: ident,$type_str: literal, $cfg_ty: ty)) + $(,)?) => {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct ModelProviders {
            #[serde(default, skip_serializing_if="HashMap::is_empty")]
            $(pub $field: HashMap<String, $cfg_ty>,)+
        }
    };
}

for_each_model_provider_slot!(emit_model_provider_struct);

impl ModelProviders {
    pub fn find(&self, family: &str, alias: &str) -> Option<&ModelProviderConfig> {
        macro_rules! emit_get {
            // field + field_str + config
            ($(($field: ident,$type_str: literal, $cfg_ty: ty)) + $(,)?) => {
                match family {
                    $($type_str => self.$field.get(alias).map(|cfg|&cfg.base), )+
                    _ => None,
                }
            };
        }
        for_each_model_provider_slot!(emit_get)
    }

    pub fn iter_entries(&self) ->impl Iterator<Item = (&str, &str, &ModelProviderConfig)>{
        let mut out:Vec<(&str, &str, &ModelProviderConfig)> = Vec::new();
        macro_rules! emit_iter {
            // field + field_str + config
            ($(($field: ident,$type_str: literal, $cfg_ty: ty)) + $(,)?) => {
                $(
                for (alias, cfg) in &self.$field {
                    out.push(($type_str, alias.as_str() , &cfg.base))
                }
                )+
            };
        }
        for_each_model_provider_slot!(emit_iter);
        out.into_iter()
    }

    pub fn ensure(&mut self, family: &str, alias: &str) -> Option<&mut ModelProviderConfig> {
        macro_rules! emit_ensure {
            // field + field_str + config
            ($(($field: ident,$type_str: literal, $cfg_ty: ty)) + $(,)?) => {
                match family {
                    $(
                     $type_str =>
                         Some(&mut self.$field.entry(alias.to_string()).or_default().base),
                    )+
                    _ => None,
                }
            };
        }
        for_each_model_provider_slot!(emit_ensure)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Providers {
    pub models: ModelProviders,
    // pub tts: TtsProviders,
    // pub transcription: TranscriptionProviders,
}

/// 单个 provider 配置条目
///
/// 每个 `[providers.<family>.<alias>]` 块反序列化为此结构。
/// 不同 family (openai/anthropic/custom...) 共享同一结构,
/// 通过 `base_url` 区分端点。
///
/// 序列化总是输出 `api_keys = [...]` 形态。反序列化接受:
/// ```toml
/// api_key = "sk-xxx"            # 单 key (向后兼容)
/// api_keys = ["sk-1", "sk-2"]   # 多 key
/// ```
/// 两者都存在时合并去重 (api_key 优先)。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProviderEntry {
    /// API 密钥列表 (支持 key 轮换 / fallback)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<String>,

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

    /// Reliable 层配置 (重试 / 退避 / 限流)
    #[serde(default, skip_serializing_if = "ReliableConfig::is_default")]
    pub reliable: ReliableConfig,
}

impl ProviderEntry {
    /// 取第一个 key (单 key 场景的便捷访问)
    #[must_use]
    pub fn first_key(&self) -> Option<&str> {
        self.api_keys.first().map(String::as_str)
    }

    /// 是否有 key
    #[must_use]
    pub fn has_key(&self) -> bool {
        !self.api_keys.is_empty()
    }
}

/// 手动 Deserialize -- 接受 `api_key` (单值) 或 `api_keys` (数组),
/// 合并去重后存入 `ProviderEntry::api_keys`.
impl<'de> Deserialize<'de> for ProviderEntry {
    fn deserialize<D>(de: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// 内部辅助结构 -- 字段级使用 derive 简化代码
        #[derive(Deserialize, Default)]
        struct Helper {
            #[serde(default)]
            api_key: Option<String>,
            #[serde(default)]
            api_keys: Option<Vec<String>>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default)]
            base_url: Option<String>,
            #[serde(default)]
            temperature: Option<f64>,
            #[serde(default)]
            max_tokens: Option<u32>,
            #[serde(default)]
            timeout_secs: Option<u64>,
            #[serde(default)]
            fallback_models: Vec<String>,
            #[serde(default)]
            reliable: ReliableConfig,
        }
        let h = Helper::deserialize(de)?;
        // 合并 api_key 和 api_keys -> api_keys (api_key 在前, 去重)
        let mut api_keys = Vec::new();
        if let Some(k) = h.api_key {
            api_keys.push(k);
        }
        if let Some(ks) = h.api_keys {
            for k in ks {
                if !api_keys.contains(&k) {
                    api_keys.push(k);
                }
            }
        }
        Ok(ProviderEntry {
            api_keys,
            model: h.model,
            base_url: h.base_url,
            temperature: h.temperature,
            max_tokens: h.max_tokens,
            timeout_secs: h.timeout_secs,
            fallback_models: h.fallback_models,
            reliable: h.reliable,
        })
    }
}

/// Reliable 层配置 -- 控制重试 / 退避 / 限流行为
///
/// ```toml
/// [providers.openai.default.reliable]
/// max_retries = 3
/// initial_backoff_ms = 1000
/// max_backoff_ms = 60000
/// jitter_pct = 25
/// requests_per_minute = 0   # 0 = 无限流
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReliableConfig {
    /// 最大重试次数 (0 = 不重试, 只调一次)
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// 初始退避 (毫秒)
    #[serde(default = "default_initial_backoff_ms")]
    pub initial_backoff_ms: u64,

    /// 退避上限 (毫秒)
    #[serde(default = "default_max_backoff_ms")]
    pub max_backoff_ms: u64,

    /// Jitter 百分比 (0-100), 实际退避 = base * (1 ± jitter_pct/100)
    #[serde(default = "default_jitter_pct")]
    pub jitter_pct: u8,

    /// 每分钟最大请求数 (0 = 无限流)
    #[serde(default)]
    pub requests_per_minute: u32,
}

fn default_max_retries() -> u32 {
    3
}
fn default_initial_backoff_ms() -> u64 {
    1000
}
fn default_max_backoff_ms() -> u64 {
    60_000
}
fn default_jitter_pct() -> u8 {
    25
}

impl Default for ReliableConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_backoff_ms: default_initial_backoff_ms(),
            max_backoff_ms: default_max_backoff_ms(),
            jitter_pct: default_jitter_pct(),
            requests_per_minute: 0,
        }
    }
}

impl ReliableConfig {
    /// 是否全字段等于默认值 (用于 skip_serializing_if)
    #[must_use]
    pub fn is_default(&self) -> bool {
        let d = Self::default();
        self.max_retries == d.max_retries
            && self.initial_backoff_ms == d.initial_backoff_ms
            && self.max_backoff_ms == d.max_backoff_ms
            && self.jitter_pct == d.jitter_pct
            && self.requests_per_minute == d.requests_per_minute
    }
}

/// Router 配置段 -- 跨 provider 路由与 fallback
///
/// ```toml
/// [router]
/// default = "openai.default"
///
/// [router.routes.reasoning]
/// provider = "anthropic.claude"
/// model = "claude-sonnet-4-20250514"
///
/// [router.fallback_chains]
/// default = ["anthropic.claude", "openai.default"]
/// reasoning = ["openai.default"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterConfig {
    /// 默认 provider 引用 -- "family.alias" 格式
    #[serde(default)]
    pub default: String,

    /// hint → 路由规则
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub routes: HashMap<String, RouteEntry>,

    /// hint (或 "default") → 备选 provider 引用列表
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fallback_chains: HashMap<String, Vec<String>>,
}

/// 单条路由规则 -- hint → (provider, model)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteEntry {
    /// provider 引用 -- "family.alias" 格式
    pub provider: String,
    /// 实际下发的 model 名
    pub model: String,
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
                family_map.keys().cloned().collect::<Vec<_>>().join(", ")
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
