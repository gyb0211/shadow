//! provider 模块集成测试 -- ProviderRef 解析、resolve_provider、list_providers。

use std::collections::HashMap;
use shadow_config::{
    default_base_url, list_providers, resolve_provider, ProviderEntry, ProviderRef, ResolvedProvider,
};

#[test]
fn parse_dotted_ref() {
    let r = ProviderRef::parse("openai.default");
    assert_eq!(r.family, "openai");
    assert_eq!(r.alias, "default");
}

#[test]
fn parse_bare_ref_defaults_to_default() {
    let r = ProviderRef::parse("openai");
    assert_eq!(r.family, "openai");
    assert_eq!(r.alias, "default");
}

#[test]
fn parse_custom_ref() {
    let r = ProviderRef::parse("custom.minimax1");
    assert_eq!(r.family, "custom");
    assert_eq!(r.alias, "minimax1");
}

#[test]
fn to_dotted_round_trip() {
    let r = ProviderRef::parse("custom.glm2");
    assert_eq!(r.to_dotted(), "custom.glm2");
}

#[test]
fn resolve_existing_provider() {
    let mut providers = HashMap::new();
    let mut openai = HashMap::new();
    openai.insert(
        "default".to_string(),
        ProviderEntry {
            api_key: Some("sk-xxx".to_string()),
            model: Some("gpt-4o-mini".to_string()),
            ..Default::default()
        },
    );
    providers.insert("openai".to_string(), openai);

    let resolved = resolve_provider(&providers, "openai.default").unwrap();
    assert_eq!(resolved.family, "openai");
    assert_eq!(resolved.alias, "default");
    assert_eq!(resolved.entry.api_key.as_deref(), Some("sk-xxx"));
}

#[test]
fn resolve_bare_name_uses_default_alias() {
    let mut providers = HashMap::new();
    let mut openai = HashMap::new();
    openai.insert(
        "default".to_string(),
        ProviderEntry {
            model: Some("gpt-4o-mini".to_string()),
            ..Default::default()
        },
    );
    providers.insert("openai".to_string(), openai);

    let resolved = resolve_provider(&providers, "openai").unwrap();
    assert_eq!(resolved.alias, "default");
}

#[test]
fn resolve_unknown_family_errors() {
    let providers = HashMap::new();
    let err = resolve_provider(&providers, "nonexistent.default").unwrap_err();
    assert!(err.to_string().contains("未知的 provider family"));
}

#[test]
fn resolve_unknown_alias_errors_with_suggestions() {
    let mut providers = HashMap::new();
    let mut custom = HashMap::new();
    custom.insert("minimax1".to_string(), ProviderEntry::default());
    providers.insert("custom".to_string(), custom);

    let err = resolve_provider(&providers, "custom.glm2").unwrap_err();
    assert!(err.to_string().contains("minimax1"));
}

#[test]
fn list_providers_sorted() {
    let mut providers = HashMap::new();
    let mut custom = HashMap::new();
    custom.insert("glm2".to_string(), ProviderEntry::default());
    custom.insert("minimax1".to_string(), ProviderEntry::default());
    providers.insert("custom".to_string(), custom);

    let mut openai = HashMap::new();
    openai.insert("default".to_string(), ProviderEntry::default());
    providers.insert("openai".to_string(), openai);

    let list = list_providers(&providers);
    assert_eq!(
        list,
        vec![
            ("custom".to_string(), "glm2".to_string()),
            ("custom".to_string(), "minimax1".to_string()),
            ("openai".to_string(), "default".to_string()),
        ]
    );
}

#[test]
fn default_base_url_for_known_families() {
    assert_eq!(
        default_base_url("openai"),
        Some("https://api.openai.com/v1")
    );
    assert_eq!(default_base_url("custom"), None);
}

#[test]
fn effective_model_uses_entry_then_fallback() {
    let resolved = ResolvedProvider {
        family: "openai".to_string(),
        alias: "default".to_string(),
        entry: ProviderEntry {
            model: Some("gpt-4o".to_string()),
            ..Default::default()
        },
    };
    assert_eq!(resolved.effective_model("fallback"), "gpt-4o");

    let resolved_no_model = ResolvedProvider {
        family: "openai".to_string(),
        alias: "default".to_string(),
        entry: ProviderEntry::default(),
    };
    assert_eq!(resolved_no_model.effective_model("gpt-4o-mini"), "gpt-4o-mini");
}
