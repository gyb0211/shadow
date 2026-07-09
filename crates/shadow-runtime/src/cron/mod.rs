//! Cron 调度器 -- 定时任务调度与持久化
//!
//! 借鉴 ZeroClaw 的 Cron 设计但精简:
//! - ZeroClaw: 7306 行, SQLite 持久化, 安全验证, announce 投递
//! - Shadow: 简单调度 + SQLite 持久化
//!
//! cron 表达式格式 (6-7 字段, 使用 cron crate 解析):
//! ```text
//! 秒 分 时 日 月 周 [年]
//! *  *  *  *  *  *
//! |  |  |  |  |  |
//! |  |  |  |  |  +-- 星期 (0-7, 0 和 7 都是周日)
//! |  |  |  |  +---- 月 (1-12)
//! |  |  |  +------ 日 (1-31)
//! |  |  +-------- 时 (0-23)
//! |  +----------- 分 (0-59)
//! +-------------- 秒 (0-59)
//! ```
//!
//! 示例:
//! - `"0 * * * * *"` -- 每分钟第 0 秒执行
//! - `"*/30 * * * * *"` -- 每 30 秒执行一次
//! - `"0 0 * * * *"` -- 每天午夜执行
//! - `"0 0 9 * * 1-5"` -- 工作日每天上午 9 点执行

mod schedule;
pub mod types;

use crate::tools::cron::add::{JobType, Schedule};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Error, Value};
use shadow_log::Action;
use std::path::Path;
use uuid::Uuid;
use shadow_config::Config;
use crate::cron::schedule::{next_run_for_schedule, validate_delivery_config, validate_schedule};
use crate::cron::types::DeliveryConfig;

pub(crate) const CRON_DELIVERY_SCHEMA_CHANNELS: &[&str] = &[
    "feishu"
];




pub fn add_shell_job_with_approval(
    config: &Config,
    agent_alias: &str,
    name: Option<String>,
    schedule: Schedule,
    command: &str,
    delivery: Option<DeliveryConfig>,
    approved: bool,
) -> anyhow::Result<CronJob> {
    bail!("delivery.to is required for announce mode");
}

pub fn add_agent_job(
    config: &Config,
    agent_alias: &str,
    name: Option<String>,
    schedule: Schedule,
    prompt: &str,
    session_target: crate::tools::cron::types::SessionTarget,
    model: Option<String>,
    delivery: Option<crate::cron::types::DeliveryConfig>,
    del_after_run: bool,
    allowed_tools: Option<Vec<String>>,
) -> anyhow::Result<CronJob> {
    let now = Utc::now();
    validate_schedule(&schedule, now)?;
    validate_delivery_config(delivery.as_ref())?;

    let next_run = next_run_for_schedule(&schedule, now)?;
    let id = Uuid::new_v4().to_string();
    // let expr = schedule_cron_expression(&schedule).unwrap_or_default();
    let schedule_json = serde_json::to_string(&schedule)?;
    let delivery = delivery.unwrap_or_default();
    let agent_alias = agent_alias.trim();
    if agent_alias.is_empty() {anyhow::bail!("agent_alias is required; cron jobs must name an owning agent");}

    // with_initialized_connection(config, |conn|{
    //     conn.execute()
    // });
    bail!("delivery.to is required for announce mode");

}

/// 如果收到是 Value 可以直接转成 T 就直接转
/// 如果不能转 就变成字符串 再尝试转
pub fn deserialize_maybe_stringified<T: DeserializeOwned>(
    v: &Value,
) -> Result<T, serde_json::Error> {
    match serde_json::from_value::<T>(v.clone()) {
        Ok(v) => Ok(v),
        Err(err) => {
            if let Some(s) = v.as_str() {
                let s = s.trim();
                if (s.starts_with("{") || s.starts_with("["))
                    && let Ok(inner) = serde_json::from_str::<Value>(s)
                {
                    return serde_json::from_value::<T>(inner);
                }
            }
            Err(err)
        }
    }
}

// ───────────────────────── 数据结构 ─────────────────────────

/// Cron 作业 -- 一个定时任务的定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub expression: String,
    pub schedule: Schedule,
    pub job_type: JobType,
    pub enabled: bool,
    pub next_run: DateTime<Utc>,
}

/// Cron 运行记录 -- 一次作业执行的历史记录
#[derive(Debug, Clone)]
pub struct CronRun {
    /// 运行记录 ID (由数据库分配, 新建时设为 0)
    pub id: i64,
    /// 关联的作业 ID
    pub job_id: i64,
    /// 开始时间 (Unix 时间戳)
    pub started_at: i64,
    /// 结束时间 (Unix 时间戳, None 表示仍在运行)
    pub finished_at: Option<i64>,
    /// 运行状态: "success" / "failed" / "running"
    pub status: String,
    /// 输出内容 (stdout + stderr, 已截断)
    pub output: String,
}

/// 运行输出最大长度 (10KB), 超过则截断
const MAX_OUTPUT_LEN: usize = 10 * 1024;
