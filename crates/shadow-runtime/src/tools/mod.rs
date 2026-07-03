//! 工具集合 -- agent 可调用的基本工具
//!
//! 当前实现:
//! - Shell: 执行 shell 命令
//! - FileRead: 读取文件内容
//! - FileWrite: 写入文件内容
//! - MemoryRecall: 检索记忆
//! - MemoryStore: 存储记忆
//! - GlobSearch: 按文件名模式搜索文件
//! - ContentSearch: 在文件内容中搜索文本

pub mod content_search;
pub mod file_read;
pub mod file_write;
pub mod glob_search;
pub mod memory_recall;
pub mod memory_store;
pub mod registry;
pub mod shell;
pub mod wrapper;

pub use content_search::ContentSearchTool;
pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use glob_search::GlobSearchTool;
pub use memory_recall::MemoryRecallTool;
pub use memory_store::MemoryStoreTool;
pub use registry::ToolRegistry;
pub use shell::ShellTool;
pub use wrapper::{PathGuardedTool, RateLimitedTool, ToolWrapper};

use shadow_core::Memory;
use std::sync::Arc;
use crate::security::SecurityPolicy;
/// 创建默认工具集 -- 返回所有内置工具, 用装饰器包装敏感工具
///
/// - `memory`: 可选的 Memory 后端, Some 时注册 memory_recall / memory_store 工具
pub fn default_tools(memory: Option<Arc<dyn Memory>>) -> ToolRegistry {
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    default_tools_with_workspace(memory, workspace)
}

/// 创建默认工具集 (指定工作目录, 用于路径安全检查)
pub fn default_tools_with_workspace(
    memory: Option<Arc<dyn Memory>>,
    workspace: std::path::PathBuf,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    // 安全策略: 黑名单 + 环境变量过滤 + 工作目录限制
    let security = SecurityPolicy::new().with_workspace(workspace.clone());

    // Shell 工具 -- 安全策略 + 速率限制 (每秒 10 次) + 路径安全
    registry.register(Box::new(RateLimitedTool::new(
        Box::new(ShellTool::new(security)),
        10,
    )));

    // FileRead 工具 -- 路径安全 (防止读取工作目录外文件)
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(FileReadTool),
        workspace.clone(),
    )));

    // FileWrite 工具 -- 路径安全 (防止写入工作目录外文件)
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(FileWriteTool),
        workspace,
    )));

    // 文件搜索工具
    registry.register(Box::new(GlobSearchTool));
    registry.register(Box::new(ContentSearchTool));

    // 记忆工具 -- 仅在 memory 后端可用时注册
    if let Some(mem) = memory {
        registry.register(Box::new(MemoryRecallTool::new(Arc::clone(&mem))));
        registry.register(Box::new(MemoryStoreTool::new(mem)));
    }

    registry
}
