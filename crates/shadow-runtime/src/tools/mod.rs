//! 工具集合 -- agent 可调用的基本工具
//!
//! 当前实现:
//! - Shell: 执行 shell 命令
//! - FileRead: 读取文件内容
//! - FileWrite: 写入文件内容

pub mod file_read;
pub mod file_write;
pub mod shell;

pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use shell::ShellTool;

use agent_core::Tool;

/// 创建默认工具集 -- 返回所有内置工具
pub fn default_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ShellTool),
        Box::new(FileReadTool),
        Box::new(FileWriteTool),
    ]
}
