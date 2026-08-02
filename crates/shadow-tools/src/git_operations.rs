//! Git 操作工具 -- 结构化 Git 操作
//!
//! 支持: status, diff, log, branch, add, commit, checkout, stash
//!
//! 参考 ZeroClaw git_operations.rs，简化实现核心操作

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use shadow_core::{Tool, ToolResult};
use std::path::PathBuf;

/// Git 操作工具
pub struct GitOperationsTool {
    /// 工作区根目录
    workspace: PathBuf,
}

impl GitOperationsTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// 运行 git 命令
    async fn run_git(&self, args: &[&str], working_dir: &std::path::Path) -> Result<String> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(working_dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .context("Failed to execute git")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("{}", stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// 检查是否在 git 仓库中
    async fn ensure_git_repo(&self, dir: &std::path::Path) -> Result<()> {
        let output = tokio::process::Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await?;

        if !output.success() {
            anyhow::bail!("Not a git repository: {}", dir.display());
        }
        Ok(())
    }

    /// 解析工作目录
    fn resolve_working_dir(&self, path: Option<&str>) -> PathBuf {
        match path {
            Some(p) if !p.is_empty() => {
                if std::path::Path::new(p).is_absolute() {
                    PathBuf::from(p)
                } else {
                    self.workspace.join(p)
                }
            }
            _ => self.workspace.clone(),
        }
    }

    // ── 读操作 ──────────────────────────────────────────────────

    async fn git_status(&self, dir: &std::path::Path) -> Result<ToolResult> {
        let output = self.run_git(&["status", "--porcelain=v2", "--branch"], dir).await?;

        let mut branch = String::new();
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();

        for line in output.lines() {
            if let Some(b) = line.strip_prefix("# branch.head ") {
                branch = b.to_string();
            } else if let Some(rest) = line.strip_prefix("1 ") {
                let parts: Vec<&str> = rest.splitn(3, ' ').collect();
                if parts.len() >= 3 {
                    let xy = parts[1];
                    let path = parts[2];
                    if let Some(c) = xy.chars().next() {
                        if c != '.' && c != ' ' {
                            staged.push(json!({"path": path, "status": c}));
                        }
                    }
                    if let Some(c) = xy.chars().nth(1) {
                        if c != '.' && c != ' ' {
                            unstaged.push(json!({"path": path, "status": c}));
                        }
                    }
                }
            } else if let Some(rest) = line.strip_prefix("? ") {
                untracked.push(rest.to_string());
            }
        }

        let result = json!({
            "branch": branch,
            "staged": staged,
            "unstaged": unstaged,
            "untracked": untracked,
            "clean": staged.is_empty() && unstaged.is_empty() && untracked.is_empty(),
        });

        Ok(ToolResult::ok(serde_json::to_string_pretty(&result).unwrap_or_default()))
    }

    async fn git_diff(&self, args: &Value, dir: &std::path::Path) -> Result<ToolResult> {
        let files = args.get("files").and_then(|v| v.as_str()).unwrap_or(".");
        let cached = args.get("cached").and_then(|v| v.as_bool()).unwrap_or(false);

        let mut git_args = vec!["diff", "--unified=3"];
        if cached {
            git_args.push("--cached");
        }
        git_args.push("--");
        git_args.push(files);

        let output = self.run_git(&git_args, dir).await?;

        if output.trim().is_empty() {
            return Ok(ToolResult::ok("No changes"));
        }

        Ok(ToolResult::ok(output))
    }

    async fn git_log(&self, args: &Value, dir: &std::path::Path) -> Result<ToolResult> {
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

        let format = "--pretty=format:%H|%h|%an|%ad|%s";
        let output = self
            .run_git(&["log", format, "--date=short", "-n", &limit.to_string()], dir)
            .await?;

        let commits: Vec<Value> = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(5, '|').collect();
                if parts.len() == 5 {
                    Some(json!({
                        "hash": parts[0],
                        "short_hash": parts[1],
                        "author": parts[2],
                        "date": parts[3],
                        "message": parts[4],
                    }))
                } else {
                    None
                }
            })
            .collect();

        let result = json!({ "commits": commits });
        Ok(ToolResult::ok(serde_json::to_string_pretty(&result).unwrap_or_default()))
    }

    async fn git_branch(&self, dir: &std::path::Path) -> Result<ToolResult> {
        let output = self.run_git(&["branch", "--format=%(refname:short)|%(objectname:short)|%(committerdate:short)"], dir).await?;

        let branches: Vec<Value> = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '|').collect();
                if parts.len() >= 2 {
                    Some(json!({
                        "name": parts[0],
                        "hash": parts[1],
                        "last_commit": parts.get(2).unwrap_or(&""),
                    }))
                } else {
                    None
                }
            })
            .collect();

        // 获取当前分支
        let current = self.run_git(&["branch", "--show-current"], dir).await?;
        let result = json!({
            "current": current.trim(),
            "branches": branches,
        });

        Ok(ToolResult::ok(serde_json::to_string_pretty(&result).unwrap_or_default()))
    }

    // ── 写操作 ──────────────────────────────────────────────────

    async fn git_add(&self, args: &Value, dir: &std::path::Path) -> Result<ToolResult> {
        let paths = args.get("paths").and_then(|v| v.as_str()).unwrap_or(".");

        let output = self.run_git(&["add", paths], dir).await?;
        let msg = if output.is_empty() { "Files staged" } else { &output };
        Ok(ToolResult::ok(msg.to_string()))
    }

    async fn git_commit(&self, args: &Value, dir: &std::path::Path) -> Result<ToolResult> {
        let message = args.get("message").and_then(|v| v.as_str());
        let message = match message {
            Some(m) if !m.is_empty() => m,
            _ => return Ok(ToolResult::err("Commit message is required")),
        };

        let output = self.run_git(&["commit", "-m", message], dir).await?;
        Ok(ToolResult::ok(output.trim().to_string()))
    }

    async fn git_checkout(&self, args: &Value, dir: &std::path::Path) -> Result<ToolResult> {
        let branch = args.get("branch").and_then(|v| v.as_str());
        let branch = match branch {
            Some(b) if !b.is_empty() => b,
            _ => return Ok(ToolResult::err("Branch name is required")),
        };

        match self.run_git(&["checkout", branch], dir).await {
            Ok(output) => Ok(ToolResult::ok(output.trim().to_string())),
            Err(e) => Ok(ToolResult::err(format!("git checkout failed: {e}"))),
        }
    }

    async fn git_stash(&self, args: &Value, dir: &std::path::Path) -> Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("push");

        match action {
            "push" => {
                let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("auto-stash");
                let output = self.run_git(&["stash", "push", "-m", message], dir).await?;
                Ok(ToolResult::ok(output.trim().to_string()))
            }
            "pop" => {
                let output = self.run_git(&["stash", "pop"], dir).await?;
                Ok(ToolResult::ok(output.trim().to_string()))
            }
            "list" => {
                let output = self.run_git(&["stash", "list"], dir).await?;
                Ok(ToolResult::ok(output))
            }
            "drop" => {
                let index = args.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let stash_ref = format!("stash@{{{}}}", index);
                let output = self.run_git(&["stash", "drop", &stash_ref], dir).await?;
                Ok(ToolResult::ok(output.trim().to_string()))
            }
            other => Ok(ToolResult::err(format!("Unknown stash action: {other}"))),
        }
    }
}

shadow_core::tool_attribution!(GitOperationsTool, shadow_core::ToolKind::Shell);

#[async_trait]
impl Tool for GitOperationsTool {
    fn name(&self) -> &str {
        "git_operations"
    }

    fn description(&self) -> &str {
        "Perform structured Git operations: status, diff, log, branch, add, commit, checkout, stash. \
         Returns structured JSON output for read operations."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["status", "diff", "log", "branch", "add", "commit", "checkout", "stash"],
                    "description": "Git operation to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (for 'commit') or stash message (for 'stash push')"
                },
                "paths": {
                    "type": "string",
                    "description": "File paths to stage (for 'add'). Default: '.'"
                },
                "branch": {
                    "type": "string",
                    "description": "Branch name (for 'checkout')"
                },
                "files": {
                    "type": "string",
                    "description": "Files to diff (for 'diff'). Default: '.'"
                },
                "cached": {
                    "type": "boolean",
                    "description": "Show staged changes only (for 'diff')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of log entries (for 'log'). Default: 10"
                },
                "action": {
                    "type": "string",
                    "enum": ["push", "pop", "list", "drop"],
                    "description": "Stash action (for 'stash')"
                },
                "index": {
                    "type": "integer",
                    "description": "Stash index (for 'stash drop')"
                },
                "path": {
                    "type": "string",
                    "description": "Subdirectory path within workspace. Default: workspace root."
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let operation = match args.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => return Ok(ToolResult::err("Missing required parameter: operation")),
        };

        let path = args.get("path").and_then(|v| v.as_str());
        let working_dir = self.resolve_working_dir(path);

        // 检查 git 仓库
        if let Err(e) = self.ensure_git_repo(&working_dir).await {
            return Ok(ToolResult::err(format!("{e}")));
        }

        match operation {
            "status" => self.git_status(&working_dir).await,
            "diff" => self.git_diff(&args, &working_dir).await,
            "log" => self.git_log(&args, &working_dir).await,
            "branch" => self.git_branch(&working_dir).await,
            "add" => self.git_add(&args, &working_dir).await,
            "commit" => self.git_commit(&args, &working_dir).await,
            "checkout" => self.git_checkout(&args, &working_dir).await,
            "stash" => self.git_stash(&args, &working_dir).await,
            other => Ok(ToolResult::err(format!("Unknown operation: {other}"))),
        }
    }
}