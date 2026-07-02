//! Shell 工具 -- 执行 shell 命令并返回输出

use shadow_core::{tool_attribution, Attributable, Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

/// stdout 最大输出字节数 (10KB), 超过则截断
const MAX_STDOUT_BYTES: usize = 10 * 1024;

/// 危险命令黑名单 -- 匹配到任意一条则直接拒绝执行
const DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf /",   // 递归删除根目录
    "mkfs",       // 格式化文件系统
    "dd if=",     // 磁盘镜像写入
    "> /dev/sd",  // 直接写入块设备
];

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

    /// Shell 工具需要审批 -- 涉及系统操作, 危险性较高
    fn requires_approval(&self) -> bool {
        true
    }

    /// 超时 30 秒, 覆盖默认值
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 command 参数"))?;

        // 危险命令检测 -- 命中黑名单直接返回错误, 不执行
        if let Some(pattern) = detect_dangerous(command) {
            return Ok(ToolResult::err(format!(
                "拒绝执行危险命令: 命中黑名单规则 '{pattern}'"
            )));
        }

        // 使用 tokio 的异步进程执行
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("执行命令失败: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // 截断超长 stdout (超过 10KB)
        let stdout = truncate_stdout(&stdout);

        if output.status.success() {
            // 成功: 返回 stdout (如果有 stderr 也附加)
            let result = if stderr.is_empty() {
                stdout
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

/// 检测命令是否命中危险黑名单
///
/// 返回命中的规则字符串 (用于错误提示), 未命中返回 None
fn detect_dangerous(command: &str) -> Option<&'static str> {
    DANGEROUS_PATTERNS
        .iter()
        .find(|pattern| command.contains(*pattern))
        .copied()
}

/// 截断超长 stdout -- 超过 MAX_STDOUT_BYTES 时只保留前部分并附加提示
fn truncate_stdout(stdout: &str) -> String {
    if stdout.len() <= MAX_STDOUT_BYTES {
        return stdout.to_string();
    }

    // 在字节边界安全截断 (避免截断 UTF-8 字符的中间)
    let mut end = MAX_STDOUT_BYTES;
    while end > 0 && !stdout.is_char_boundary(end) {
        end -= 1;
    }

    format!(
        "{}\n\n[stdout 已截断, 显示前 {end} / 共 {} 字节]",
        &stdout[..end],
        stdout.len()
    )
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

    // ---- 危险命令测试 ----

    #[tokio::test]
    async fn shell_dangerous_rm_rf_root() {
        let tool = ShellTool;
        let result = tool
            .execute(json!({"command": "rm -rf /"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("危险命令"));
    }

    #[tokio::test]
    async fn shell_dangerous_rm_rf_subdir() {
        // rm -rf /home 也应被拦截 (前缀匹配 "rm -rf /")
        let tool = ShellTool;
        let result = tool
            .execute(json!({"command": "rm -rf /home/user"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("危险命令"));
    }

    #[tokio::test]
    async fn shell_dangerous_mkfs() {
        let tool = ShellTool;
        let result = tool
            .execute(json!({"command": "mkfs.ext4 /dev/sda1"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("危险命令"));
    }

    #[tokio::test]
    async fn shell_dangerous_dd() {
        let tool = ShellTool;
        let result = tool
            .execute(json!({"command": "dd if=/dev/zero of=/dev/sda"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("危险命令"));
    }

    #[tokio::test]
    async fn shell_dangerous_dev_sd() {
        let tool = ShellTool;
        let result = tool
            .execute(json!({"command": "echo bad > /dev/sda"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("危险命令"));
    }

    // ---- 审批与超时测试 ----

    #[test]
    fn shell_requires_approval() {
        let tool = ShellTool;
        assert!(tool.requires_approval());
    }

    #[test]
    fn shell_timeout_is_30s() {
        let tool = ShellTool;
        assert_eq!(tool.timeout(), Some(Duration::from_secs(30)));
    }

    // ---- 输出截断测试 ----

    #[tokio::test]
    async fn shell_stdout_truncation() {
        let tool = ShellTool;
        // 输出约 20KB, 超过 10KB 限制
        let result = tool
            .execute(json!({"command": "yes x | head -c 20480"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("[stdout 已截断"));
    }
}
