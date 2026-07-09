use std::str::FromStr;
use chrono::DateTime;

use crate::cron::{deserialize_maybe_stringified, CronJob};
use crate::tools::cron::add::Schedule;
use serde_json::{Value, json};

pub fn deserialize_schedule_arg(value: &Value) -> Result<Schedule, String> {
    reject_at_without_explicit_offset(value)?;
    deserialize_maybe_stringified(value).map_err(|err| format!("Invalid schedule: {err}"))
}

fn reject_at_without_explicit_offset(value: &Value) -> Result<(), String> {
    let Some(normalized) = normalize_maybe_json_str(value) else {
        return Ok(());
    };

    if normalized.get("kind").and_then(Value::as_str) != Some("at") {
        return Ok(());
    }

    let Some(raw_at) = normalized.get("at").and_then(Value::as_str) else {
        return Ok(());
    };

    DateTime::parse_from_rfc3339(raw_at)
        .map(|_| ())
        .map_err(|err| {
            format!(
                "Invalid schedule: 'at' must be an RFC3339 timestamp with explicit Z or offset,\
            e.g. 2026-05-18T08:00:00Z or 2026-05-18T08:00:00-04:00; got '{raw_at}: {err}'"
            )
        })
}

fn normalize_maybe_json_str(value: &Value) -> Option<Value> {
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with("{") || trimmed.starts_with("[") {
                serde_json::from_str(trimmed).ok()
            } else {
                None
            }
        }
        other => Some(other.clone()),
    }
}

pub fn cron_add_output(job: &CronJob) -> Value {
    let fields = timezone_confirmation_fields(job);
    json!({
        "id": job.id,
        "name":job.name,
        "schedule": job.schedule,
        "next_run": job.next_run,
        "enabled": job.enabled,
        "schedule_timezone": fields.schedule_timezone,
        "timezone_source": fields.timezone_source,
        "next_run_local": fields.next_run_local,
    })
}
struct TimezoneConfirmationFields {
    schedule_timezone: Value,
    timezone_source: Value,
    next_run_local: Value,
}
fn timezone_confirmation_fields(job: &CronJob) -> TimezoneConfirmationFields {
    match &job.schedule {
        Schedule::Cron { tz: Some(tz), .. } => {
            let next_run_local = chrono_tz::Tz::from_str(tz).map_or(Value::Null, |timezone| {
                json!(job.next_run.with_timezone(&chrono::Local).to_rfc3339())
            });
            TimezoneConfirmationFields {
                schedule_timezone: json!(tz),
                timezone_source: json!("explicit"),
                next_run_local,
            }
        }
        Schedule::Cron { tz: None, .. } => TimezoneConfirmationFields {
            schedule_timezone: json!("runtime local timezone"),
            timezone_source: json!("runtime_local"),
            next_run_local: json!(job.next_run.with_timezone(&chrono::Local).to_rfc3339()),
        },
        Schedule::At { .. } | Schedule::Every { .. } => TimezoneConfirmationFields {
            schedule_timezone: Value::Null,
            timezone_source: json!("not_applicable"),
            next_run_local: Value::Null,
        },
    }
}
