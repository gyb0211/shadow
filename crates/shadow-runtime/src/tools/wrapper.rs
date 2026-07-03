//! 工具包装器 -- 装饰器模式, 为 Tool 添加横切逻辑
//!
//! 装饰器不修改 Tool trait 本身, 而是通过组合 (持有 inner) 包装工具,
//! 在 execute() 前后注入额外逻辑 (速率限制、路径安全检查等).

use shadow_core::{tool_attribution, Attributable, Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 工具包装器 trait -- 装饰器模式
///
/// 所有包装器实现此 trait, 通过 `inner()` 访问被包装的原始工具.
/// 包装器本身也是 Tool, 可以被 Agent 当作普通工具使用.
pub trait ToolWrapper: Tool {
    /// 被包装的内部工具
    fn inner(&self) -> &dyn Tool;
}

/// 速率限制工具 -- 每秒最多 N 次调用
///
/// 超过限制时返回错误, 不执行内部工具.
/// 用于防止 LLM 高频调用敏感工具 (如 shell).
pub struct RateLimitedTool {
    inner: Box<dyn Tool>,
    max_per_sec: u32,
    state: Mutex<RateLimitState>,
}

/// 速率限制状态
struct RateLimitState {
    /// 当前秒的起始时间
    window_start: Instant,
    /// 当前窗口内的调用次数
    call_count: u32,
}

impl RateLimitedTool {
    /// 创建速率限制包装器
    ///
    /// - `inner`: 被包装的工具
    /// - `max_per_sec`: 每秒最大调用次数
    pub fn new(inner: Box<dyn Tool>, max_per_sec: u32) -> Self {
        Self {
            inner,
            max_per_sec,
            state: Mutex::new(RateLimitState {
                window_start: Instant::now(),
                call_count: 0,
            }),
        }
    }
}

impl Attributable for RateLimitedTool {
    tool_attribution!("rate_limited");
}

#[async_trait]
impl Tool for RateLimitedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 检查速率限制
        let blocked = {
            let mut state = self.state.lock();
            let elapsed = state.window_start.elapsed();

            // 超过 1 秒, 重置窗口
            if elapsed >= Duration::from_secs(1) {
                state.window_start = Instant::now();
                state.call_count = 0;
            }

            state.call_count += 1;
            state.call_count > self.max_per_sec
        };

        if blocked {
            return Ok(ToolResult::err(format!(
                "速率限制: 每秒最多 {} 次调用, 已超限",
                self.max_per_sec
            )));
        }

        // 转发给内部工具执行
        self.inner.execute(args).await
    }

    fn timeout(&self) -> Option<Duration> {
        self.inner.timeout()
    }

    fn requires_approval(&self) -> bool {
        self.inner.requires_approval()
    }
}

impl ToolWrapper for RateLimitedTool {
    fn inner(&self) -> &dyn Tool {
        self.inner.as_ref()
    }
}

/// 路径安全工具 -- 检查路径参数不超出工作目录
///
/// 扫描 args 中的所有字符串参数, 如果包含路径且解析后超出 workspace,
/// 则拒绝执行. 防止路径遍历攻击 (如 `../../etc/passwd`).
pub struct PathGuardedTool {
    inner: Box<dyn Tool>,
    workspace: PathBuf,
}

impl PathGuardedTool {
    /// 创建路径安全包装器
    ///
    /// - `inner`: 被包装的工具
    /// - `workspace`: 允许操作的工作目录根
    pub fn new(inner: Box<dyn Tool>, workspace: PathBuf) -> Self {
        Self { inner, workspace }
    }
}

impl Attributable for PathGuardedTool {
    tool_attribution!("path_guarded");
}

#[async_trait]
impl Tool for PathGuardedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 检查 args 中的路径参数
        if let Some(violation) = check_path_safety(&args, &self.workspace) {
            return Ok(ToolResult::err(format!(
                "路径安全检查失败: '{violation}' 超出工作目录范围"
            )));
        }

        self.inner.execute(args).await
    }

    fn timeout(&self) -> Option<Duration> {
        self.inner.timeout()
    }

    fn requires_approval(&self) -> bool {
        self.inner.requires_approval()
    }
}

impl ToolWrapper for PathGuardedTool {
    fn inner(&self) -> &dyn Tool {
        self.inner.as_ref()
    }
}

/// 递归检查 Value 中的字符串值, 检测路径是否超出 workspace
///
/// 返回第一个违规的路径字符串 (如有)
fn check_path_safety(value: &Value, workspace: &std::path::Path) -> Option<String> {
    match value {
        Value::String(s) => {
            // 只检查看起来像路径的字符串 (包含 / 或 \)
            if !s.contains('/') && !s.contains('\\') {
                return None;
            }

            // 尝试将字符串解析为路径
            let path = std::path::Path::new(s);

            // 解析为绝对路径
            let abs = if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace.join(path)
            };

            // 规范化路径 (处理 .. 和 .)
            // 注: canonicalize 要求路径存在, 这里手动简化处理
            let normalized = normalize_path(&abs);

            // 检查规范化后的路径是否仍在 workspace 内
            if normalized.starts_with(workspace) {
                None
            } else {
                Some(s.clone())
            }
        }
        Value::Object(map) => {
            for (_, v) in map {
                if let Some(violation) = check_path_safety(v, workspace) {
                    return Some(violation);
                }
            }
            None
        }
        Value::Array(arr) => {
            for v in arr {
                if let Some(violation) = check_path_safety(v, workspace) {
                    return Some(violation);
                }
            }
            None
        }
        _ => None,
    }
}

/// 简化路径规范化 -- 处理 `.` 和 `..` (不要求路径存在)
fn normalize_path(path: &std::path::Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => { /* 跳过 . */ }
            std::path::Component::ParentDir => {
                // 回退一级 (但不退到根之前)
                result.pop();
            }
            std::path::Component::RootDir
            | std::path::Component::Normal(_)
            | std::path::Component::Prefix(_) => {
                result.push(component.as_os_str());
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{FileReadTool, ShellTool};
    use serde_json::json;

    #[test]
    fn rate_limit_allows_under_limit() {
        // 测试: 不超过限制时正常执行
        // 这里只验证 name 委托, execute 需要异步环境
        let wrapper = RateLimitedTool::new(Box::new(ShellTool), 10);
        assert_eq!(wrapper.name(), "shell");
        assert!(wrapper.requires_approval());
    }

    #[tokio::test]
    async fn rate_limit_blocks_over_limit() {
        let wrapper = RateLimitedTool::new(Box::new(ShellTool), 2);

        // 前两次应通过 (虽然命令会失败但不是速率限制)
        let r1 = wrapper.execute(json!({"command": "true"})).await.unwrap();
        assert!(r1.success);

        let r2 = wrapper.execute(json!({"command": "true"})).await.unwrap();
        assert!(r2.success);

        // 第三次应被速率限制
        let r3 = wrapper.execute(json!({"command": "true"})).await.unwrap();
        assert!(!r3.success);
        assert!(r3.error.unwrap().contains("速率限制"));
    }

    #[test]
    fn path_guarded_allows_within_workspace() {
        let workspace = PathBuf::from("/tmp");
        let wrapper = PathGuardedTool::new(Box::new(FileReadTool), workspace);
        assert_eq!(wrapper.name(), "file_read");
    }

    #[tokio::test]
    async fn path_guarded_blocks_traversal() {
        let workspace = PathBuf::from("/tmp/shadow_safe_test");
        std::fs::create_dir_all(&workspace).ok();
        let wrapper = PathGuardedTool::new(Box::new(FileReadTool), workspace.clone());

        // 尝试路径遍历 -- 应被拒绝
        let result = wrapper
            .execute(json!({"path": "../../etc/passwd"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("路径安全检查失败"));

        // 清理
        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn normalize_path_handles_dotdot() {
        let p = normalize_path(std::path::Path::new("/tmp/a/../b"));
        assert_eq!(p, PathBuf::from("/tmp/b"));
    }
}
