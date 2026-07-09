//! Shell 工具 -- 执行 shell 命令并返回输出
//!
//! 安全措施 (通过 [`SecurityPolicy`] 注入):
//! - 命令黑名单检测 (危险命令直接拒绝)
//! - 工作目录限制 (可选, 防止越权访问)
//! - 环境变量过滤 (只传递白名单变量, 防止泄露密钥)

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_core::{Attributable, Tool, ToolResult, tool_attribution};
use std::sync::Arc;
use std::time::Duration;

use crate::security::SecurityPolicy;

/// stdout 最大输出字节数 (10KB), 超过则截断
const MAX_STDOUT_BYTES: usize = 10 * 1024;

/// Shell 工具 -- 执行 shell 命令
///
/// 使用 `sh -c` 执行命令, 返回 stdout + stderr.
/// 在 ReadOnly 模式下应拒绝执行 (由 Agent 层判断).
///
/// 持有 [`SecurityPolicy`] 用于命令黑名单检测、工作目录限制和环境变量过滤.
/// 使用 [`ShellTool::default()`] 获取默认策略, 或 [`ShellTool::new()`] 自定义.
pub struct ShellTool {
    /// 安全策略 (黑名单 + 环境变量过滤 + 工作目录)
    security: Arc<SecurityPolicy>,
}

impl ShellTool {
    /// 创建 Shell 工具, 使用指定的安全策略
    pub fn new(security: SecurityPolicy) -> Self {
        Self {
            security: Arc::new(security),
        }
    }

    /// 获取安全策略引用
    pub fn security(&self) -> &SecurityPolicy {
        &self.security
    }
}

impl Default for ShellTool {
    /// 默认使用 [`SecurityPolicy::new()`] (无工作目录限制)
    fn default() -> Self {
        Self::new(SecurityPolicy::new())
    }
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

        // 1. 危险命令检测 -- 命中黑名单直接返回错误, 不执行
        if let Some(pattern) = self.security.is_blocked(command) {
            return Ok(ToolResult::err(format!(
                "拒绝执行危险命令: 命中黑名单规则 '{pattern}'"
            )));
        }

        // 2. 构建命令, 应用安全策略 (工作目录 + 环境变量过滤)
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);

        // 设置工作目录 (如果策略指定)
        if let Some(workspace) = self.security.workspace() {
            cmd.current_dir(workspace);
        }

        // 环境变量过滤: 清空继承的环境, 只设置白名单中的变量
        // 防止子进程获取到 API_KEY / SECRET 等敏感变量
        cmd.env_clear();
        let env_vars: Vec<(String, String)> = std::env::vars().collect();
        for (key, value) in self.security.filter_env(&env_vars) {
            cmd.env(key, value);
        }

        let output = cmd
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
