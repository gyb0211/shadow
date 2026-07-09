//! Cron 定时任务管理工具
//!
//! 统一接口, action 参数区分: add/list/remove/run/runs/update
//! 支持: 过滤, 分页, 手动执行

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use crate::cron::{CronJob, CronRun, CronScheduler};
use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

pub struct CronTool {
    store: Option<Arc<CronScheduler>>,
}

impl CronTool {
    pub fn new() -> Self {
        Self { store: None }
    }
    pub fn with_store(store: Arc<CronScheduler>) -> Self {
        Self { store: Some(store) }
    }
}

impl Default for CronTool {
    fn default() -> Self {
        Self::new()
    }
}



#[async_trait]
impl Tool for CronTool {
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> &str {
        "管理定时任务。add(添加) / list(列出,支持过滤) / remove(删除) / run(手动执行) / runs(执行历史,支持分页) / update(更新)"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["add", "list", "remove", "run", "runs", "update"], "description": "操作类型" },
                "id": { "type": "integer", "description": "任务 ID (remove/run/runs/update)" },
                "name": { "type": "string", "description": "任务名称 (add)" },
                "schedule": { "type": "string", "description": "cron 表达式 6 字段 (如 '0 0 9 * * *')" },
                "command": { "type": "string", "description": "要执行的 shell 命令 (add)" },
                "enabled": { "type": "boolean", "description": "是否启用 (update)" },
                "filter": { "type": "string", "enum": ["all", "enabled", "disabled"], "default": "all", "description": "list 过滤 (默认 all)" },
                "limit": { "type": "integer", "description": "runs 分页: 返回条数 (默认 10)" },
                "offset": { "type": "integer", "description": "runs 分页: 偏移量 (默认 0)" },
                "status_filter": { "type": "string", "enum": ["all", "success", "failed", "running"], "default": "all", "description": "runs 状态过滤" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");
        let store = match &self.store {
            Some(s) => s,
            None => {
                return Ok(ToolResult::err(
                    "未配置 CronScheduler。请先初始化 cron 持久化。",
                ));
            }
        };

        match action {
            "add" => self.do_add(store, &args).await,
            "list" => self.do_list(store, &args),
            "remove" => self.do_remove(store, &args),
            "run" => self.do_run(store, &args).await,
            "runs" => self.do_runs(store, &args),
            "update" => self.do_update(store, &args),
            other => Ok(ToolResult::err(format!("未知的 action: {other}"))),
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

impl CronTool {
    // ── add ──
    async fn do_add(&self, store: &CronScheduler, args: &Value) -> Result<ToolResult> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 name"))?;
        let schedule = args
            .get("schedule")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 schedule"))?;
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 command"))?;
        match store.add_job(CronJob::new(name, schedule, command)) {
            Ok(id) => Ok(ToolResult::ok(format!(
                "定时任务已创建: id={id}, name={name}, schedule={schedule}"
            ))),
            Err(e) => Ok(ToolResult::err(format!("创建失败: {e}"))),
        }
    }

    // ── list (支持 filter: all/enabled/disabled) ──
    fn do_list(&self, store: &CronScheduler, args: &Value) -> Result<ToolResult> {
        let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("all");
        match store.list_jobs() {
            Ok(jobs) => {
                let filtered: Vec<_> = jobs
                    .into_iter()
                    .filter(|j| match filter {
                        "enabled" => j.enabled,
                        "disabled" => !j.enabled,
                        _ => true,
                    })
                    .collect();
                if filtered.is_empty() {
                    return Ok(ToolResult::ok("当前没有定时任务"));
                }
                let mut lines = vec![format!(
                    "定时任务 ({} 个, filter={filter}):",
                    filtered.len()
                )];
                for job in &filtered {
                    let st = if job.enabled { "启用" } else { "禁用" };
                    lines.push(format!(
                        "  [{}] {} | {} | {} | {}",
                        job.id, job.name, job.schedule, st, job.command
                    ));
                }
                Ok(ToolResult::ok(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::err(format!("列出失败: {e}"))),
        }
    }

    // ── remove ──
    fn do_remove(&self, store: &CronScheduler, args: &Value) -> Result<ToolResult> {
        let id = args
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 id"))?;
        match store.remove_job(id) {
            Ok(()) => Ok(ToolResult::ok(format!("定时任务 {id} 已删除"))),
            Err(e) => Ok(ToolResult::err(format!("删除失败: {e}"))),
        }
    }

    // ── run (手动执行: 读取 job, 执行 command, 记录 CronRun) ──
    async fn do_run(&self, store: &CronScheduler, args: &Value) -> Result<ToolResult> {
        let id = args
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 id"))?;

        // 获取 job
        let job = match store.get_job(id) {
            Ok(Some(j)) => j,
            Ok(None) => return Ok(ToolResult::err(format!("未找到任务 {id}"))),
            Err(e) => return Ok(ToolResult::err(format!("查询失败: {e}"))),
        };

        // 执行命令
        let started = chrono::Utc::now().timestamp();
        let output_result = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&job.command)
            .output()
            .await;
        let finished = chrono::Utc::now().timestamp();

        let (status, output_text) = match output_result {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{stdout}\n[stderr]\n{stderr}")
                };
                let truncated = if combined.len() > 10240 {
                    format!("{}...(截断)", &combined[..10240])
                } else {
                    combined
                };
                if out.status.success() {
                    ("success", truncated)
                } else {
                    ("failed", truncated)
                }
            }
            Err(e) => ("failed", format!("执行失败: {e}")),
        };

        // 记录运行历史
        let run = CronRun {
            id: 0,
            job_id: id,
            started_at: started,
            finished_at: Some(finished),
            status: status.to_string(),
            output: output_text.clone(),
        };
        let _ = store.record_run(&run);

        let result_msg = format!(
            "任务 [{}] (id={}) 手动执行: {}\n输出:\n{}",
            job.name,
            id,
            status,
            &output_text[..output_text.len().min(2000)]
        );
        if status == "success" {
            Ok(ToolResult::ok(result_msg))
        } else {
            Ok(ToolResult {
                success: false,
                output: result_msg,
                error: Some(format!("任务执行失败: {status}")),
            })
        }
    }

    // ── runs (执行历史, 支持 limit/offset/status_filter) ──
    fn do_runs(&self, store: &CronScheduler, args: &Value) -> Result<ToolResult> {
        let id = args
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 id"))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let status_filter = args
            .get("status_filter")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        match store.list_runs(id) {
            Ok(runs) => {
                // 状态过滤
                let filtered: Vec<_> = runs
                    .into_iter()
                    .filter(|r| match status_filter {
                        "success" => r.status == "success",
                        "failed" => r.status == "failed",
                        "running" => r.status == "running",
                        _ => true,
                    })
                    .collect();
                // 分页
                let paged: Vec<_> = filtered.iter().skip(offset).take(limit).collect();
                if paged.is_empty() {
                    return Ok(ToolResult::ok(format!(
                        "任务 {id} 暂无执行历史 (offset={offset})"
                    )));
                }
                let mut lines = vec![format!(
                    "任务 {id} 执行历史 (共 {} 条, 显示 {}-{}):",
                    filtered.len(),
                    offset,
                    offset + paged.len()
                )];
                for run in &paged {
                    let output = if run.output.is_empty() {
                        "(无输出)"
                    } else {
                        &run.output
                    };
                    lines.push(format!(
                        "  #{} | {} | {} | {}",
                        run.id,
                        run.started_at,
                        run.status,
                        &output[..output.len().min(200)]
                    ));
                }
                Ok(ToolResult::ok(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::err(format!("查询失败: {e}"))),
        }
    }

    // ── update ──
    fn do_update(&self, store: &CronScheduler, args: &Value) -> Result<ToolResult> {
        let id = args
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 id"))?;
        let schedule = args.get("schedule").and_then(|v| v.as_str());
        let command = args.get("command").and_then(|v| v.as_str());
        let enabled = args.get("enabled").and_then(|v| v.as_bool());
        match store.update_job(id, schedule, command, enabled) {
            Ok(true) => Ok(ToolResult::ok(format!("任务 {id} 已更新"))),
            Ok(false) => Ok(ToolResult::err(format!("未找到任务 {id}"))),
            Err(e) => Ok(ToolResult::err(format!("更新失败: {e}"))),
        }
    }
}
