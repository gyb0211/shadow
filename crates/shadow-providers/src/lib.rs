//! 影子 LLM provider 实现
//!
//! 架构 (借鉴 zeroclaw 3 层):
//! - **Router** (顶层): 按 alias 路由到具体 family provider
//! - **Compat** (中层): 把家族差异 (auth, API path, payload) 适配为统一 OpenAI 形态
//! - **Reliable** (底层, Phase 2): 重试/退避/key 轮换
//!
//! MVP 交付 Router + Compat, Reliable 延后.

pub mod openai;
pub mod router;

pub use openai::OpenAiCompat;
pub use router::Router;

use shadow_core::{ModelProvider, ModelProviderRuntimeOptions};
use anyhow::Result;
use std::sync::Arc;

/// 工厂函数 -- 按 (alias, family) 创建 provider
///
/// - `alias`: 完整别名 (如 "openai.default") -- 用于 Attributable::alias()
/// - `family`: 家族名 (如 "openai" / "openrouter" / "ollama" / "compatible") -- 决定 base_url 和 provider_type()
/// - `api_key`: API key (None 时不发 auth header, 兼容 ollama)
/// - `base_url`: 自定义 base_url (None 时按 family 选默认)
/// - `opts`: 运行时选项 (auth_style / timeout / extra_headers / ...)
///
/// 返回 `Arc<dyn ModelProvider>` 而非 Box, 因为 Agent.provider 字段类型是 Arc,
/// 维持 Arc 把改动量压到最小.
pub fn create_provider(
    alias: &str,
    family: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
    opts: ModelProviderRuntimeOptions,
) -> Result<Arc<dyn ModelProvider>> {
    match family {
        "openai" | "openrouter" | "ollama" | "compatible" => {
            OpenAiCompat::new(alias, family, api_key, base_url, opts)
                .map(|p| Arc::new(p) as Arc<dyn ModelProvider>)
        }
        _ => anyhow::bail!("未知的 provider family: {family}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_creates_provider_for_known_family() {
        let p = create_provider(
            "openai.default",
            "openai",
            Some("sk-test"),
            None,
            ModelProviderRuntimeOptions::default(),
        )
        .unwrap();
        assert_eq!(p.provider_type(), "openai");
    }

    #[test]
    fn factory_creates_for_compatible_family() {
        let p = create_provider(
            "custom.glm2",
            "compatible",
            None,
            Some("https://open.bigmodel.cn/api/paas/v4"),
            ModelProviderRuntimeOptions::default(),
        )
        .unwrap();
        assert_eq!(p.provider_type(), "compatible");
    }

    #[test]
    fn factory_unknown_family_errors() {
        let result = create_provider(
            "x.default",
            "nonexistent",
            None,
            None,
            ModelProviderRuntimeOptions::default(),
        );
        let err = match result {
            Ok(_) => panic!("expected error for unknown family"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("nonexistent"), "err = {err}");
    }
}
