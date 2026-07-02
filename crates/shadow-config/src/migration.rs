//! schema 版本迁移链
//!
//! 借鉴 zeroclaw-config 的迁移架构,精简为最小可用形态。
//!
//! - `CURRENT_SCHEMA_VERSION`: 当前代码理解的 schema 版本
//! - `MIGRATION_STEPS[i]`: 把 version (i+1) 迁移到 (i+2) 的函数
//! - 编译期断言: steps 数量恰好覆盖所有版本
//!
//! 当前 v1 是首个版本,steps 为空。未来加 v2 时:
//! 1. `CURRENT_SCHEMA_VERSION` 改为 2
//! 2. 在 `MIGRATION_STEPS` 末尾加一个 v1→v2 的 step 函数
//! 3. 在 v1 的"部分类型透镜"模块里实现该 step

use anyhow::Result;

/// 当前 schema 版本
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// 单个迁移步骤: 接受旧版本 toml::Value,返回新版本 toml::Value。
type MigrationStep = fn(toml::Value) -> Result<toml::Value>;

/// 迁移链。`MIGRATION_STEPS[i]` 把 version (i+1) 迁移到 (i+2)。
///
/// 当前为空 -- v1 是首个版本,无历史可迁。
pub const MIGRATION_STEPS: &[MigrationStep] = &[];

// 编译期断言: steps 数量 + 1 == CURRENT。加版本时忘了补 step 会直接编译失败。
const _: () = assert!(
    MIGRATION_STEPS.len() as u32 + 1 == CURRENT_SCHEMA_VERSION,
    "MIGRATION_STEPS must cover all versions up to CURRENT_SCHEMA_VERSION"
);

/// 检测 toml 文档的 schema 版本。
///
/// - 缺失 `schema_version` 字段 → 0 (表示"未版本化",等价于 v1 前身)
/// - 字段为非正整数 → 0
/// - 否则取字段值
fn detect_version(value: &toml::Value) -> u32 {
    value
        .as_table()
        .and_then(|t| t.get("schema_version"))
        .and_then(|v| v.as_integer())
        .filter(|i| *i > 0)
        .map(|i| i as u32)
        .unwrap_or(0)
}

/// 迁移 toml 字符串到当前 schema 版本。
///
/// - 输入已是当前版本 → 返回 `Ok(None)`
/// - 输入需要迁移 → 返回 `Ok(Some(新 toml 字符串))`,内含 `schema_version = CURRENT`
///
/// 当前 v1 是首个版本,迁移行为仅为"缺失字段则补戳"。
pub fn migrate_str(input: &str) -> Result<Option<String>> {
    let mut value: toml::Value = toml::from_str(input)?;
    let detected = detect_version(&value);
    if detected >= CURRENT_SCHEMA_VERSION {
        return Ok(None);
    }
    // 运行迁移链: 从 detected 升到 CURRENT。
    // step 索引 (from-1) 对应 MIGRATION_STEPS[i] 把 (i+1)→(i+2)。
    // detected=0 (未版本化) 视作"比 v1 还早",但 v1 是基线,无需数据转换,只补版本戳。
    for from in detected.max(1)..CURRENT_SCHEMA_VERSION {
        let idx = (from - 1) as usize;
        if idx < MIGRATION_STEPS.len() {
            value = (MIGRATION_STEPS[idx])(value)?;
        }
    }
    // 补戳当前版本号。
    if let Some(table) = value.as_table_mut() {
        table.insert(
            "schema_version".into(),
            toml::Value::Integer(i64::from(CURRENT_SCHEMA_VERSION)),
        );
    }
    Ok(Some(toml::to_string_pretty(&value)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_version_returns_zero_when_missing() {
        let v: toml::Value = toml::from_str("[agent]\nalias = \"x\"").unwrap();
        assert_eq!(detect_version(&v), 0);
    }

    #[test]
    fn detect_version_reads_field() {
        let v: toml::Value = toml::from_str("schema_version = 3\n").unwrap();
        assert_eq!(detect_version(&v), 3);
    }

    #[test]
    fn detect_version_non_positive_becomes_zero() {
        let v: toml::Value = toml::from_str("schema_version = 0\n").unwrap();
        assert_eq!(detect_version(&v), 0);
        let v: toml::Value = toml::from_str("schema_version = -1\n").unwrap();
        assert_eq!(detect_version(&v), 0);
    }

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
}
