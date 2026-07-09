//! 集中式 Attributable impl -- 为本 crate 所有 Tool 实现归因
//!
//! 每行 tool_attribution! 生成 Role::Tool(ToolKind) + alias = Tool::name()
//! 日志归因使用 <kind>.<alias> 复合标识, 和 channel/provider/memory 一致.
//!
//! 新增 Tool 时在此文件追加一行即可, 无需在工具源码里写 impl Attributable.

use shadow_core::{tool_attribution, ToolKind};

use crate::tools::backup_tool::BackupTool;
use crate::tools::content_search::ContentSearchTool;
use crate::tools::cron_tool::CronTool;
use crate::tools::file_download::FileDownloadTool;
use crate::tools::file_edit::FileEditTool;
use crate::tools::file_read::FileReadTool;
use crate::tools::file_upload::FileUploadTool;
use crate::tools::file_upload_bundle::FileUploadBundleTool;
use crate::tools::file_write::FileWriteTool;
use crate::tools::git_ops::GitOpsTool;
use crate::tools::glob_search::GlobSearchTool;
use crate::tools::http_request::HttpRequestTool;
use crate::tools::memory_export::MemoryExportTool;
use crate::tools::memory_forget::MemoryForgetTool;
use crate::tools::memory_purge::MemoryPurgeTool;
use crate::tools::shell::ShellTool;
use crate::tools::spawn_subagent::SpawnSubagentTool;
use crate::tools::web_fetch::WebFetchTool;
use crate::tools::web_search::WebSearchTool;
use crate::tools::wrapper::{PathGuardedTool, RateLimitedTool};

tool_attribution!(BackupTool, ToolKind::Plugin);
tool_attribution!(ContentSearchTool, ToolKind::Search);
tool_attribution!(CronTool, ToolKind::Plugin);
tool_attribution!(FileDownloadTool, ToolKind::Plugin);
tool_attribution!(FileEditTool, ToolKind::Plugin);
tool_attribution!(FileReadTool, ToolKind::Plugin);
tool_attribution!(FileUploadBundleTool, ToolKind::Plugin);
tool_attribution!(FileUploadTool, ToolKind::Plugin);
tool_attribution!(FileWriteTool, ToolKind::Plugin);
tool_attribution!(GitOpsTool, ToolKind::Shell);
tool_attribution!(GlobSearchTool, ToolKind::Search);
tool_attribution!(HttpRequestTool, ToolKind::HttpRequest);
tool_attribution!(MemoryExportTool, ToolKind::Memory);
tool_attribution!(MemoryForgetTool, ToolKind::Memory);
tool_attribution!(MemoryPurgeTool, ToolKind::Memory);
tool_attribution!(PathGuardedTool, ToolKind::Plugin);
tool_attribution!(RateLimitedTool, ToolKind::Plugin);
tool_attribution!(ShellTool, ToolKind::Shell);
tool_attribution!(SpawnSubagentTool, ToolKind::SpawnSubAgent);
tool_attribution!(WebFetchTool, ToolKind::FetchUrl);
tool_attribution!(WebSearchTool, ToolKind::Search);
