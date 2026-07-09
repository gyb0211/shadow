//! schema 版本迁移链
//!
//! 借鉴 zeroclaw-config 的迁移架构,精简为最小可用形态。
//!
//! - `CURRENT_SCHEMA_VERSION`: 当前代码理解的 schema 版本
//! - `MIGRATION_STEPS[i]`: 把 version (i+1) 迁移到 (i+2) 的函数
//! - 编译期断言: steps 数量恰好覆盖所有版本
//!
//! ## 版本历史
//!
//! - v1: 初始版本 -- 单 api_key 字段
//! - v2: api_keys 列表 + ReliableConfig (重试/退避/限流)
//!   迁移: `api_key = "x"` → `api_keys = ["x"]`

use anyhow::Result;

/// 当前 schema 版本
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// 单个迁移步骤: 接受旧版本 toml::Value,返回新版本 toml::Value。
type MigrationStep = fn(toml::Value) -> Result<toml::Value>;

/// 迁移链。`MIGRATION_STEPS[i]` 把 version (i+1) 迁移到 (i+2)。
pub const MIGRATION_STEPS: &[MigrationStep] = &[migrate_v1_to_v2];

// 编译期断言: steps 数量 + 1 == CURRENT。
const _: () = assert!(
    MIGRATION_STEPS.len() as u32 + 1 == CURRENT_SCHEMA_VERSION,
    "MIGRATION_STEPS must cover all versions up to CURRENT_SCHEMA_VERSION"
);

/// v1 → v2: 把每个 provider entry 的 `api_key = "x"` 转成 `api_keys = ["x"]`
///
/// 遍历 `providers.<family>.<alias>` 三层 table, 对每个 entry:
/// - 若有 `api_key` (string) 而无 `api_keys`, 转成单元素数组
/// - 若两者都有, 合并 (api_key 优先, 去重)
/// - 若只有 `api_keys`, 不动
fn migrate_v1_to_v2(mut value: toml::Value) -> Result<toml::Value> {
    let table = match value.as_table_mut() {
        Some(t) => t,
        None => return Ok(value),
    };
    let providers = match table.get_mut("providers").and_then(toml::Value::as_table_mut) {
        Some(t) => t,
        None => return Ok(value), // 无 providers 段, 无需迁移
    };
    for (_family, family_map) in providers.iter_mut() {
        let entries = match family_map.as_table_mut() {
            Some(t) => t,
            None => continue,
        };
        for (_alias, entry) in entries.iter_mut() {
            let entry_table = match entry.as_table_mut() {
                Some(t) => t,
                None => continue,
            };
            let single = entry_table.remove("api_key");
            if let Some(toml::Value::String(s)) = single {
                let existing = entry_table
                    .get("api_keys")
                    .and_then(|v: &toml::Value| v.as_array())
                    .cloned();
                let mut merged: Vec<toml::Value> = vec![toml::Value::String(s)];
                if let Some(arr) = existing {
                    for v in arr {
                        if !merged.contains(&v) {
                            merged.push(v);
                        }
                    }
                }
                entry_table.insert("api_keys".to_string(), toml::Value::Array(merged));
            }
        }
    }
    Ok(value)
}

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
pub fn migrate_str(input: &str) -> Result<Option<String>> {
    let mut value: toml::Value = toml::from_str(input)?;
    let detected = detect_version(&value);
    if detected >= CURRENT_SCHEMA_VERSION {
        return Ok(None);
    }
    // 运行迁移链: 从 detected 升到 CURRENT。
    // step 索引 (from-1) 对应 MIGRATION_STEPS[i] 把 (i+1)→(i+2)。
    // detected=0 (未版本化) 视作"比 v1 还早",从 v1 开始迁移。
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

