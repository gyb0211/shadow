//! Shell 工具 -- 执行 shell 命令并返回输出

use agent_core::{tool_attribution, Attributable, Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

/// Shell 工具 -- 执行 shell 命令
///
/// 使用 `sh -c` 执行命令, 返回 stdout + stderr.
/// 在 ReadOnly 模式下应拒绝执行 (由 Agent 层判断).
pub struct ShellTool;

impl Attributable for ShellTool {
    tool_attribution!("shell");
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "执行 shell 命令并返回输出. 用于运行脚本、查看目录、安装包等操作."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "要执行的 shell 命令"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 command 参数"))?;

        // 使用 tokio 的异步进程执行
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("执行命令失败: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            // 成功: 返回 stdout (如果有 stderr 也附加)
            let result = if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("{stdout}\n[stderr]\n{stderr}")
            };
            Ok(ToolResult::ok(result))
        } else {
            // 失败: 返回 exit code + stderr
            let code = output.status.code().unwrap_or(-1);
            Ok(ToolResult::err(format!(
                "命令退出码: {code}\n[stdout]\n{stdout}\n[stderr]\n{stderr}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shell_echo() {
        let tool = ShellTool;
        let result = tool
            .execute(json!({"command": "echo hello"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn shell_invalid_command() {
        let tool = ShellTool;
        let result = tool
            .execute(json!({"command": "false"}))
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn shell_missing_param() {
        let tool = ShellTool;
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }
}
