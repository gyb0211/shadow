//! 工厂函数集成测试 -- create_reliable_provider 构造完整 Reliable 包装.
//!
//! 注意: 不实际调用 chat/list_models (会触发网络请求); 只验证构造与元数据.

use shadow_providers::{create_reliable_provider, RetryPolicy};
use shadow_core::{Attributable, Provider, Role};

#[test]
fn create_reliable_provider_returns_attributable() {
    let provider = create_reliable_provider(
        "openai.default",
        "openai",
        vec!["sk-test".to_string()],
        None,
        vec![],
        RetryPolicy::default(),
        0,
    )
    .expect("factory should succeed");

    // Attributable 接口正常
    assert_eq!(provider.role(), Role::Provider);
    assert_eq!(provider.alias(), "openai.default");
    assert_eq!(provider.provider_type(), "openai");
}

#[test]
fn create_reliable_provider_with_multi_key() {
    let provider = create_reliable_provider(
        "openai.default",
        "openai",
        vec!["sk-1".to_string(), "sk-2".to_string(), "sk-3".to_string()],
        None,
        vec!["gpt-4o-mini".to_string()],
        RetryPolicy {
            max_retries: 2,
            ..RetryPolicy::default()
        },
        60,
    )
    .expect("factory should succeed with multi-key + rate limit + fallback");

    assert_eq!(provider.alias(), "openai.default");
}

#[test]
fn create_reliable_provider_no_key_works_for_ollama() {
    // ollama 不需要 key, 空 vec 也应正常构造
    let provider = create_reliable_provider(
        "ollama.default",
        "ollama",
        vec![],
        None,
        vec![],
        RetryPolicy::default(),
        0,
    )
    .expect("factory should succeed without key");

    assert_eq!(provider.provider_type(), "ollama");
}
