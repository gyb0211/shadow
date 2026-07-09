//! 调度配置校验与下次运行时间计算.
//!
//! 本模块对 [`crate::tools::cron::add::Schedule`] 三种形态做语法/语义校验, 并统一计算下一次触发时间:
//! - `Cron`: 标准 cron 表达式, 可选时区 (`tz`). 用 `cron` crate 解析.
//! - `At`:   一次性绝对时间点.
//! - `Every`: 固定间隔循环.
//!
//! 注意: `cron` crate 的类型也叫 `Schedule`, 本文中以 `cron::Schedule` 全限定书写,
//! 避免与 [`crate::tools::cron::add::Schedule`] 混淆.

use crate::tools::cron::add::Schedule;
use crate::cron::Schedule as CronExprSchedule;
use anyhow::Context;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use chrono_tz::Tz as ChronoTz;

/// 校验调度配置是否合法.
///
/// 对每种调度类型做对应检查:
/// - `Cron`: 表达式可解析 (语法) 且存在至少一个未来触发点 (语义).
/// - `At`:   `at` 必须严格大于 `now` (过去时间没有意义).
/// - `Every`: `every_ms` 必须大于 0 (为 0 会导致调度器死循环).
///
/// # 参数
/// - `schedule`: 待校验的调度配置.
/// - `now`:      当前时间基准 (一般传 `Utc::now()`, 测试中可注入).
pub fn validate_schedule(schedule: &Schedule, now: DateTime<Utc>) -> anyhow::Result<()> {
    match schedule {
        Schedule::Cron { expr, .. } => {
            // 1. 语法校验 + 归一化 (失败直接返回错误)
            normalize_expression(expr)?;
            // 2. 语义校验: 确认存在未来触发点, 排除只匹配过去日期的表达式
            next_run_for_schedule(schedule, now)?;
            Ok(())
        }
        Schedule::At { at, .. } => {
            // 一次性任务: 触发时间必须在未来 (允许 == 视为已过期, 也拒绝)
            if *at <= now {
                anyhow::bail!("'at' 时间必须在未来 (now={now}, at={at})");
            }
            Ok(())
        }
        Schedule::Every { every_ms, .. } => {
            // 间隔为 0 会让调度器无延迟地反复触发, 直接拒绝
            if *every_ms == 0 {
                anyhow::bail!("'every_ms' 必须大于 0");
            }
            Ok(())
        }
    }
}

/// 归一化 cron 表达式字符串 (只在字段层面处理, 不做完整语法校验).
///
/// cron 标准格式支持 5/6/7 个字段:
/// - 5 位: 分 时 日 月 周          → 前补 "0" 秒, 升级为 6 位
/// - 6 位: 秒 分 时 日 月 周       → 不变
/// - 7 位: 秒 分 时 日 月 周 年    → 不变
///
/// 字段数 < 5 或 > 7 视为非法, 返回错误.
/// 各字段内容是否合法 (如 `*/5`、`1-5`、`9` 等) 不在此检查,
/// 留给后续 `cron::Schedule` 解析.
///
/// 返回归一化后的表达式 (字段以单空格连接, 已规整秒字段).
fn normalize_expression(expr: &str) -> anyhow::Result<String> {
    // split_whitespace 已自动处理首尾与字段间多余空白
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let normalized: Vec<&str> = match fields.len() {
        5 => {
            // 5 位 (无秒): 前补 "0" 秒字段, 升级为标准 6 位
            let mut v = Vec::with_capacity(6);
            v.push("0");
            v.extend(fields);
            v
        }
        // 6 位 (含秒) 或 7 位 (含秒+年): 原样保留
        6 | 7 => fields,
        n => anyhow::bail!(
            "无效的 cron 表达式: 字段数 {n} 不合法 (期望 5/6/7): {expr}"
        ),
    };
    Ok(normalized.join(" "))
}

/// 计算调度配置的下一次触发时间, 统一以 UTC 返回.
///
/// - `Cron`: 先归一化再解析. 若提供 `tz`, 在该时区下迭代下一次触发, 再转回 UTC;
///   否则按 UTC 计算. 解析失败或无未来触发点均返回错误.
/// - `At`:   直接返回 `at` (调用方应已通过 [`validate_schedule`] 确认是未来).
/// - `Every`: `now + every_ms` 毫秒; 溢出时返回错误.
///
/// # 参数
/// - `schedule`: 调度配置.
/// - `now`:      基准时间 ("从何时起算下一次").
pub fn next_run_for_schedule(
    schedule: &Schedule,
    now: DateTime<Utc>,
) -> anyhow::Result<DateTime<Utc>> {
    match schedule {
        Schedule::Cron { expr, tz } => {
            // 归一化 + 解析 (复用校验逻辑, 保证进入迭代的表达式是合法的)
            let normalized = normalize_expression(expr)?;
            let cron = CronExprSchedule::from_str(&normalized)
                .with_context(|| format!("无效的 cron 表达式: {expr}"))?;

            if let Some(tz_str) = tz {
                // 指定时区: 解析时区名 (如 "America/New_York" / "Asia/Shanghai")
                let tz: ChronoTz = tz_str
                    .parse()
                    .with_context(|| format!("无效的时区: {tz_str}"))?;
                // 将基准时间转到目标时区, cron 字段按当地墙上时间解释
                let now_tz = now.with_timezone(&tz);
                let next = cron
                    .after(&now_tz)
                    .next().ok_or_else(|| {
                    anyhow::Error::msg(format!("expr: {expr} 没有未来可触发的时间"))
                })?;
                // 统一转回 UTC 便于存储与比较
                Ok(next.with_timezone(&Utc))
            } else {
                // 未指定时区: 按 UTC 解释 cron 字段
                let next = cron
                    .after(&now)
                    .next()
                    .context("cron 表达式没有未来触发时间")?;
                Ok(next)
            }
        }
        Schedule::At { at, .. } => {
            // 一次性绝对时间: 下次触发就是它本身
            Ok(*at)
        }
        Schedule::Every { every_ms, .. } => {
            // every_ms 是 u64, 转 i64 以构造 chrono::Duration; 理论上极少溢出但仍防御
            let ms = i64::try_from(*every_ms).context("'every_ms' 超出 i64 范围")?;
            let duration = ChronoDuration::milliseconds(ms);
            // checked_add_signed 在日期溢出时返回 None
            now.checked_add_signed(duration)
                .ok_or_else(|| anyhow::anyhow!("计算下次运行时间溢出 (every_ms={every_ms})"))
        }
    }
}
