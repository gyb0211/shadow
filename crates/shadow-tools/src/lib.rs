//! 影子工具集 -- agent 可调用的工具实现
//!
//! 基本工具：
//! - shell: 执行命令行命令
//! - file_read: 读取文本文件（行号 + 分页）
//! - file_write: 创建或覆盖文件
//! - file_edit: 精确字符串查找替换
//! - glob_search: 文件名 glob 搜索
//! - content_search: 文件内容正则搜索

pub mod attribution;
pub mod file_download;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod git_operations;
pub mod http_request;
pub mod jira;
pub mod llm_task;
pub mod memory_tools;
pub mod pdf_read;
pub mod search;
pub mod shell;
pub mod web_fetch;
pub mod web_search;

pub use file_download::FileDownloadTool;
pub use file_edit::FileEditTool;
pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use git_operations::GitOperationsTool;
pub use http_request::HttpRequestTool;
pub use jira::JiraTool;
pub use llm_task::LlmTaskTool;
pub use memory_tools::{MemoryStoreTool, MemoryRecallTool};
pub use pdf_read::PdfReadTool;
pub use search::{ContentSearchTool, GlobSearchTool};
pub use shell::ShellTool;
pub use web_fetch::WebFetchTool;
pub use web_search::WebSearchTool;
