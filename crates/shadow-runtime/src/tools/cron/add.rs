use std::str::FromStr;
use crate::security::SecurityPolicy;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Error, Value};
use shadow_config::Config;
use std::sync::Arc;
use async_trait::async_trait;
use shadow_core::{Tool, ToolResult};
use crate::cron::{add_agent_job, add_shell_job_with_approval, CRON_DELIVERY_SCHEMA_CHANNELS};
use crate::cron::types::DeliveryConfig;
use crate::tools::cron::common::{cron_add_output, deserialize_schedule_arg, AT_DESCRIPTION, CRON_TZ_DESCRIPTION};
use crate::tools::cron::types::{ SessionTarget};

enum Arg {
    Schedule(Schedule),
    AfterSeconds(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Schedule {
    Cron {
        expr: String,
        #[serde(default)]
        tz: Option<String>,
    },
    At {
        at: DateTime<Utc>,
    },
    Every {
        every_ms: u64,
    },
}


#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename="lowercase")]
pub enum JobType {
    #[default]
    Shell,
    Agent,
}

impl From<JobType> for &'static str {
    fn from(value: JobType) -> Self {
        match value {
            JobType::Shell => "shell",
            JobType::Agent => "agent",
        }
    }
}

impl TryFrom<&str> for JobType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {

        match value.to_lowercase().as_str() {
            "shell" => Ok(JobType::Shell),
            "agent" => Ok(JobType::Agent),
            _ => Err(format!(
                "Invalid job type '{}'. expected one of 'shell', 'agent'.", value
            ))
        }
    }
}

impl Arg {
    fn after_from_value(value: &Value) -> Result<Self, String> {
        let after = value
            .get("after_seconds")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                format!("Invalid schedule: after_seconds must be an integer. now is {value}")
            })?;
        if after == 0 {
            return Err("Invalid schedule: after_seconds must > 0".to_string());
        }
        Ok(Self::AfterSeconds(after))
    }

    fn default_delete_after_run(&self) -> bool {
        matches!(
            self,
            Self::Schedule(Schedule::At { .. }) | Self::AfterSeconds(_)
        )
    }

    fn into_schedule(self) -> Result<Schedule, String> {
        match self {
            Arg::Schedule(schedule) => Ok(schedule),
            Arg::AfterSeconds(secs) => {
                let after = i64::try_from(secs)
                    .map_err(|_| "Invalid schedule: after_seconds is too large")?;
                let delay = ChronoDuration::seconds(after);
                let at = Utc::now().checked_add_signed(delay).ok_or_else(|| {
                    "Invalid schedule: after_seconds  overflowed DateTime arithmetic".to_string()
                })?;

                Ok(Schedule::At { at })
            }
        }
    }
}

pub struct CronAddTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
    agent_alias: String,
}

impl CronAddTool {
    pub fn new(
        config: Arc<Config>,
        security: Arc<SecurityPolicy>,
        agent_alias: impl Into<String>,
    ) -> Self {
        Self {
            config,
            security,
            agent_alias: agent_alias.into(),
        }
    }

    fn deserialize_arg(value: &Value) -> Result<Arg, String> {
        if let Some(normalized) = normalize_schedule_arg(value)?
            && normalized.get("kind").and_then(Value::as_str) == Some("after")
        {
            return Arg::after_from_value(&value);
        }
        deserialize_schedule_arg(value).map(Arg::Schedule)
    }

    fn enforce_mutation_allowed(&self, action: &str) -> Option<ToolResult>{
        if !self.security.can_act(){
            return Some(ToolResult::err(format!("Security policy: read-only mode, cannot perform '{action}'")));
        }

        if self.security.is_rate_limited() {
            return Some(ToolResult::err("Rate limit exceed: too many actions in the last hour."));
        }

        if !self.security.record_action() {
            return Some(ToolResult::err("Rate limit exceed: action budget exhausted."));
        }
        None
    }

    fn plain_schedule_error(raw: &str) -> Option<String> {
        let schedule = raw.trim();

        if schedule.starts_with("{") {
            return None;
        }

        let got = serde_json::to_string(schedule).unwrap_or_else(|_| "\"<invalid>\"".to_string());
        Some(format!(
            "Invalid schedule: expected a JSON object with \"kind\" field, go plain string {got}.\
            Use one of:
            "
        ))
    }
}

fn normalize_schedule_arg(value: &Value) -> Result<Option<Value>, String> {
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with("{") {
                serde_json::from_str(trimmed)
                    .map(Some)
                    .map_err(|e| format!("Invalid schedule: {e}"))
            } else {
                Ok(None)
            }
        }
        other => Ok(Some(other.clone())),
    }
}

#[async_trait]
impl Tool for CronAddTool{
    fn name(&self) -> &str {
        "cron_add"
    }

    fn description(&self) -> &str {
        "Create a scheduled cron job (shell or agent) with cron/at/after/every schedules. \
         Use job_type='agent' with a prompt to run the AI agent on schedule. \
         For relative one-shot reminders such as 'in 10 minutes' or 'after 2 hours', \
         use schedule={\"kind\":\"after\",\"after_seconds\":...}; the runtime resolves it \
         with the live clock when the tool executes. \
         To deliver output to a configured channel, set \
         delivery={\"mode\":\"announce\",\"channel\":\"discord\",\"to\":\"<channel_id_or_chat_id>\"}. \
         For webhook deliveries that must thread through the originating conversation, also set \
         delivery.thread_id=\"<reply_target>\". \
         This is the preferred tool for sending scheduled/delayed messages to users via channels."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Optional human-readable name for the job"
                },
                // NOTE: oneOf is correct for OpenAI-compatible APIs (including OpenRouter).
                // Gemini does not support oneOf in tool schemas; if Gemini native tool calling
                // is ever wired up, SchemaCleanr::clean_for_gemini must be applied before
                // tool specs are sent. See src/tools/schema.rs.
                "schedule": {
                    "description": "When to run the job. Exactly one of four forms must be used. Prefer 'after' for relative one-shot reminders.",
                    "oneOf": [
                        {
                            "type": "object",
                            "description": "Cron expression schedule (repeating). Example: {\"kind\":\"cron\",\"expr\":\"0 9 * * 1-5\",\"tz\":\"America/New_York\"}",
                            "properties": {
                                "kind": { "type": "string", "enum": ["cron"] },
                                "expr": { "type": "string", "description": "Standard 5-field cron expression, e.g. '*/5 * * * *'" },
                                "tz": { "type": "string", "description": CRON_TZ_DESCRIPTION }
                            },
                            "required": ["kind", "expr"]
                        },
                        {
                            "type": "object",
                            "description": "One-shot schedule at a specific RFC3339 timestamp with explicit Z or offset. Example: {\"kind\":\"at\",\"at\":\"2025-12-31T23:59:00Z\"}",
                            "properties": {
                                "kind": { "type": "string", "enum": ["at"] },
                                "at": { "type": "string", "description": AT_DESCRIPTION }
                            },
                            "required": ["kind", "at"]
                        },
                        {
                            "type": "object",
                            "description": "One-shot relative delay in seconds. Prefer this for reminders like 'in 10 minutes' so the runtime resolves the live clock. Example: {\"kind\":\"after\",\"after_seconds\":600}",
                            "properties": {
                                "kind": { "type": "string", "enum": ["after"] },
                                "after_seconds": { "type": "integer", "minimum": 1, "description": "Delay from job creation time in seconds, e.g. 600 for 10 minutes" }
                            },
                            "required": ["kind", "after_seconds"]
                        },
                        {
                            "type": "object",
                            "description": "Repeating interval schedule in milliseconds. Example: {\"kind\":\"every\",\"every_ms\":3600000} runs every hour.",
                            "properties": {
                                "kind": { "type": "string", "enum": ["every"] },
                                "every_ms": { "type": "integer", "description": "Interval in milliseconds, e.g. 3600000 for every hour" }
                            },
                            "required": ["kind", "every_ms"]
                        }
                    ]
                },
                "job_type": {
                    "type": "string",
                    "enum": ["shell", "agent"],
                    "description": "Type of job: 'shell' runs a command, 'agent' runs the AI agent with a prompt"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to run (required when job_type is 'shell')"
                },
                "prompt": {
                    "type": "string",
                    "description": "Agent prompt to run on schedule (required when job_type is 'agent')"
                },
                "session_target": {
                    "type": "string",
                    "enum": ["isolated", "main"],
                    "description": "Agent session context: 'isolated' starts a fresh session each run, 'main' reuses the primary session"
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override for agent jobs, e.g. 'x-ai/grok-4-1-fast'"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional allowlist of tool names for agent jobs. When omitted, cron-launched agent runs keep non-scheduler tools available but exclude scheduler mutation tools such as cron_add, cron_update, cron_remove, cron_run, and schedule. Include those names explicitly to opt back in."
                },
                "delivery": {
                    "type": "object",
                    "description": "Optional delivery config to send job output to a channel after each run. When provided, all three of mode, channel, and to are expected.",
                    "properties": {
                        "mode": {
                            "type": "string",
                            "enum": ["none", "announce"],
                            "description": "'announce' sends output to the specified channel; 'none' disables delivery"
                        },
                        "channel": {
                            "type": "string",
                            "enum": CRON_DELIVERY_SCHEMA_CHANNELS,
                            "description": "Channel type to deliver output to"
                        },
                        "to": {
                            "type": "string",
                            "description": "Destination ID: Discord channel ID, Telegram chat ID, Slack channel name, webhook recipient, etc."
                        },
                        "thread_id": {
                            "type": "string",
                            "description": "Optional thread/conversation identifier. Used by the webhook channel to route callbacks to the originating conversation; ignored by channels whose threading is implied by `to`."
                        },
                        "best_effort": {
                            "type": "boolean",
                            "description": "If true, a delivery failure does not fail the job itself. Defaults to true."
                        }
                    }
                },
                "delete_after_run": {
                    "type": "boolean",
                    "description": "If true, the job is automatically deleted after its first successful run. Defaults to true for one-shot 'at' and 'after' schedules."
                },
                "approved": {
                    "type": "boolean",
                    "description": "Set true to explicitly approve medium/high-risk shell commands in supervised mode",
                    "default": false
                }
            },
            "required": ["schedule"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if !self.config.scheduler.enabled {
            return Ok(ToolResult::err("cron is disabled by config(scheduler.enabled=false)"));
        }

        let schedule_arg = match args.get("schedule") {

            Some(v @ serde_json::Value::String(raw)) => {
                if let Some(error) = Self::plain_schedule_error(raw) {
                    return Ok(ToolResult::err(error));
                }
                match Self::deserialize_arg(v) {
                    Ok(schedule) =>schedule,
                    Err(error) => return Ok(ToolResult::err(error)),
                }
            }

            Some(v)  =>  match Self::deserialize_arg(v) {
                Ok(schedule) =>schedule,
                Err(error) => return Ok(ToolResult::err(error)),
            }
            None => return Ok(ToolResult::err("Missing 'schedule' parameter"))
        };


        let name = args.get("name").and_then(Value::as_str).map(str::to_string);
        let job_type = match args.get("job_type").and_then(Value::as_str) {
            Some("agent") => JobType::Agent,
            Some("shell") => JobType::Shell,
            Some(other) =>  return Ok(ToolResult::err(format!("Invalid job_type: {other}"))),
            None => {
                if args.get("prompt").is_some() {
                    JobType::Agent
                }else{
                    JobType::Shell
                }
            }

        };

        let del_after_run = args.get("delete_after_run").and_then(Value::as_bool).unwrap_or(schedule_arg.default_delete_after_run());
        let approved = args.get("approved").and_then(Value::as_bool).unwrap_or(false);
        let delivery = match args.get("delivery") {
            Some(v) => match serde_json::from_value::<DeliveryConfig>(v.clone()) {
                Ok(cfg)=> Some(cfg),
                Err(err) => return Ok(ToolResult::err(format!("Invalid delivery config: {err}"))),
            },
            None => None

        };

        let result = match job_type {
            JobType::Shell => {
                let command = match args.get("command").and_then(Value::as_str) {
                    Some(cmd) if !cmd.trim().is_empty() => cmd,
                    _ => return Ok(ToolResult::err("Missing 'command' for shell job.")),
                };

                if let Err(reason) = self.security.validate_command_execution(command, approved){}

                if let Some(blocked) = self.enforce_mutation_allowed("cron_add")
                {
                    return Ok(blocked);
                }

                let schedule = match schedule_arg.into_schedule() {
                    Ok(schedule) => schedule,
                    Err(error) => return Ok(ToolResult::err(error)),
                };

                add_shell_job_with_approval(
                    &self.config,
                    &self.agent_alias,
                    name,
                    schedule,
                    command,
                    delivery,
                    approved
                )
            }
            JobType::Agent => {
                let prompt = match args.get("prompt").and_then(Value::as_str) {
                    Some(prompt) if !prompt.trim().is_empty() => prompt,
                    _ => return Ok(ToolResult::err("Agent job missing 'prompt'")),

                };

                let session_target = match args.get("session_target") {
                    Some(v) => match serde_json::from_value::<SessionTarget>(v.clone()) {
                        Ok(target) => target,
                        Err(e) => return Ok(ToolResult::err(format!("Invalid session_target: {e}"))),
                    }
                    None => SessionTarget::Isolated

                };

                let model = args.get("model").and_then(Value::as_str).map(str::to_string);

                let allowed_tools = match args.get("allowed_tools"){
                    Some(tools) => match serde_json::from_value::<Vec<String>>(tools.clone()) {
                        Ok(als) => {
                            if als.is_empty() {None} else{Some(als)}
                        }
                        Err(err) => return Ok(ToolResult::err(format!("Invalid allowed_tools: {err}"))),
                    }
                    None => None
                };

                if let Some(blocked) = self.enforce_mutation_allowed("cron_add") {
                    return Ok(blocked);
                }

                let schedule = match schedule_arg.into_schedule() {
                    Ok(schedule) => schedule,
                    Err(error) => return Ok(ToolResult::err(error)),
                };

                add_agent_job( &self.config,
                               &self.agent_alias,
                               name,
                               schedule,
                               prompt,
                               session_target,
                               model,
                               delivery,
                               del_after_run,
                               allowed_tools,)
            }


        };

        match result {
            Ok(job) => Ok(ToolResult::ok(serde_json::to_string_pretty(&cron_add_output(&job))?)),
            Err(e) => Ok(ToolResult::err("")),
        }
    }
}