//! Git 操作工具 -- 执行常用 Git 命令
//!
//! 支持: status / diff / log / add / commit / branch / checkout / push / pull
//! 通过 Shell 执行 git 命令, 内置安全检查

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// Git 操作工具
pub struct GitOpsTool;

/// 允许的 Git 子命令 (白名单)
const ALLOWED_COMMANDS: &[&str] = &[
    "status", "diff", "log", "add", "commit", "branch",
    "checkout", "push", "pull", "merge", "rebase", "fetch",
    "stash", "tag", "show", "blame", "restore", "reset --soft",
    "reset --mixed", "reset --hard", "rev-parse", "remote",
];

impl GitOpsTool {
    pub fn new() -> Self {
        Self
    }

    /// 检查 Git 子命令是否在白名单中
    fn is_allowed(subcommand: &str) -> bool {
        let sub = subcommand.trim();
        ALLOWED_COMMANDS.iter().any(|&allowed| {
            sub == allowed || sub.starts_with(allowed)
        })
    }
}

impl Default for GitOpsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Attributable for GitOpsTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        "git_ops"
    }
}

#[async_trait]
impl Tool for GitOpsTool {
    fn name(&self) -> &str {
        "git_ops"
    }

    fn description(&self) -> &str {
        "执行 Git 操作。支持 status/diff/log/add/commit/branch/checkout/push/pull 等常用命令。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Git 子命令 (如 status, diff, log --oneline -5, add -A, commit -m \"msg\")"
                },
                "args": {
                    "type": "string",
                    "description": "命令参数 (已包含在 command 中, 此字段保留)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let command = args.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 command 参数"))?;

        // 安全: 检查子命令是否在白名单中
        if !Self::is_allowed(command) {
            return Ok(ToolResult::err(format!(
                "不允许的 Git 命令: {command}\n允许的命令: {}",
                ALLOWED_COMMANDS.join(", ")
            )));
        }

        // 构建 git 命令
        let full_cmd = format!("git {command}");

        // 执行
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&full_cmd)
            .output()
            .await;

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let exit_code = out.status.code().unwrap_or(-1);

                if exit_code == 0 {
                    let result = if stdout.is_empty() && stderr.is_empty() {
                        format!("$ {full_cmd}\n(无输出)")
                    } else if stdout.is_empty() {
                        format!("$ {full_cmd}\n{stderr}")
                    } else {
                        format!("$ {full_cmd}\n{stdout}")
                    };
                    Ok(ToolResult::ok(result))
                } else {
                    let err_msg = if stderr.is_empty() {
                        format!("Git 命令失败 (exit {exit_code})")
                    } else {
                        format!("Git 命令失败 (exit {exit_code}): {stderr}")
                    };
                    Ok(ToolResult::err(err_msg))
                }
            }
            Err(e) => Ok(ToolResult::err(format!("执行失败: {e}"))),
        }
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    fn requires_approval(&self) -> bool {
        // push / reset --hard 需要审批, 但 trait 层面统一设为 false
        // 实际审批由 agent 的 AutonomyLevel 控制
        false
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
    fn test_is_allowed() {
        assert!(GitOpsTool::is_allowed("status"));
        assert!(GitOpsTool::is_allowed("diff"));
        assert!(GitOpsTool::is_allowed("log --oneline -5"));
        assert!(GitOpsTool::is_allowed("add -A"));
        assert!(GitOpsTool::is_allowed("commit -m \"test\""));
        assert!(GitOpsTool::is_allowed("push origin main"));
        assert!(GitOpsTool::is_allowed("reset --hard HEAD~1"));

        // 危险命令
        assert!(!GitOpsTool::is_allowed("config user.email"));
        assert!(!GitOpsTool::is_allowed("filter-branch"));
        assert!(!GitOpsTool::is_allowed("rm -rf /"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = GitOpsTool::new();
        assert_eq!(tool.name(), "git_ops");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.timeout(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_schema() {
        let tool = GitOpsTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("command").is_some());
    }

    #[tokio::test]
    async fn test_disallowed_command() {
        let tool = GitOpsTool::new();
        let result = tool.execute(json!({"command": "filter-branch"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("不允许"));
    }

    #[tokio::test]
    async fn test_missing_command() {
        let tool = GitOpsTool::new();
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }
}
