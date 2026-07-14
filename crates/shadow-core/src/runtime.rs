//! AgentRuntime trait -- Agent 主循环抽象

use std::path::{Path, PathBuf};
use anyhow::Result;

pub trait RuntimePlatformAdapter: Send + Sync {
    /// 运行时所在的环境
    /// docker. native.
    fn name(&self) -> &str;

    /// 环境是否支持shell command
    fn has_shell_access(&self) -> bool;

    /// 文件系统是否支持读写
    fn has_filesystem_access(&self) -> bool;

    /// 当前运行时可以持久化存储的目录
    fn storage_path(&self) -> PathBuf;

    /// 是否支持长时间运行
    /// 如果支持 可能会启动gateway heartbeat tasks
    /// 短期运行的 Serverless runtimes 就返回false
    fn supports_long_running(&self) -> bool;

    /// 当前运行时预计最大可以使用的内存
    /// 默认0。无限制
    /// 嵌入式 serverless 应返回可用的内存容量
    fn memory_budget(&self) -> u64{
        0
    }

    /// 为当前运行时构建一个shell命令进程
    fn build_shell_command(&self, command:&str, workspace_dir: &Path) -> Result<tokio::process::Command>;
}
