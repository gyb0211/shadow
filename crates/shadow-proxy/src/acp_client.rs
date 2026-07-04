//! ACP 客户端 -- 通过 stdio JSON-RPC 与子进程 agent 通信
//!
//! 协议: JSON-RPC 2.0 over stdin/stdout
//! 典型目标: claude --acp --stdio, codex --acp --stdio

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use crate::card::AgentCard;
use crate::transport::AgentTransport;

/// ACP 子进程内部状态 (需要可变, 用 Mutex 保护)
struct AcpState {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
}

/// ACP 子进程客户端
pub struct AcpClient {
    card: AgentCard,
    state: Mutex<AcpState>,
    next_id: AtomicU64,
    workdir: Option<std::path::PathBuf>,
}

impl AcpClient {
    /// 创建 ACP 客户端 (不立即启动)
    pub fn new(name: &str, command: &str, args: Vec<String>, capabilities: Vec<String>) -> Self {
        let card = AgentCard::acp(name, command, args, capabilities);
        Self {
            card,
            state: Mutex::new(AcpState {
                child: None,
                stdin: None,
                stdout: None,
            }),
            next_id: AtomicU64::new(1),
            workdir: None,
        }
    }

    /// 设置工作目录
    pub fn with_workdir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.workdir = Some(dir.into());
        self
    }

    /// 启动子进程
    pub async fn spawn(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if state.child.is_some() {
            return Ok(()); // 已启动
        }

        let command = self.card.command.as_deref().context("ACP card 缺少 command")?;
        let args = self.card.args.clone().unwrap_or_default();

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // 环境变量白名单
        cmd.env_clear();
        for var in &["PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR"] {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        if let Some(dir) = &self.workdir {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn().with_context(|| format!("启动 ACP 进程失败: {command}"))?;
        let stdin = child.stdin.take().context("无法获取 stdin")?;
        let stdout = child.stdout.take().context("无法获取 stdout")?;

        state.child = Some(child);
        state.stdin = Some(stdin);
        state.stdout = Some(BufReader::new(stdout));

        // 初始化握手 (内联, 避免递归)
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": self.next_id.fetch_add(1, Ordering::Relaxed),
            "method": "initialize",
            "params": {
                "protocolVersion": "1.0",
                "client": { "name": "shadow-proxy", "version": "0.1.0" }
            }
        });
        let mut line = serde_json::to_string(&init_req)?;
        line.push('\n');
        state.stdin.as_mut().unwrap().write_all(line.as_bytes()).await?;
        state.stdin.as_mut().unwrap().flush().await?;
        let mut buf = String::new();
        state.stdout.as_mut().unwrap().read_line(&mut buf).await?;
        drop(state);

        Ok(())
    }

    /// 发送 JSON-RPC 请求并等待响应
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let mut line = serde_json::to_string(&req)?;
        line.push('\n');

        let mut state = self.state.lock().await;

        if state.child.is_none() {
            drop(state);
            self.spawn().await?;
            state = self.state.lock().await;
        }

        let stdin = state.stdin.as_mut().context("stdin 不可用")?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;

        let stdout = state.stdout.as_mut().context("stdout 不可用")?;
        let mut buffer = String::new();
        let n = stdout.read_line(&mut buffer).await?;

        if n == 0 {
            bail!("ACP 进程已关闭 (EOF on stdout)");
        }

        drop(state);

        let resp: Value = serde_json::from_str(buffer.trim())
            .with_context(|| format!("解析 ACP 响应失败: {}", buffer))?;

        if let Some(error) = resp.get("error") {
            bail!("ACP 错误: {}", error);
        }

        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    /// 关闭子进程
    pub async fn shutdown(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(mut child) = state.child.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        state.stdin = None;
        state.stdout = None;
        Ok(())
    }
}

#[async_trait]
impl AgentTransport for AcpClient {
    async fn chat(&self, prompt: &str) -> Result<String> {
        let result = self.request("chat", json!({
            "messages": [{
                "role": "user",
                "content": prompt
            }]
        })).await?;

        // 解析响应: { "content": "...", "role": "assistant" }
        if let Some(content) = result.get("content").and_then(|v| v.as_str()) {
            Ok(content.to_string())
        } else {
            Ok(serde_json::to_string_pretty(&result)?)
        }
    }

    async fn chat_stream(&self, prompt: &str) -> BoxStream<'_, Result<String>> {
        let result = self.chat(prompt).await;
        futures::stream::once(async move { result }).boxed()
    }

    fn card(&self) -> &AgentCard {
        &self.card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_client_creation() {
        let client = AcpClient::new(
            "claude",
            "claude",
            vec!["--acp".into(), "--stdio".into()],
            vec!["coding".into()],
        );
        assert_eq!(client.card().name, "claude");
        assert_eq!(client.card().transport, crate::card::TransportKind::Acp);
        assert_eq!(client.card().command.as_deref(), Some("claude"));
    }

    #[tokio::test]
    async fn acp_spawn_nonexistent_binary() {
        let client = AcpClient::new(
            "fake",
            "/nonexistent/binary/path",
            vec![],
            vec![],
        );
        assert!(client.spawn().await.is_err());
    }
}
