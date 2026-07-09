//! Bootstrap 文件系统 -- 加载 workspace 下的身份文件注入 prompt
//!
//! 参考 ZeroClaw 的 system_prompt.rs: agent 启动时读取 workspace 根目录下的
//! 身份文件 (`AGENTS.md`, `SOUL.md`, `IDENTITY.md`, `USER.md`), 将内容注入
//! system prompt, 让 agent 感知项目上下文与用户偏好.
//!
//! # 安全措施
//! - 每个文件最大 20000 字符, 超出截断
//! - 注入前用 [`injection_guard`] 扫描, 检测 prompt 注入攻击
//! - 告知 LLM 这些文件已注入, 无需用 `file_read` 重新读取
//!
//! # 优先级
//! BootstrapSection priority=95, 位于 Identity(100) 之后, DateTime(90) 之前.
//!
//! [`injection_guard`]: super::injection_guard

use std::fs;
use std::path::Path;

use super::injection_guard::scan_context_content;
use super::{PromptContext, PromptSection};

/// 每个身份文件的最大字符数, 超出截断
const MAX_FILE_CHARS: usize = 20_000;

/// Bootstrap 加载的身份文件名 (按此顺序加载)
const BOOTSTRAP_FILES: &[&str] = &["AGENTS.md", "SOUL.md", "IDENTITY.md", "USER.md"];

/// 单个身份文件的加载结果
#[derive(Debug, Clone)]
struct LoadedFile {
    /// 文件名
    name: &'static str,
    /// 渲染后的内容 (已截断 + 已通过安全扫描; 不安全则为拦截提示)
    content: String,
    /// 是否成功加载且安全 (供调试/未来扩展使用)
    #[allow(dead_code)]
    loaded: bool,
}

/// 从指定目录加载单个身份文件
///
/// 返回 `Some(LoadedFile)` 表示文件存在, `None` 表示文件不存在.
fn load_file(dir: &Path, filename: &'static str) -> Option<LoadedFile> {
    let path = dir.join(filename);
    let raw = fs::read_to_string(&path).ok()?;

    // 截断超长文件
    let truncated = if raw.chars().count() > MAX_FILE_CHARS {
        let truncated_str: String = raw.chars().take(MAX_FILE_CHARS).collect();
        format!("{truncated_str}\n\n[... 文件超出 {MAX_FILE_CHARS} 字符, 已截断 ...]")
    } else {
        raw
    };

    // 安全扫描
    let scan = scan_context_content(&truncated, filename);
    if scan.safe {
        Some(LoadedFile {
            name: filename,
            content: truncated,
            loaded: true,
        })
    } else {
        // 不安全: 用拦截提示替换内容, 仍注入 (让 LLM 知道文件被拦截)
        Some(LoadedFile {
            name: filename,
            content: scan.sanitized,
            loaded: false,
        })
    }
}

/// 从 workspace 目录加载所有身份文件
fn load_bootstrap_files(workspace: &Path) -> Vec<LoadedFile> {
    BOOTSTRAP_FILES
        .iter()
        .filter_map(|&name| load_file(workspace, name))
        .collect()
}

/// Bootstrap 文件系统段 -- 加载 workspace 身份文件并注入 system prompt
///
/// # 渲染逻辑
/// 1. 从 `ctx.workspace_dir` 读取身份文件
/// 2. 每个文件截断至 20000 字符, 经 injection_guard 扫描
/// 3. 拼接所有文件内容, 前置声明 "文件已注入, 无需重新读取"
/// 4. 无任何文件时返回空字符串 (不污染 prompt)
pub struct BootstrapSection;

impl BootstrapSection {
    /// 创建 Bootstrap 段
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for BootstrapSection {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptSection for BootstrapSection {
    fn name(&self) -> &str {
        "bootstrap"
    }

    fn render(&self, ctx: &PromptContext) -> String {
        let files = load_bootstrap_files(&ctx.workspace_dir);
        if files.is_empty() {
            // 无身份文件, 返回空 (build 会过滤空字符串)
            return String::new();
        }

        let mut lines: Vec<String> = Vec::new();
        lines.push("## 项目身份文件 (Bootstrap)".to_string());
        lines.push("以下文件已自动注入, 请勿使用 file_read 工具重新读取:".to_string());
        lines.push(String::new());

        for file in &files {
            lines.push(format!("### {} ###", file.name));
            lines.push(file.content.clone());
            lines.push(String::new());
        }

        lines.join("\n")
    }

    fn priority(&self) -> i32 {
        95
    }
}

// ── 单元测试 ──
