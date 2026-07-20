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

use anyhow::Context;
use serde::{Deserialize, Serialize};
use shadow_config::autonomy::DelegationPolicy;
use shadow_config::{Config, RiskProfileConfig, RuntimeProfileConfig};
use shadow_core::AutonomyLevel;
use std::fs;
use std::path::{Path, PathBuf};
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
}

// ── SecurityPolicy ───────────────────────────────────────────────────

/// 命令安全策略 -- 黑名单 + 环境变量过滤 + 工作目录限制
///
/// 由 [`crate::tools::ShellTool`] 持有, 在每次执行命令前应用.
///
/// # 示例
/// ```ignore
/// use shadow_runtime::security::SecurityPolicy;
/// use std::path::PathBuf;
///
/// // 默认策略 (无工作目录限制)
/// let policy = SecurityPolicy::new();
/// assert!(policy.is_blocked("rm -rf /").is_some());
///
/// // 带工作目录限制
/// let policy = SecurityPolicy::new().with_workspace(PathBuf::from("/tmp/work"));
/// assert_eq!(policy.workspace(), Some(std::path::Path::new("/tmp/work")));
/// ```
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    pub autonomy: AutonomyLevel,

    pub risk_profile_name: String,
    pub delegation_policy: DelegationPolicy,
    pub workspace_dir: PathBuf,
    pub config_path: Option<PathBuf>,
    pub workspace_only: bool,
    pub allowed_commands: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub allowed_roots: Vec<PathBuf>,
    pub allowed_roots_read_only: Vec<PathBuf>,
    pub max_actions_per_hour: u32,
    pub max_cost_per_day_cents: u32,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
    pub shell_env_passthrough: Vec<String>,
    pub shell_timeout_secs: u64,
    /// Tool name allowlist. `None` is unrestricted (default for agents
    /// without an explicit `risk_profile.allowed_tools` setting).
    /// `Some(vec![])` denies every tool. `Some(list)` admits only the
    /// listed names. Enforced at the agent loop's tool-dispatch site.
    pub allowed_tools: Option<Vec<String>>,
    /// Tool name denylist. Subtracts from the allowed set (whether the
    /// allowed set comes from `allowed_tools` or from the unrestricted
    /// default). `None` and `Some(vec![])` both mean "exclude nothing".
    pub excluded_tools: Option<Vec<String>>,
    /// Tools that never require approval in this profile. Mirrors
    /// `RiskProfileConfig.auto_approve`.
    pub auto_approve: Vec<String>,
    /// Tools that always require approval in this profile. Mirrors
    /// `RiskProfileConfig.always_ask`.
    pub always_ask: Vec<String>,
    /// Whether the sandbox is enabled for this profile. `None`
    /// inherits the global default at the call site.
    pub sandbox_enabled: Option<bool>,
    /// Sandbox backend identifier (e.g. `"firejail"`, `"landlock"`).
    /// `None` inherits the global default.
    pub sandbox_backend: Option<String>,
    /// Extra arguments forwarded to firejail when `sandbox_backend`
    /// resolves to `"firejail"`.
    pub firejail_args: Vec<String>,
    pub tracker: PerSenderTracker,

}

#[derive(Debug)]
pub struct PerSenderTracker {
    buckets: std::sync::Arc<parking_lot::Mutex<HashMap<String, ActionTracker>>>,
}

impl PerSenderTracker {
    /// Bucket key used when no per-sender context is available (cron, CLI).
    pub const GLOBAL_KEY: &'static str = "__global__";

    /// Create an empty tracker with no sender buckets.
    pub fn new() -> Self {
        Self {
            buckets: std::sync::Arc::new(parking_lot::Mutex::new(HashMap::new())),
        }
    }

    /// Resolve the current sender key from the task-local, falling back to GLOBAL_KEY.
    fn current_key() -> String {
        zeroclaw_api::TOOL_LOOP_THREAD_ID
            .try_with(|v| v.clone())
            .ok()
            .flatten()
            .unwrap_or_else(|| Self::GLOBAL_KEY.to_string())
    }

    /// Record one action for the current sender. Returns `true` if allowed
    /// (count after recording <= max), `false` if budget exhausted.
    pub fn record_for_current(&self, max: u32) -> bool {
        let key = Self::current_key();
        self.record_within(&key, max)
    }

    /// Record one action for `key`. Allows the action when count == max (≤ max);
    /// blocks and returns false when count > max.
    pub fn record_within(&self, key: &str, max: u32) -> bool {
        let mut buckets = self.buckets.lock();
        let tracker = buckets.entry(key.to_string()).or_default();
        let count = tracker.record();
        count <= max as usize
    }

    /// Check if the current sender is at or over the limit (without recording).
    pub fn is_limited_for_current(&self, max: u32) -> bool {
        let key = Self::current_key();
        self.is_exhausted(&key, max)
    }

    pub fn is_exhausted(&self, key: &str, max: u32) -> bool {
        if max == 0 {
            return true;
        }
        let mut buckets = self.buckets.lock();
        match buckets.get_mut(key) {
            Some(tracker) => tracker.count() >= max as usize,
            None => false,
        }
    }
}

impl Clone for PerSenderTracker {
    fn clone(&self) -> Self {
        Self {
            buckets: std::sync::Arc::clone(&self.buckets),
        }
    }
}

impl Default for PerSenderTracker {
    fn default() -> Self {
        Self::new()
    }
}


impl Default for SecurityPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityPolicy {
    /// 创建默认安全策略
    ///
    /// - 黑名单: [`DEFAULT_BLOCKED_PATTERNS`]
    /// - 环境变量白名单: [`SAFE_ENV_VARS`]
    /// - 工作目录: None (不限制)
    #[must_use]
    pub fn new() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            blocked_patterns: DEFAULT_BLOCKED_PATTERNS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            allowed_env_vars: SAFE_ENV_VARS.iter().map(|s| (*s).to_string()).collect(),
            workspace: None,
            allowed_commands: Vec::new(),
            forbidden_paths: DEFAULT_FORBIDDEN_PATHS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            block_high_risk_commands: true,
            require_approval_for_medium_risk: true,
        }
    }

    pub fn for_agent(config: &Config, agent_alias: &str) -> anyhow::Result<Self> {
        let risk_profile = config.risk_profile_for_agent(agent_alias).ok_or_else(|| {
            shadow_log::record!(
                ERROR,
                shadow_log::Event::new(module_path!(), shadow_log::Action::Fail).with_outcome(shadow_log::EventOutcome::Failure)
                .with_attrs(serde_json::json!({"agent_alias": agent_alias})),
                "SecurityPolicy::for_agent agent has no resolvable risk_profile"
            );
            anyhow::Error::msg(
                format!("agents.{agent_alias} has no resolvable risk_profile (load-time validation should have caught this)")
            );
        })?;

        let runtime_profile = config.runtime_profile_for_agent(agent_alias);
        let agent_workspace = config.agent_workspace_dir(agent_alias);

        fs::create_dir_all(&agent_workspace).with_context(|| {
            format!(
                "SecurityPolicy::for_agent: failed to create agent workspace dir :{}",
                agent_workspace.display()
            )
        })?;

        let mut policy: SecurityPolicy =
            Self::from_profiles(risk_profile, runtime_profile, &agent_workspace);
        if let Some(agent_cfg) = config.agents.get(agent_alias) {
            policy.ris
        }
    }

    /// 设置工作目录 (builder 风格)
    #[must_use]
    pub fn with_workspace(mut self, workspace: PathBuf) -> Self {
        self.workspace = Some(workspace);
        self
    }

    pub fn can_act(&self) -> bool {
        self.autonomy != AutonomyLevel::ReadOnly
    }

    pub fn is_rate_limited(&self) -> bool {
        true
    }

    pub fn record_action(&self) -> bool {
        true
    }

    fn is_command_allowed(&self, command: &str) -> bool {
        true
    }
    fn is_command_explicitly_allowed(&self, command: &str) -> bool {
        true
    }
    fn command_risk_level(&self, command: &str) -> CommandRiskLevel {
        CommandRiskLevel::Low
    }

    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
    ) -> Result<CommandRiskLevel, String> {
        if !self.is_command_allowed(command) {
            return Err(format!("Command not allowed by security policy: {command}"));
        }

        let risk: CommandRiskLevel = self.command_risk_level(command);

        if risk == CommandRiskLevel::High {
            if self.block_high_risk_commands && !self.is_command_explicitly_allowed(command) {
                return Err(format!(
                    "Command({command}) blocked: high-risk command is disallowed by policy."
                ));
            }
            if self.autonomy == AutonomyLevel::Supervised && !approved {
                return Err(format!(
                    "Command({command}) required explicit approval(true): high-risk operation."
                ));
            }
        }
        if risk == CommandRiskLevel::Medium
            && self.autonomy == AutonomyLevel::Supervised
            && self.require_approval_for_medium_risk
            && !approved
        {
            return Err(format!(
                "Command({command}) required explicit approval(true): medium-risk operation."
            ));
        }

        Ok(risk)
    }

    /// 设置允许的命令白名单 (builder 风格)
    ///
    /// 空列表表示不限制 (默认). 非空时, 只有列表中的命令允许执行.
    #[must_use]
    pub fn with_allowed_commands(mut self, commands: Vec<String>) -> Self {
        self.allowed_commands = commands;
        self
    }

    /// 设置禁止访问的路径 (builder 风格)
    ///
    /// 覆盖默认的 [`DEFAULT_FORBIDDEN_PATHS`].
    #[must_use]
    pub fn with_forbidden_paths(mut self, paths: Vec<String>) -> Self {
        self.forbidden_paths = paths;
        self
    }

    /// 检查命令是否被黑名单阻止
    ///
    /// 返回命中的规则字符串 (用于错误提示), 未命中返回 None.
    ///
    /// 匹配规则:
    /// - 含 `.*` 的模式按正则匹配 (unanchored search, 用 `regex::Regex::is_match`)
    /// - 其余模式按子串匹配 (`str::contains`)
    pub fn is_blocked(&self, command: &str) -> Option<String> {
        for pattern in &self.blocked_patterns {
            // 含 ".*" 的模式按正则匹配
            if pattern.contains(".*") {
                if let Ok(re) = regex::Regex::new(pattern)
                    && re.is_match(command)
                {
                    return Some(pattern.clone());
                }
                // 正则编译失败则跳过该规则 (不应发生, 模式是预定义的)
                continue;
            }
            // 其余按子串匹配
            if command.contains(pattern.as_str()) {
                return Some(pattern.clone());
            }
        }
        None
    }

    /// 过滤环境变量 -- 只保留白名单中的变量
    ///
    /// # 参数
    /// - `env`: 原始环境变量列表 (如 `std::env::vars().collect()`)
    ///
    /// # 返回
    /// 过滤后的环境变量列表, 仅包含 `allowed_env_vars` 中列出的变量.
    pub fn filter_env(&self, env: &[(String, String)]) -> Vec<(String, String)> {
        env.iter()
            .filter(|(k, _)| self.allowed_env_vars.iter().any(|allowed| allowed == k))
            .cloned()
            .collect()
    }

    /// 获取工作目录 (None = 不限制)
    pub fn workspace(&self) -> Option<&Path> {
        self.workspace.as_deref()
    }

    /// 获取黑名单模式列表
    pub fn blocked_patterns(&self) -> &[String] {
        &self.blocked_patterns
    }

    /// 获取允许的环境变量名列表
    pub fn allowed_env_vars(&self) -> &[String] {
        &self.allowed_env_vars
    }

    /// 获取允许的命令白名单 (空 = 不限制)
    pub fn allowed_commands(&self) -> &[String] {
        &self.allowed_commands
    }

    /// 获取禁止访问的路径列表
    pub fn forbidden_paths(&self) -> &[String] {
        &self.forbidden_paths
    }

    fn from_profiles(
        risk_profile: &RiskProfileConfig,
        runtime_profile: Option<&RuntimeProfileConfig>,
        workspace: &PathBuf,
    ) -> Self {
        let effective_workspace_only = if risk_profile.level == AutonomyLevel::Full {
            false
        } else {
            risk_profile.workspace_only
        };

        let runtime_default = RuntimeProfileConfig::default();
        let runtime = runtime_profile.unwrap_or(runtime_default);

        Self {
            autonomy: risk_profile.level,
            blocked_patterns: vec![],
            allowed_env_vars: vec![],
            workspace: None,
            allowed_commands: vec![],
            forbidden_paths: vec![],
            block_high_risk_commands: false,
            require_approval_for_medium_risk: false,
        }
    }
}
