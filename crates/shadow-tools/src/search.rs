//! 搜索工具 -- 文件名搜索 (glob) + 内容搜索 (regex)
//!
//! 设计参考 ZeroClaw GlobSearchTool + ContentSearchTool：
//! - GlobSearchTool: 用 glob 模式匹配文件路径
//! - ContentSearchTool: 用正则表达式搜索文件内容

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;
use shadow_core::{Tool, ToolResult};

/// 最大返回结果数
const MAX_RESULTS: usize = 100;

// ── Glob 文件搜索 ─────────────────────────────────────

pub struct GlobSearchTool {
    workspace: PathBuf,
}

impl GlobSearchTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }
}

shadow_core::tool_attribution!(GlobSearchTool, shadow_core::ToolKind::Search);

#[async_trait]
impl Tool for GlobSearchTool {
    fn name(&self) -> &str {
        "glob_search"
    }

    fn description(&self) -> &str {
        "Find files by glob pattern (e.g. '**/*.rs', 'src/**/*.py'). \
         Returns matching file paths relative to workspace. \
         Results are sorted by modification time (newest first)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern. Supports **, *, ?. Example: '**/*.rs'"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(ToolResult::err("Missing or empty required parameter: pattern")),
        };

        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            self.workspace.join(pattern).to_string_lossy().to_string()
        };

        // 用 glob crate 匹配
        let mut entries: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        for entry in glob::glob(&full_pattern).map_err(|e| anyhow::anyhow!("Invalid glob: {e}"))? {
            match entry {
                Ok(path) if path.is_file() => {
                    let mtime = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    entries.push((path, mtime));
                }
                _ => {}
            }
        }

        if entries.is_empty() {
            return Ok(ToolResult::ok("No files found."));
        }

        // 按修改时间降序排序
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(MAX_RESULTS);

        // 输出相对于 workspace 的路径
        let mut output = String::new();
        for (path, _) in &entries {
            let display = path
                .strip_prefix(&self.workspace)
                .unwrap_or(path)
                .display()
                .to_string();
            output.push_str(&display);
            output.push('\n');
        }

        output.push_str(&format!("\n({} files found)", entries.len()));
        Ok(ToolResult::ok(output))
    }
}

// ── 内容搜索 ──────────────────────────────────────────

pub struct ContentSearchTool {
    workspace: PathBuf,
}

impl ContentSearchTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }
}

shadow_core::tool_attribution!(ContentSearchTool, shadow_core::ToolKind::Search);

#[async_trait]
impl Tool for ContentSearchTool {
    fn name(&self) -> &str {
        "content_search"
    }

    fn description(&self) -> &str {
        "Search file contents by regex pattern. \
         Searches all text files under workspace recursively. \
         Returns matching lines with file path and line number. \
         Up to 100 results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for."
                },
                "file_glob": {
                    "type": "string",
                    "description": "Optional glob to filter files (e.g. '*.rs'). Default: all files."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(ToolResult::err("Missing or empty required parameter: pattern")),
        };

        let file_glob = args.get("file_glob").and_then(|v| v.as_str());

        let re = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("Invalid regex: {e}"))),
        };

        // 递归遍历 workspace
        let mut results: Vec<(String, usize, String)> = Vec::new(); // (file, line_num, line_content)
        let mut visited = 0u32;
        let mut matched = 0u32;

        let mut stack = vec![self.workspace.clone()];
        while let Some(dir) = stack.pop() {
            if results.len() >= MAX_RESULTS {
                break;
            }
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                // 跳过隐藏文件和常见忽略目录
                if name_str.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    if matches!(
                        name_str.as_ref(),
                        "node_modules" | "target" | ".git" | "__pycache__" | "dist" | "build"
                    ) {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }

                if path.is_file() {
                    // file_glob 过滤
                    if let Some(glob_pat) = file_glob {
                        if let Ok(glob_matcher) = glob::Pattern::new(glob_pat) {
                            if !glob_matcher.matches(&name_str) {
                                continue;
                            }
                        }
                    }

                    visited += 1;

                    // 读取文件内容（限制大小，跳过二进制）
                    let bytes = match tokio::fs::read(&path).await {
                        Ok(b) if b.len() < 512 * 1024 => b, // 跳过 >512KB 的文件
                        _ => continue,
                    };

                    // 二进制检测
                    let check_len = bytes.len().min(1024);
                    if bytes[..check_len].contains(&0) {
                        continue;
                    }

                    let content = String::from_utf8_lossy(&bytes);
                    let rel_path = path
                        .strip_prefix(&self.workspace)
                        .unwrap_or(&path)
                        .display()
                        .to_string();

                    for (i, line) in content.lines().enumerate() {
                        if re.is_match(line) {
                            results.push((rel_path.clone(), i + 1, line.trim().to_string()));
                            matched += 1;
                            if results.len() >= MAX_RESULTS {
                                break;
                            }
                        }
                    }
                }
            }
        }

        if results.is_empty() {
            return Ok(ToolResult::ok(format!(
                "No matches found (searched {visited} files)."
            )));
        }

        let mut output = String::new();
        for (file, line_num, line) in &results {
            output.push_str(&format!("{file}:{line_num}: {line}\n"));
        }
        output.push_str(&format!(
            "\n({} matches in {} files, searched {visited} files)",
            results.len(),
            results.iter().map(|(f, _, _)| f).collect::<std::collections::HashSet<_>>().len(),
        ));

        Ok(ToolResult::ok(output))
    }
}

