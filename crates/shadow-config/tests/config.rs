//! Config schema + load/save 集成测试 -- 加密落盘、原子写、迁移、schema_version。

use shadow_config::{load_from, save_to, Config, CURRENT_SCHEMA_VERSION};

// ── load/save + 加密集成 ──

#[test]
fn save_writes_encrypted_api_key_to_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut config = Config::default();
    config
        .providers
        .find_or_create("openai", "default")
        .api_keys = vec!["sk-plaintext".into()];
    save_to(&config, dir).unwrap();
    let raw = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(
        raw.contains("enc2:"),
        "api_keys must be encrypted on disk, got: {raw}"
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
        .api_keys = vec!["sk-roundtrip".into()];
    save_to(&config, dir).unwrap();
    let loaded = load_from(dir).unwrap();
    assert_eq!(
        loaded
            .providers
            .find("openai", "default")
            .unwrap()
            .api_keys
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["sk-roundtrip"]
    );
}

#[test]
fn load_decrypts_encrypted_api_key() {
    // 先在一个 dir 里 save 一个带 api_keys 的配置,拿到 enc2: 形态,
    // 再清空内存重新 load,验证 decrypt 生效。
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut config = Config::default();
    config
        .providers
        .find_or_create("anthropic", "default")
        .api_keys = vec!["sk-ant-secret".into()];
    save_to(&config, dir).unwrap();
    // 磁盘上 api_keys 行必须是加密形态(值带 enc2: 前缀)
    let raw = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    let api_keys_line = raw
        .lines()
        .find(|l| l.contains("api_keys"))
        .expect("config must have an api_keys line");
    assert!(
        api_keys_line.contains("enc2:"),
        "api_keys on disk must be encrypted, got: {api_keys_line}"
    );
    // 重新 load 应解密回明文
    let loaded = load_from(dir).unwrap();
    assert_eq!(
        loaded
            .providers
            .find("anthropic", "default")
            .unwrap()
            .api_keys
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["sk-ant-secret"]
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
    assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
}

#[test]
fn save_load_empty_api_keys_stays_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut config = Config::default();
    config.providers.find_or_create("openai", "default").api_keys.clear();
    save_to(&config, dir).unwrap();
    let loaded = load_from(dir).unwrap();
    assert!(loaded
        .providers
        .find("openai", "default")
        .unwrap()
        .api_keys
        .is_empty());
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

// ── Config schema 单元测试 ──

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
    openai_entry.api_keys = vec!["sk-xxx".to_string()];
    openai_entry.model = Some("gpt-4o-mini".to_string());

    // 添加 custom.minimax1
    let minimax_entry = config.providers.find_or_create("custom", "minimax1");
    minimax_entry.api_keys = vec!["minimax-key".to_string()];
    minimax_entry.base_url = Some("https://api.minimax.chat/v1".to_string());
    minimax_entry.model = Some("abab6.5s-chat".to_string());

    // 添加 custom.glm2
    let glm_entry = config.providers.find_or_create("custom", "glm2");
    glm_entry.api_keys = vec!["glm-key".to_string()];
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
        parsed.providers.find("openai", "default").unwrap().api_keys,
        vec!["sk-xxx".to_string()]
    );
    assert_eq!(
        parsed.providers.find("custom", "minimax1").unwrap().model,
        Some("abab6.5s-chat".to_string())
    );
    assert_eq!(
        parsed.providers.find("custom", "glm2").unwrap().base_url,
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
fn default_config_has_current_schema_version() {
    let config = Config::default();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
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
api_keys = ["minimax-key"]
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
    // 单值 api_key 和列表 api_keys 都能解析
    assert_eq!(
        config.providers.find("openai", "default").unwrap().api_keys,
        vec!["sk-xxx".to_string()]
    );
    assert_eq!(
        config.providers.find("custom", "minimax1").unwrap().api_keys,
        vec!["minimax-key".to_string()]
    );
}

// ── Phase 2: 多 key + ReliableConfig ──

#[test]
fn api_keys_accepts_string_or_array() {
    // 单值 api_key 形态
    let toml_str = r#"
[providers.openai.default]
api_key = "sk-single"
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(
        config.providers.find("openai", "default").unwrap().api_keys,
        vec!["sk-single".to_string()]
    );

    // 数组 api_keys 形态
    let toml_str = r#"
[providers.openai.default]
api_keys = ["sk-1", "sk-2"]
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(
        config.providers.find("openai", "default").unwrap().api_keys,
        vec!["sk-1".to_string(), "sk-2".to_string()]
    );
}

#[test]
fn api_keys_serializes_as_array() {
    let mut config = Config::default();
    config.providers.find_or_create("openai", "default").api_keys = vec![
        "sk-1".to_string(),
        "sk-2".to_string(),
    ];
    let toml_str = toml::to_string_pretty(&config).unwrap();
    // 序列化为数组形态
    assert!(toml_str.contains("api_keys = ["));
    assert!(toml_str.contains("\"sk-1\""));
    assert!(toml_str.contains("\"sk-2\""));
}

#[test]
fn reliable_config_defaults() {
    use shadow_config::providers::ReliableConfig;
    let rc = ReliableConfig::default();
    assert_eq!(rc.max_retries, 3);
    assert_eq!(rc.initial_backoff_ms, 1000);
    assert_eq!(rc.max_backoff_ms, 60_000);
    assert_eq!(rc.jitter_pct, 25);
    assert_eq!(rc.requests_per_minute, 0);
}

#[test]
fn reliable_config_round_trip() {
    let toml_str = r#"
[providers.openai.default]
api_keys = ["sk-x"]

[providers.openai.default.reliable]
max_retries = 5
initial_backoff_ms = 500
max_backoff_ms = 30000
jitter_pct = 30
requests_per_minute = 60
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let entry = config.providers.find("openai", "default").unwrap();
    assert_eq!(entry.reliable.max_retries, 5);
    assert_eq!(entry.reliable.initial_backoff_ms, 500);
    assert_eq!(entry.reliable.max_backoff_ms, 30000);
    assert_eq!(entry.reliable.jitter_pct, 30);
    assert_eq!(entry.reliable.requests_per_minute, 60);
}

#[test]
fn reliable_config_omitted_uses_defaults() {
    let toml_str = r#"
[providers.openai.default]
api_keys = ["sk-x"]
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let entry = config.providers.find("openai", "default").unwrap();
    // 不写 [reliable] 段时用默认值
    assert_eq!(entry.reliable.max_retries, 3);
}

#[test]
fn reliable_default_section_not_serialized() {
    // 全默认的 reliable 段不应出现在 toml 输出里
    let mut config = Config::default();
    let entry = config.providers.find_or_create("openai", "default");
    entry.api_keys = vec!["sk-x".to_string()];
    // reliable 用默认值
    let toml_str = toml::to_string_pretty(&config).unwrap();
    assert!(
        !toml_str.contains("[providers.openai.default.reliable]"),
        "默认 reliable 段不应序列化, got: {toml_str}"
    );
}

// ── Phase 4: RouterConfig ──

#[test]
fn router_section_parsing() {
    let toml_str = r#"
[router]
default = "openai.default"

[router.routes.reasoning]
provider = "anthropic.claude"
model = "claude-sonnet-4-20250514"

[router.fallback_chains]
default = ["anthropic.claude", "custom.glm"]
reasoning = ["openai.default"]
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let router = config.router.expect("router section should parse");
    assert_eq!(router.default, "openai.default");

    let reasoning_route = router.routes.get("reasoning").expect("reasoning route");
    assert_eq!(reasoning_route.provider, "anthropic.claude");
    assert_eq!(reasoning_route.model, "claude-sonnet-4-20250514");

    let default_chain = router.fallback_chains.get("default").expect("default chain");
    assert_eq!(*default_chain, vec!["anthropic.claude".to_string(), "custom.glm".to_string()]);
}

#[test]
fn router_section_optional_when_omitted() {
    let toml_str = r#"
[agent]
alias = "default"
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert!(config.router.is_none(), "router should be None when omitted");
}

#[test]
fn router_section_round_trip() {
    use shadow_config::providers::{RouterConfig, RouteEntry};
    let mut config = Config::default();
    config.router = Some(RouterConfig {
        default: "openai.default".to_string(),
        routes: std::collections::HashMap::from([(
            "reasoning".to_string(),
            RouteEntry {
                provider: "anthropic.claude".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
            },
        )]),
        fallback_chains: std::collections::HashMap::from([(
            "default".to_string(),
            vec!["anthropic.claude".to_string()],
        )]),
    });
    let toml_str = toml::to_string_pretty(&config).unwrap();
    assert!(toml_str.contains("[router]"));
    assert!(toml_str.contains("default = \"openai.default\""));
    assert!(toml_str.contains("[router.routes.reasoning]"));

    let parsed: Config = toml::from_str(&toml_str).unwrap();
    let router = parsed.router.unwrap();
    assert_eq!(router.default, "openai.default");
}
