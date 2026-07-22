//! 安全策略 -- 命令黑名单 + 环境变量过滤 + Sandbox trait
//!
//! 参考 ZeroClaw security/traits.rs 的 Sandbox 抽象, 在 Shadow 中实现:
//! - [`Sandbox`] trait: OS 级进程隔离抽象 (当前只有 [`NoopSandbox`] 直通实现)
//! - [`SecurityPolicy`]: 命令黑名单 + 环境变量白名单 + 工作目录限制
//!
//! [`SecurityPolicy`] 被 [`crate::tools::ShellTool`] 持有, 在 execute() 中:
//! 1. `is_blocked()` 检查命令是否命中黑名单
//! 2. 设置工作目录 (如果 policy.workspace 有值)
//! 3. `filter_env()` 过滤环境变量 -- 只保留白名单中的变量

pub mod detect;
mod seatbelt;
mod docker;


pub use detect::create_sandbox;

use anyhow::Context;
use serde::{Deserialize, Serialize};

// ── 默认配置 ─────────────────────────────────────────────────────────

/// 默认危险命令黑名单 -- 匹配到任意一条则拒绝执行
///
/// 含 `.*` 的模式按正则匹配 (unanchored search), 其余按子串匹配.
/// 正则模式中 `\|` 表示字面管道符 `|` (转义, 避免被当作正则交替).
const DEFAULT_BLOCKED_PATTERNS: &[&str] = &[
    "rm -rf /",      // 递归删除根目录
    "rm -rf ~",      // 递归删除 home 目录
    "rm -rf *",      // 递归删除当前目录所有文件
    "mkfs",          // 格式化文件系统
    "dd if=",        // 磁盘镜像写入
    "> /dev/sd",     // 写入 SCSI/SATA 块设备
    "> /dev/nvme",   // 写入 NVMe 块设备
    ":(){:|:&};:",   // fork bomb
    r"curl.*\|.*sh", // curl 管道执行 (正则: curl ... | sh)
    r"wget.*\|.*sh", // wget 管道执行 (正则: wget ... | sh)
    "chmod 777 /",   // 修改根目录权限
    "shutdown",      // 关机
    "reboot",        // 重启
    "init 0",        // 关机 (SysV)
    "init 6",        // 重启 (SysV)
];

/// 默认禁止访问的系统目录 -- 这些目录包含系统关键文件, 不应被读写
///
/// 用于安全策略注入 (system prompt) 和路径检查. 可通过
/// [`SecurityPolicy::with_forbidden_paths`] 覆盖.
const DEFAULT_FORBIDDEN_PATHS: &[&str] = &[
    "/etc",  // 系统配置文件
    "/root", // root 用户主目录
    "/boot", // 启动文件
    "/sys",  // 内核虚拟文件系统
    "/proc", // 进程虚拟文件系统
    "/dev",  // 设备文件
];

/// 默认允许的环境变量白名单 -- 只有这些变量会传递给子进程
///
/// 参考 ZeroClaw skill_tool.rs: 过滤掉可能泄露敏感信息的环境变量
/// (如 API_KEY, TOKEN, SECRET 等), 只保留运行命令所需的基本变量.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH",     // 可执行文件搜索路径 (必需, 否则找不到命令)
    "HOME",     // 用户主目录
    "TERM",     // 终端类型
    "LANG",     // 语言/区域设置
    "LC_ALL",   // 区域覆盖设置
    "LC_CTYPE", // 字符分类设置
    "USER",     // 当前用户名
    "SHELL",    // 默认 shell
    "TMPDIR",   // 临时目录
    "PWD",      // 当前工作目录
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandRiskLevel {
    Low,
    Medium,
    High,
}

// ── Sandbox trait ────────────────────────────────────────────────────

/// Sandbox trait -- OS 级进程隔离抽象
///
/// 参考 ZeroClaw security/traits.rs. 不同的 Sandbox 实现可以在命令执行前
/// 对 `Command` 进行包装 (如设置 chroot、namespace、seccomp 过滤等).
///
/// 当前 Shadow 只提供 [`NoopSandbox`] (直通, 无隔离). 后续可扩展:
/// - `FirejailSandbox`: 使用 firejail 沙箱
/// - `NamespaceSandbox`: 使用 Linux namespace 隔离
pub trait Sandbox: Send + Sync {
    /// 包装命令 -- 在命令执行前注入沙箱参数
    ///
    /// 返回 Err 表示沙箱不可用或包装失败, 调用方应决定是否回退到直通执行.
    fn wrap_command(&self, cmd: &mut std::process::Command) -> std::io::Result<()>;

    /// 沙箱是否可用 (如 firejail 未安装时返回 false)
    fn is_available(&self) -> bool;

    /// 沙箱名称 (如 "noop", "firejail")
    fn name(&self) -> &str;

    fn description(&self) -> &str;

}

/// 无沙箱 -- 直通执行, 不做任何隔离
///
/// 默认实现, 命令直接在当前进程环境中执行.
pub struct NoopSandbox;

impl Sandbox for NoopSandbox {
    fn wrap_command(&self, _cmd: &mut std::process::Command) -> std::io::Result<()> {
        // 直通 -- 不修改命令
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "noop"
    }

    fn description(&self) -> &str {
        "Cannot use sandbox"
    }
}


