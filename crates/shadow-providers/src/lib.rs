//! 影子 LLM provider 实现
//!
//! 架构 (借鉴 zeroclaw 3 层):
//! - **Router** (顶层): 按 alias 路由到具体 family provider
//! - **Reliable** (中层): 重试/退避/key 轮换/限流
//! - **Compat** (底层): 把家族差异 (auth, API path, payload) 适配为统一 OpenAI 形态

pub mod anthropic;
pub mod dispatch;
pub mod error;
pub mod openai;
pub mod rate_limit;
pub mod reliable;
pub mod router;

pub use anthropic::AnthropicProvider;
pub use error::{ChatError, RetryClass};
pub use openai::OpenAiProvider;
pub use rate_limit::TokenBucket;
pub use reliable::{KeyRotator, ReliableModelProvider, RetryPolicy};
pub use router::RouterModelProvider;

use shadow_core::{ModelProvider, ModelProviderRuntimeOptions};
use anyhow::Result;
use std::sync::Arc;

/// 工厂函数 -- 按 (family) 创建 provider (向后兼容旧签名)
///
/// 等价于 `create_provider_with_opts(family, api_key, base_url, ModelProviderRuntimeOptions::default())`.
pub fn create_provider(
    provider_type: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Arc<dyn ModelProvider>> {
    create_provider_with_opts(
        provider_type,
        api_key,
        base_url,
        ModelProviderRuntimeOptions::default(),
    )
}

/// 工厂函数 (带运行时选项) -- 按 (alias, family) 创建 provider
///
/// - `alias`: 别名 (如 "openai.default") -- 用于 Attributable::alias(); 传入 family 时回退为 family
/// - `family`: 家族名 (如 "openai" / "openrouter" / "ollama" / "compatible") -- 决定 base_url
/// - `api_key`: API key (None 时不发 auth header, 兼容 ollama)
/// - `base_url`: 自定义 base_url (None 时按 family 选默认)
/// - `opts`: 运行时选项 (auth_style / timeout / extra_headers / ...)
///
/// 返回 `Arc<dyn ModelProvider>`, 因为 Agent.provider 字段类型是 Arc.
pub fn create_provider_with_opts(
    alias: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
    opts: ModelProviderRuntimeOptions,
) -> Result<Arc<dyn ModelProvider>> {
    match alias {
        "openai" | "openrouter" | "ollama" | "compatible" => {
            OpenAiProvider::new_with_opts(alias, api_key, base_url, opts)
                .map(|p| Arc::new(p) as Arc<dyn ModelProvider>)
        }
        "anthropic" => AnthropicProvider::new_with_alias(alias, api_key, base_url, opts)
            .map(|p| Arc::new(p) as Arc<dyn ModelProvider>),
        _ => anyhow::bail!("未知的 provider family: {alias}"),
    }
}

/// 创建 Reliable 包装的 provider -- 从 ResolvedProvider 字段构造完整 3 层栈.
///
/// 参数从 `ProviderEntry` 提取 (调用方负责), 避免本 crate 反向依赖 shadow-config.
///
/// - `alias`: 完整别名 (如 "openai.default") -- 用于 Attributable::alias()
/// - `family`: 家族名 (如 "openai" / "openrouter" / "ollama" / "compatible")
/// - `api_keys`: API key 列表 (多 key 支持轮换; 空 vec 表示无 auth)
/// - `base_url`: 自定义 base_url (None 时按 family 选默认)
/// - `fallback_models`: 主模型失败后依次尝试的备选模型列表
/// - `policy`: 重试/退避策略 (max_retries / initial_backoff_ms / max_backoff_ms / jitter_pct)
/// - `requests_per_minute`: 限流 (0 = 无限流)
///
/// 返回 `Arc<dyn ModelProvider>` -- 内部已 Reliable 包装.
pub fn create_reliable_provider(
    alias: &str,
    family: &str,
    api_keys: Vec<String>,
    base_url: Option<&str>,
    fallback_models: Vec<String>,
    policy: RetryPolicy,
    requests_per_minute: u32,
) -> Result<Arc<dyn ModelProvider>> {
    // 1. 构造 Compat 层 provider -- 按 family 选择具体实现
    //    返回 (dyn Provider, dyn KeyRotator) 双重 Arc, 共享同一底层对象
    let (inner_provider, rotator): (Arc<dyn ModelProvider>, Arc<dyn KeyRotator>) = match family {
        "anthropic" => {
            let p = Arc::new(AnthropicProvider::new_with_alias(
                alias,
                api_keys.first().map(String::as_str),
                base_url,
                ModelProviderRuntimeOptions::default(),
            )?);
            let r = Arc::clone(&p) as Arc<dyn KeyRotator>;
            (p, r)
        }
        // OpenAI 兼容家族 (openai/openrouter/ollama/compatible/其他)
        _ => {
            let p = Arc::new(OpenAiProvider::new_with_opts(
                family,
                api_keys.first().map(String::as_str),
                base_url,
                ModelProviderRuntimeOptions::default(),
            )?);
            let r = Arc::clone(&p) as Arc<dyn KeyRotator>;
            (p, r)
        }
    };

    // 2. 构造 Reliable 包装层, 注入 key 轮换 / 限流 / fallback
    let mut reliable = ReliableModelProvider::new(alias, inner_provider, policy);
    if !api_keys.is_empty() {
        reliable = reliable.with_key_rotation(api_keys, rotator);
    }
    if requests_per_minute > 0 {
        reliable = reliable.with_rate_limiter(Arc::new(TokenBucket::new(requests_per_minute)));
    }
    if !fallback_models.is_empty() {
        reliable = reliable.with_fallback_models(fallback_models);
    }
    Ok(Arc::new(reliable) as Arc<dyn ModelProvider>)
}
