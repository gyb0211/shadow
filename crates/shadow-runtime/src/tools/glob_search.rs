//! GlobSearch 工具 -- 按文件名模式搜索文件

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_core::{Attributable, Tool, ToolResult, tool_attribution};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 单次 glob 搜索最大返回文件数
const MAX_RESULTS: usize = 100;

/// GlobSearch 工具 -- 按文件名通配符搜索文件
///
/// 支持简单 glob 模式:
/// - `*` 匹配单层任意字符 (不含路径分隔符)
/// - `**` 匹配任意层级目录
/// - `?` 匹配单个字符
///
/// 例如: `**/*.rs` 匹配所有 Rust 文件, `src/*.ts` 匹配 src 下的 ts 文件
pub struct GlobSearchTool;

impl Attributable for GlobSearchTool {
    tool_attribution!("glob_search");
}

#[async_trait]
impl Tool for GlobSearchTool {
    fn name(&self) -> &str {
        "glob_search"
    }

    fn description(&self) -> &str {
        "按文件名模式搜索文件。支持 * 和 ** 通配符。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "文件名模式, 如 **/*.rs"
                },
                "path": {
                    "type": "string",
                    "description": "搜索根目录",
                    "default": "."
                }
            },
            "required": ["pattern"]
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 pattern 参数"))?;

        let base_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        // 递归遍历目录, 收集匹配的文件 -- 使用 spawn_blocking 避免阻塞异步运行时
        let base = PathBuf::from(base_path);
        let pattern_owned = pattern.to_string();
        let base_clone = base.clone();
        let results = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            collect_glob_matches(&base_clone, &pattern_owned, MAX_RESULTS)
        })
        .await
        .map_err(|e| anyhow::anyhow!("搜索任务执行失败: {e}"))??;

        if results.is_empty() {
            return Ok(ToolResult::ok(format!("未找到匹配 '{pattern}' 的文件")));
        }

        Ok(ToolResult::ok(results.join("\n")))
    }
}

/// 递归遍历目录, 收集匹配 glob 模式的文件路径
///
/// 简化实现: 将 glob 模式转为正则, 逐个文件匹配相对路径.
fn collect_glob_matches(base: &Path, pattern: &str, max: usize) -> Result<Vec<String>> {
    let regex = glob_to_regex(pattern);
    let mut results = Vec::new();

    fn walk(dir: &Path, base: &Path, regex: &regex::Regex, results: &mut Vec<String>, max: usize) {
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
            let is_dir = path.is_dir();

            // 计算相对于 base 的路径, 用于匹配
            let rel = path.strip_prefix(base).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");

            if !is_dir && regex.is_match(&rel_str) {
                results.push(path.to_string_lossy().into_owned());
            }

            // 递归进入子目录
            if is_dir {
                walk(&path, base, regex, results, max);
            }
        }
    }

    walk(base, base, &regex, &mut results, max);
    Ok(results)
}

/// 将简单 glob 模式转为正则表达式
///
/// - `*` -> `[^/]*` (匹配单层, 不含路径分隔符)
/// - `**` -> `.*` (匹配任意层级)
/// - `?` -> `[^/]` (匹配单个字符)
/// - 其他字符按字面量匹配 (转义正则特殊字符)
fn glob_to_regex(pattern: &str) -> regex::Regex {
    let mut out = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // ** 匹配任意层级
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            out.push_str(".*");
            i += 2;
            // 跳过 ** 后的 / (因为 .* 已包含)
            if i < chars.len() && chars[i] == '/' {
                i += 1;
            }
        } else if chars[i] == '*' {
            out.push_str("[^/]*");
            i += 1;
        } else if chars[i] == '?' {
            out.push_str("[^/]");
            i += 1;
        } else {
            // 转义正则特殊字符
            out.push_str(&regex::escape(&chars[i].to_string()));
            i += 1;
        }
    }

    // 锚定到完整匹配
    regex::Regex::new(&format!("^{out}$")).unwrap_or_else(|_| {
        // 如果模式无效, 匹配空 (永不命中)
        regex::Regex::new(r"$^").unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_to_regex_double_star() {
        let re = glob_to_regex("**/*.rs");
        assert!(re.is_match("src/main.rs"));
        assert!(re.is_match("a/b/c.rs"));
        assert!(!re.is_match("src/main.txt"));
    }

    #[test]
    fn glob_to_regex_single_star() {
        let re = glob_to_regex("*.rs");
        assert!(re.is_match("main.rs"));
        assert!(!re.is_match("src/main.rs"));
    }

    #[tokio::test]
    async fn glob_search_finds_rust_files() {
        let tool = GlobSearchTool;
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let result = tool
            .execute(json!({"pattern": "**/*.rs", "path": manifest_dir}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains(".rs"));
    }

    #[tokio::test]
    async fn glob_search_no_match() {
        let tool = GlobSearchTool;
        let result = tool
            .execute(json!({"pattern": "*.nonexistent_ext", "path": "."}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("未找到"));
    }
}
