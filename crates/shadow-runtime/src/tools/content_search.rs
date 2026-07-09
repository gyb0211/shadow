//! ContentSearch 工具 -- 在文件内容中搜索文本 (类似 grep)

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_core::{ Tool, ToolResult, tool_attribution, ToolKind};
use std::path::{Path, PathBuf};

/// ContentSearch 工具 -- 递归遍历目录, 逐行匹配文本模式
///
/// 返回格式: `file:line: content` (类似 grep -rn)
/// 仅搜索文本文件, 跳过二进制文件和超大文件.
pub struct ContentSearchTool;

#[async_trait]
impl Tool for ContentSearchTool {
    fn name(&self) -> &str {
        "content_search"
    }

    fn description(&self) -> &str {
        "在文件内容中搜索文本。类似 grep。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "要搜索的文本模式"
                },
                "path": {
                    "type": "string",
                    "description": "搜索根目录",
                    "default": "."
                },
                "max_results": {
                    "type": "integer",
                    "description": "最大返回结果数",
                    "default": 20
                }
            },
            "required": ["pattern"]
        })
    }
    
    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 pattern 参数"))?;

        let base_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(20);

        // 使用 spawn_blocking 避免阻塞异步运行时
        let pattern_owned = pattern.to_string();
        let base = PathBuf::from(base_path);
        let results = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            search_content(&base, &pattern_owned, max_results)
        })
        .await
        .map_err(|e| anyhow::anyhow!("搜索任务执行失败: {e}"))??;

        if results.is_empty() {
            return Ok(ToolResult::ok(format!("未找到包含 '{pattern}' 的内容")));
        }

        Ok(ToolResult::ok(results.join("\n")))
    }
}

/// 递归搜索文件内容, 返回 "file:line: content" 格式的匹配列表
fn search_content(base: &Path, pattern: &str, max: usize) -> Result<Vec<String>> {
    let mut results = Vec::new();

    fn walk(dir: &Path, _base: &Path, pattern: &str, results: &mut Vec<String>, max: usize) {
        if results.len() >= max {
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            if results.len() >= max {
                return;
            }

            let path = entry.path();

            if path.is_dir() {
                // 跳过隐藏目录 (如 .git, .cargo)
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && name.starts_with('.')
                {
                    continue;
                }
                walk(&path, _base, pattern, results, max);
            } else if path.is_file() {
                // 跳过隐藏文件
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && name.starts_with('.')
                {
                    continue;
                }

                // 跳过超大文件 (超过 1MB)
                if let Ok(meta) = entry.metadata()
                    && meta.len() > 1024 * 1024
                {
                    continue;
                }

                search_file(&path, pattern, results, max);
            }
        }
    }

    walk(base, base, pattern, &mut results, max);
    Ok(results)
}

/// 在单个文件中搜索匹配行
fn search_file(path: &Path, pattern: &str, results: &mut Vec<String>, max: usize) {
    let Ok(content) = std::fs::read_to_string(path) else {
        // 读取失败 (可能是二进制文件), 跳过
        return;
    };

    let path_str = path.to_string_lossy();
    for (line_num, line) in content.lines().enumerate() {
        if results.len() >= max {
            return;
        }
        if line.contains(pattern) {
            // 截断过长的行
            let display_line = if line.len() > 200 {
                format!("{}...", &line[..line.floor_char_boundary(200)])
            } else {
                line.to_string()
            };
            results.push(format!("{}:{}: {}", path_str, line_num + 1, display_line));
        }
    }
}
