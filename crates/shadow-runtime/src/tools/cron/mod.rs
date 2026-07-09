use crate::cron::CronJob;
use crate::tools::cron::add::{CronAddTool, Schedule};
use crate::tools::cron::types::{DeliveryConfig, SessionTarget};
use chrono::Utc;
use shadow_config::Config;
use shadow_core::{ToolKind, tool_attribution};
use uuid::Uuid;

tool_attribution!(CronAddTool, ToolKind::Plugin);
pub mod add;
mod common;
mod list;
mod remove;
mod run;
mod runs;
mod types;

pub fn add_shell_job_with_approval(
    config: &Config,
    agent_alias: &str,
    name: Option<String>,
    schedule: Schedule,
    command: &str,
    delivery: Option<DeliveryConfig>,
    approved: bool,
) -> Result<CronJob, String> {
    println!("add_shell_job_with_approval command");
    Ok(CronJob {})
}

pub fn add_agent_job(
    config: &Config,
    agent_alias: &str,
    name: Option<String>,
    schedule: Schedule,
    prompt: &str,
    session_target: SessionTarget,
    model: Option<String>,
    delivery: Option<DeliveryConfig>,
    del_after_run: bool,
    allowed_tools: Option<Vec<String>>,
) -> Result<CronJob, String> {
    let now = Utc::now();
    validate_schedule(&schedule, now)?;
    validate_delivery_config(delivery.as_ref())?;

    let next_run = next_run_for_schedule(&schedule, now)?;
    let id = Uuid::new_v4().to_string();
    let expr = schedule_cron_expression(&schedule).unwrap_or_default();
    let schedule_json = serde_json::to_string(&schedule)?;
    let delivery = delivery.unwrap_or_default();
    let agent_alias = agent_alias.trim();
    if agent_alias.is_empty() {anyhow::bail!("agent_alias is required; cron jobs must name an owning agent");}
    
    with_initialized_connection(config, |conn|{
        conn.execute()
    })

    Ok(CronJob {})
}
