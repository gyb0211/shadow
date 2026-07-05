//! Cron 定时任务管理工具
//!
//! 统一的 Cron 管理接口, 通过 action 参数区分操作:
//! add / list / remove / run / runs / update

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

use crate::cron::{CronJob, CronScheduler};

/// Cron 管理工具
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

impl Attributable for CronTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        "cron"
    }
}

#[async_trait]
impl Tool for CronTool {
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> &str {
        "管理定时任务。支持: add(添加) / list(列出) / remove(删除) / runs(执行历史) / update(更新)"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove", "runs", "update"],
                    "description": "操作类型"
                },
                "id": { "type": "integer", "description": "任务 ID (remove/runs/update 时需要)" },
                "name": { "type": "string", "description": "任务名称 (add 时需要)" },
                "schedule": { "type": "string", "description": "cron 表达式 (如 '0 9 * * *')" },
                "command": { "type": "string", "description": "要执行的命令 (add 时需要)" },
                "enabled": { "type": "boolean", "description": "是否启用 (update 时可选)" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args.get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        let store = match &self.store {
            Some(s) => s,
            None => return Ok(ToolResult::err("未配置 CronScheduler。请先初始化 cron 持久化。")),
        };

        match action {
            "add" => {
                let name = args.get("name").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("缺少 name 参数"))?;
                let schedule = args.get("schedule").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("缺少 schedule 参数"))?;
                let command = args.get("command").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("缺少 command 参数"))?;

                let job = CronJob::new(name, schedule, command);
                match store.add_job(job) {
                    Ok(id) => Ok(ToolResult::ok(format!("定时任务已创建: id={id}, name={name}, schedule={schedule}"))),
                    Err(e) => Ok(ToolResult::err(format!("创建失败: {e}"))),
                }
            }

            "list" => {
                match store.list_jobs() {
                    Ok(jobs) => {
                        if jobs.is_empty() {
                            return Ok(ToolResult::ok("当前没有定时任务"));
                        }
                        let mut lines = vec![format!("定时任务 ({} 个):", jobs.len())];
                        for job in &jobs {
                            let status = if job.enabled { "启用" } else { "禁用" };
                            lines.push(format!("  [{}] {} | {} | {} | {}", job.id, job.name, job.schedule, status, job.command));
                        }
                        Ok(ToolResult::ok(lines.join("\n")))
                    }
                    Err(e) => Ok(ToolResult::err(format!("列出失败: {e}"))),
                }
            }

            "remove" => {
                let id = args.get("id").and_then(|v| v.as_i64())
                    .ok_or_else(|| anyhow::anyhow!("缺少 id 参数"))?;
                match store.remove_job(id) {
                    Ok(()) => Ok(ToolResult::ok(format!("定时任务 {id} 已删除"))),
                    Err(e) => Ok(ToolResult::err(format!("删除失败: {e}"))),
                }
            }

            "runs" => {
                let id = args.get("id").and_then(|v| v.as_i64())
                    .ok_or_else(|| anyhow::anyhow!("缺少 id 参数"))?;
                match store.list_runs(id) {
                    Ok(runs) => {
                        if runs.is_empty() {
                            return Ok(ToolResult::ok(format!("任务 {id} 暂无执行历史")));
                        }
                        let mut lines = vec![format!("任务 {id} 执行历史 ({} 条):", runs.len())];
                        for run in &runs {
                            let status = format!("{:?}", run.status).to_lowercase();
                            let output = if run.output.is_empty() { "(无输出)" } else { &run.output };
                            lines.push(format!("  {} | {} | {}", run.started_at, status, &output[..output.len().min(200)]));
                        }
                        Ok(ToolResult::ok(lines.join("\n")))
                    }
                    Err(e) => Ok(ToolResult::err(format!("查询失败: {e}"))),
                }
            }

            "update" => {
                let id = args.get("id").and_then(|v| v.as_i64())
                    .ok_or_else(|| anyhow::anyhow!("缺少 id 参数"))?;
                let schedule = args.get("schedule").and_then(|v| v.as_str());
                let command = args.get("command").and_then(|v| v.as_str());
                let enabled = args.get("enabled").and_then(|v| v.as_bool());

                match store.update_job(id, schedule, command, enabled) {
                    Ok(true) => Ok(ToolResult::ok(format!("任务 {id} 已更新"))),
                    Ok(false) => Ok(ToolResult::err(format!("未找到任务 {id}"))),
                    Err(e) => Ok(ToolResult::err(format!("更新失败: {e}"))),
                }
            }

            other => Ok(ToolResult::err(format!("未知的 action: {other}"))),
        }
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_metadata() {
        let tool = CronTool::new();
        assert_eq!(tool.name(), "cron");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.timeout(), Some(Duration::from_secs(10)));
    }

    #[test]
    fn test_schema() {
        let tool = CronTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("action").is_some());
        assert!(schema["properties"].get("id").is_some());
        assert!(schema["properties"].get("schedule").is_some());
    }

    #[tokio::test]
    async fn test_no_store_error() {
        let tool = CronTool::new();
        let result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("CronScheduler"));
    }

    #[tokio::test]
    async fn test_list_with_store() {
        let store = Arc::new(CronScheduler::new_in_memory().unwrap());
        store.add_job(CronJob::new("test", "0 0 9 * * *", "echo hello")).unwrap();

        let tool = CronTool::with_store(store);
        let result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("test"));
    }

    #[tokio::test]
    async fn test_add_and_remove() {
        let store = Arc::new(CronScheduler::new_in_memory().unwrap());
        let tool = CronTool::with_store(store);

        // 添加任务
        let result = tool.execute(json!({
            "action": "add",
            "name": "daily backup",
            "schedule": "0 0 2 * * *",
            "command": "tar czf backup.tar.gz data/"
        })).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("已创建"));

        // 列出任务, 获取 id
        let result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(result.output.contains("daily backup"));

        // 删除 (id=1, SQLite 第一个自增 id)
        let result = tool.execute(json!({"action": "remove", "id": 1})).await.unwrap();
        assert!(result.success);

        // 确认已删除
        let result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(result.output.contains("没有定时任务") || result.output.contains("定时任务 (0"));
    }
}
