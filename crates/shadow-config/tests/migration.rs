//! migration 模块集成测试
//!
//! 注:`detect_version` 是私有 helper,不在此直接测试。
//! 它的行为通过 `migrate_str` 的公共 API 覆盖(缺失/0/当前版本各路径)。

use shadow_config::{migrate_str, CURRENT_SCHEMA_VERSION};

#[test]
fn migrate_stamps_version_on_unversioned_input() {
    let input = "[agent]\nalias = \"x\"\n";
    let out = migrate_str(input).unwrap().unwrap();
    let parsed: toml::Value = toml::from_str(&out).unwrap();
    assert_eq!(
        parsed["schema_version"].as_integer(),
        Some(i64::from(CURRENT_SCHEMA_VERSION))
    );
    // 保留原有字段
    assert_eq!(parsed["agent"]["alias"].as_str(), Some("x"));
}

#[test]
fn migrate_returns_none_when_already_current() {
    let input = format!("schema_version = {}\n", CURRENT_SCHEMA_VERSION);
    assert!(migrate_str(&input).unwrap().is_none());
}

#[test]
fn migrate_is_idempotent() {
    let input = "[agent]\nalias = \"x\"\n";
    let once = migrate_str(input).unwrap().unwrap();
    // 第二次迁移应该返回 None (已是当前版本)
    assert!(migrate_str(&once).unwrap().is_none());
}

#[test]
fn migrate_stamps_when_schema_version_is_zero() {
    // schema_version = 0 视作"未版本化",应补戳到当前版本
    let input = "schema_version = 0\n[agent]\nalias = \"x\"\n";
    let out = migrate_str(input).unwrap().unwrap();
    let parsed: toml::Value = toml::from_str(&out).unwrap();
    assert_eq!(
        parsed["schema_version"].as_integer(),
        Some(i64::from(CURRENT_SCHEMA_VERSION))
    );
}

#[test]
fn migrate_preserves_unrelated_sections() {
    let input = "[agent]\nalias = \"x\"\n[memory]\nbackend = \"markdown\"\n";
    let out = migrate_str(input).unwrap().unwrap();
    let parsed: toml::Value = toml::from_str(&out).unwrap();
    assert_eq!(parsed["memory"]["backend"].as_str(), Some("markdown"));
}
