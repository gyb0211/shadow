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
        .api_key = Some("sk-plaintext".into());
    save_to(&config, dir).unwrap();
    let raw = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(
        raw.contains("enc2:"),
        "api_key must be encrypted on disk, got: {raw}"
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
        .api_key = Some("sk-roundtrip".into());
    save_to(&config, dir).unwrap();
    let loaded = load_from(dir).unwrap();
    assert_eq!(
        loaded
            .providers
            .find("openai", "default")
            .unwrap()
            .api_key
            .as_deref(),
        Some("sk-roundtrip")
    );
}

#[test]
fn load_decrypts_encrypted_api_key() {
    // 先在一个 dir 里 save 一个带 api_key 的配置,拿到 enc2: 形态,
    // 再清空内存重新 load,验证 decrypt 生效。
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut config = Config::default();
    config
        .providers
        .find_or_create("anthropic", "default")
        .api_key = Some("sk-ant-secret".into());
    save_to(&config, dir).unwrap();
    // 磁盘上 api_key 行必须是加密形态(值带 enc2: 前缀)
    let raw = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    let api_key_line = raw
        .lines()
        .find(|l| l.contains("api_key"))
        .expect("config must have an api_key line");
    assert!(
        api_key_line.contains("enc2:"),
        "api_key on disk must be encrypted, got: {api_key_line}"
    );
    // 重新 load 应解密回明文
    let loaded = load_from(dir).unwrap();
    assert_eq!(
        loaded
            .providers
            .find("anthropic", "default")
            .unwrap()
            .api_key
            .as_deref(),
        Some("sk-ant-secret")
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
fn save_load_empty_api_key_stays_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut config = Config::default();
    config.providers.find_or_create("openai", "default").api_key = None;
    save_to(&config, dir).unwrap();
    let loaded = load_from(dir).unwrap();
    assert!(loaded
        .providers
        .find("openai", "default")
        .unwrap()
        .api_key
        .is_none());
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
