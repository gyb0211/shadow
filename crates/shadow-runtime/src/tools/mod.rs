use std::sync::Arc;
use shadow_core::Memory;
use crate::tools::registry::ToolRegistry;

pub mod attribution;
pub mod registry;
pub mod cron;

/// 创建默认工具集 -- 返回所有内置工具, 用装饰器包装敏感工具
pub fn default_tools(memory: Option<Arc<dyn Memory>>) -> ToolRegistry {
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    default_tools_with_workspace(memory, workspace)
}

/// 创建默认工具集 (指定工作目录, 用于路径安全检查)
pub fn default_tools_with_workspace(
    memory: Option<Arc<dyn Memory>>,
    workspace: std::path::PathBuf,
) -> ToolRegistry {

    ToolRegistry::new()
    
}
