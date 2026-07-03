//! 影子 LLM provider 实现
//!
//! 架构 (借鉴 zeroclaw 3 层):
//! - **Router** (顶层): 按 alias 路由到具体 family provider
//! - **Reliable** (中层): 重试/退避/key 轮换/限流
//! - **Compat** (底层): 把家族差异 (auth, API path, payload) 适配为统一 OpenAI 形态

pub mod dispatch;
pub mod error;
pub mod openai;
pub mod rate_limit;
pub mod reliable;
pub mod router;

pub use error::{ChatError, RetryClass};
pub use openai::OpenAiProvider;
pub use rate_limit::TokenBucket;
pub use reliable::{KeyRotator, ReliableModelProvider, RetryPolicy};
pub use router::RouterModelProvider;

use shadow_core::{ModelProviderRuntimeOptions, Provider};
use anyhow::Result;
use std::sync::Arc;

/// 工厂函数 -- 按 (family) 创建 provider (向后兼容旧签名)
///
/// 等价于 `create_provider_with_opts(family, api_key, base_url, ModelProviderRuntimeOptions::default())`.
pub fn create_provider(
    provider_type: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Arc<dyn Provider>> {
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
/// 返回 `Arc<dyn Provider>`, 因为 Agent.provider 字段类型是 Arc.
pub fn create_provider_with_opts(
    alias: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
    opts: ModelProviderRuntimeOptions,
) -> Result<Arc<dyn Provider>> {
    match alias {
        "openai" | "openrouter" | "ollama" | "compatible" => {
            OpenAiProvider::new_with_opts(alias, api_key, base_url, opts)
                .map(|p| Arc::new(p) as Arc<dyn Provider>)
        }
        _ => anyhow::bail!("未知的 provider family: {alias}"),
    }
}
