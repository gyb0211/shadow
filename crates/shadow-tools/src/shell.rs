//! Shell 工具 -- 执行命令行命令
//!
//! 功能：
//! - 执行 shell 命令并捕获输出
//! - 支持超时控制
//! - 自动过滤敏感环境变量（SECRET、TOKEN、PASSWORD 等）
//! - 输出截断（防止过大的输出）

use anyhow::{Context, Result};
use async_trait::async_trait;
use shadow_core::{Tool, ToolResult};
use std::time::Duration;

/// 需要过滤的环境变量关键词（黑名单）
const SECRET_KEYWORDS: &[&str] = &[
    "API_KEY", "SECRET", "TOKEN", "PASSWORD", "CREDENTIAL",
    "PRIVATE_KEY", "ACCESS_KEY", "AUTH", "SESSION",
];

/// 清理环境变量，移除包含敏感关键词的变量
fn clean_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(key, _)| {
            // 检查 key 是否包含敏感关键词
            let key_upper = key.to_uppercase();
            !SECRET_KEYWORDS.iter().any(|keyword| key_upper.contains(keyword))
        })
        .collect()
}

/// Shell 工具
pub struct ShellTool;

shadow_core::tool_attribution!(ShellTool, shadow_core::ToolKind::Shell);

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute shell commands and return output. Supports timeout and output truncation. Automatically filters sensitive environment variables (API_KEY, TOKEN, PASSWORD, etc.)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds, default 30",
                    "default": 30
                },
                "max_output": {
                    "type": "integer",
                    "description": "Maximum output length in bytes, default 1048576",
                    "default": 1048576
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        // 解析参数
        let command = args.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少必需参数: command"))?;

        let timeout_secs = args.get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);
        let max_output = args.get("max_output")
            .and_then(|v| v.as_u64())
            .unwrap_or(1024 * 1024) as usize;

        let timeout = Duration::from_secs(timeout_secs);

        // 构建命令
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
           .arg(command);

        // 设置清理后的环境变量
        for (key, value) in clean_env() {
            cmd.env(&key, &value);
        }

        // 执行命令并等待结果
        let output_future = cmd.output();
        let output = match tokio::time::timeout(timeout, output_future).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Ok(ToolResult::err(format!("命令执行失败: {}", e)));
            }
            Err(_) => {
                return Ok(ToolResult::err(format!("命令执行超时（{}秒）", timeout_secs)));
            }
        };

        // 处理输出
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        // 截断输出
        let stdout = if stdout.len() > max_output {
            format!("{}... (输出已截断，超过 {} 字节)", 
                   &stdout[..max_output], max_output)
        } else {
            stdout.to_string()
        };

        let stderr = if stderr.len() > max_output {
            format!("{}... (输出已截断，超过 {} 字节)", 
                   &stderr[..max_output], max_output)
        } else {
            stderr.to_string()
        };

        // 构建响应
        let output_str = format!(
            "退出码: {}\n标准输出:\n{}\n标准错误:\n{}",
            exit_code, stdout, stderr
        );

        if output.status.success() {
            Ok(ToolResult::ok(output_str))
        } else {
            // 失败时，详细信息放到 error 字段，output 为空
            Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(output_str),
            })
        }
    }
}

