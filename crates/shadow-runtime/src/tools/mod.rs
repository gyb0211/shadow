//! 工具集合 -- agent 可调用的基本工具
//!
//! 当前实现 (24 个工具):
//! - Shell: 执行 shell 命令 (黑名单+超时)
//! - FileRead: 读取文件内容
//! - FileWrite: 写入文件内容 (原子写入)
//! - FileEdit: 精确替换文件中的文本片段 (patch 风格)
//! - FileDownload: 从 URL 下载文件
//! - FileUpload: 上传文件到 URL (multipart)
//! - BackupTool: 文件/目录备份 (复制/tar.gz)
//! - GlobSearch: 按文件名模式搜索文件
//! - ContentSearch: 在文件内容中搜索文本
//! - HttpRequest: HTTP 请求 (SSRF 防护)
//! - WebFetch: URL 抓取 (text/markdown/raw)
//! - WebSearch: Web 搜索 (DuckDuckGo/Google)
//! - GitOps: Git 操作 (白名单)
//! - SpawnSubagent: 子代理委派
//! - CronTool: 定时任务管理
//! - MemoryRecall: 检索记忆
//! - MemoryStore: 存储记忆
//! - MemoryForget: 删除单条记忆
//! - MemoryPurge: 批量清除记忆
//! - MemoryExport: 导出记忆
//! - SkillManage: 技能管理

pub mod backup_tool;
pub mod content_search;
pub mod cron_tool;
pub mod file_download;
pub mod file_edit;
pub mod file_read;
pub mod file_upload;
pub mod file_upload_bundle;
pub mod file_write;
pub mod git_ops;
pub mod glob_search;
pub mod http_request;
pub mod memory_export;
pub mod memory_forget;
pub mod memory_purge;
pub mod memory_recall;
pub mod memory_store;
pub mod registry;
pub mod search_routing;
pub mod shell;
pub mod skill_manage;
pub mod spawn_subagent;
pub mod web_fetch;
pub mod web_search;
pub mod wrapper;

pub use backup_tool::BackupTool;
pub use content_search::ContentSearchTool;
pub use cron_tool::CronTool;
pub use file_download::FileDownloadTool;
pub use file_edit::FileEditTool;
pub use file_read::FileReadTool;
pub use file_upload::FileUploadTool;
pub use file_upload_bundle::FileUploadBundleTool;
pub use file_write::FileWriteTool;
pub use git_ops::GitOpsTool;
pub use glob_search::GlobSearchTool;
pub use http_request::HttpRequestTool;
pub use memory_export::MemoryExportTool;
pub use memory_forget::MemoryForgetTool;
pub use memory_purge::MemoryPurgeTool;
pub use memory_recall::MemoryRecallTool;
pub use memory_store::MemoryStoreTool;
pub use registry::ToolRegistry;
pub use shell::ShellTool;
pub use skill_manage::{SkillListTool, SkillManageTool, SkillViewTool};
pub use spawn_subagent::SpawnSubagentTool;
pub use web_fetch::WebFetchTool;
pub use web_search::WebSearchTool;
pub use wrapper::{PathGuardedTool, RateLimitedTool, ToolWrapper};

use crate::security::SecurityPolicy;
use shadow_core::Memory;
use std::sync::Arc;

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
    let mut registry = ToolRegistry::new();

    let security = SecurityPolicy::new().with_workspace(workspace.clone());

    // Shell 工具 -- 安全策略 + 速率限制
    registry.register(Box::new(RateLimitedTool::new(
        Box::new(ShellTool::new(security)),
        10,
    )));

    // 文件工具 -- 路径安全
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(FileReadTool),
        workspace.clone(),
    )));
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(FileWriteTool),
        workspace.clone(),
    )));
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(FileEditTool),
        workspace.clone(),
    )));
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(FileDownloadTool::new()),
        workspace.clone(),
    )));
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(FileUploadTool::new()),
        workspace.clone(),
    )));
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(FileUploadBundleTool::new()),
        workspace.clone(),
    )));
    registry.register(Box::new(PathGuardedTool::new(
        Box::new(BackupTool::new()),
        workspace.clone(),
    )));

    // 文件搜索
    registry.register(Box::new(GlobSearchTool));
    registry.register(Box::new(ContentSearchTool));

    // HTTP / Web 工具
    registry.register(Box::new(HttpRequestTool::new()));
    registry.register(Box::new(WebFetchTool::new()));
    registry.register(Box::new(WebSearchTool::new()));

    // Git 操作
    registry.register(Box::new(GitOpsTool::new()));

    // 子代理委派
    registry.register(Box::new(SpawnSubagentTool::new()));

    // Cron 管理
    registry.register(Box::new(CronTool::new()));

    // 记忆工具
    if let Some(mem) = memory {
        registry.register(Box::new(MemoryRecallTool::new(Arc::clone(&mem))));
        registry.register(Box::new(MemoryStoreTool::new(Arc::clone(&mem))));
        registry.register(Box::new(MemoryForgetTool::new(Arc::clone(&mem))));
        registry.register(Box::new(MemoryPurgeTool::new(Arc::clone(&mem))));
        registry.register(Box::new(MemoryExportTool::new(mem)));
    }

    registry
}
