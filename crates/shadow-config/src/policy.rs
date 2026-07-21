use crate::autonomy::{AutonomyLevel, DelegationPolicy};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Component, Path, PathBuf};
use std::process::Child;
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::Context;
use tokio::io::split;
use crate::multi::alias_agent::AccessMode;

#[derive(Debug)]
pub struct ActionTracker {
    actions: Mutex<Vec<Instant>>,
}

impl ActionTracker {
    pub fn new() -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
        }
    }

    /// 记录当前的action 移除一小时窗口之前的数据 并返回现在的长度
    pub fn record(&self) -> usize {
        let mut actions = self.actions.lock();
        let cutoff = Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
        actions.push(Instant::now());
        actions.len()
    }

    pub fn count(&self) -> usize {
        let mut actions = self.actions.lock();
        let cutoff = Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
        actions.len()
    }
}

impl Clone for ActionTracker {
    fn clone(&self) -> Self {
        let mut actions = self.actions.lock();
        Self {
            actions: Mutex::new(actions.clone()),
        }
    }
}

impl Default for ActionTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct PerSenderTracker {
    buckets: Arc<Mutex<HashMap<String, ActionTracker>>>,
}

impl PerSenderTracker {
    pub const GLOBAL_KEY: &'static str = "__global__";
    pub fn new() -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn current_key() -> String {
        shadow_core::TOOL_LOOP_THREAD_ID
            .try_with(|v| v.clone())
            .ok()
            .flatten()
            .unwrap_or_else(|| Self::GLOBAL_KEY.to_string())
    }

    pub fn record_for_current(&self, max: u32) -> bool {
        let key = Self::current_key();
        self.record_within(&key, max)
    }

    pub fn record_within(&self, key: &str, max: u32) -> bool {
        let mut buckets = self.buckets.lock();
        let tracker = buckets.entry(key.to_string()).or_default();
        let count = tracker.count();
        count <= max as usize
    }

    pub fn is_limited_for_current(&self, max: u32) -> bool {
        let key = Self::current_key();
        self.is_exhausted(&key, max)
    }

    pub fn is_exhausted(&self, key: &str, max: u32) -> bool {
        if max <= 0 {
            return true;
        }
        let buckets = self.buckets.lock();
        match buckets.get(key) {
            None => false,
            Some(tracker) => tracker.count() >= max as usize,
        }
    }
}

impl Default for PerSenderTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for PerSenderTracker {
    fn clone(&self) -> Self {
        Self {
            buckets: Arc::clone(&self.buckets),
        }
    }
}

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
    pub allowed_roots_write_only: Vec<PathBuf>,
    pub max_actions_per_hour: u32,
    pub max_cost_per_day_cents: u32,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
    pub shell_env_passthrough: Vec<String>,
    pub shell_timeout_secs: u64,
    pub allowed_tools: Option<Vec<String>>,
    pub excluded_tools: Option<Vec<String>>,
    /// `RiskProfileConfig.auto_approve` -- 自动批准的工具列表。
    pub auto_approve: Vec<String>,
    /// `RiskProfileConfig.always_ask` -- 始终需要确认的工具列表。
    pub always_ask: Vec<String>,
    pub sandbox_enabled: Option<bool>,
    pub sandbox_backend: Option<String>,
    pub firejail_args: Vec<String>,
    pub tracker: PerSenderTracker,
}

impl SecurityPolicy {
    /// allowed_tools 为None 或者包含了tool_name
    /// excluded_tools 为Some 且包含了tool_name
    /// 被允许 且 不被排除 即为可用
    pub fn is_tool_allowed(&self, name: &str) -> bool {
        let allowed = self
            .allowed_tools
            .as_deref()
            .is_none_or(|arr| arr.iter().any(|t| t == name));
        let excluded = self
            .excluded_tools
            .as_deref()
            .is_some_and(|arr| arr.iter().any(|t| t == name));
        allowed && !excluded
    }
}

#[cfg(not(target_os = "windows"))]
pub fn default_allowed_commands() -> Vec<String> {
    #[allow(unused_mut)]
    let mut cmds = vec![
        "git".into(),
        "npm".into(),
        "cargo".into(),
        "ls".into(),
        "cat".into(),
        "grep".into(),
        "find".into(),
        "echo".into(),
        "pwd".into(),
        "wc".into(),
        "head".into(),
        "tail".into(),
        "date".into(),
        "df".into(),
        "du".into(),
        "uname".into(),
        "uptime".into(),
        "hostname".into(),
        "python".into(),
        "python3".into(),
        "pip".into(),
        "node".into(),
    ];

    #[cfg(target_os = "linux")]
    cmds.push("free".into());
    cmds
}

#[cfg(target_os = "windows")]
pub fn default_allowed_commands() -> Vec<String> {
    vec![
        // 跨平台工具
        "git".into(),
        "npm".into(),
        "cargo".into(),
        "echo".into(),
        // Windows 原生命令
        "dir".into(),
        "type".into(),
        "findstr".into(),
        "where".into(),
        "more".into(),
        "date".into(),
        // Unix 命令 (通过 Git for Windows / MSYS2 可用)
        "ls".into(),
        "cat".into(),
        "grep".into(),
        "find".into(),
        "pwd".into(),
        "wc".into(),
        "head".into(),
        "tail".into(),
        "df".into(),
        "du".into(),
        "uname".into(),
        "uptime".into(),
        "hostname".into(),
        "python".into(),
        "python3".into(),
        "pip".into(),
        "node".into(),
    ]
}

#[cfg(not(target_os = "windows"))]
pub fn default_forbidden_paths() -> Vec<String> {
    vec![
        "/etc".into(),
        "/root".into(),
        "/home".into(),
        "/usr".into(),
        "/bin".into(),
        "/sbin".into(),
        "/lib".into(),
        "/opt".into(),
        "/boot".into(),
        "/dev".into(),
        "/proc".into(),
        "/sys".into(),
        "/var".into(),
        "/tmp".into(),
        "~/.ssh".into(),
        "~/.gnupg".into(),
        "~/.aws".into(),
        "~/.config".into(),
    ]
}

#[cfg(target_os = "windows")]
pub(crate) fn default_forbidden_paths() -> Vec<String> {
    vec![
        "C:\\Windows".into(),
        "C:\\Windows\\System32".into(),
        "C:\\Program Files".into(),
        "C:\\Program Files (x86)".into(),
        "C:\\ProgramData".into(),
        "~/.ssh".into(),
        "~/.gnupg".into(),
        "~/.aws".into(),
        "~/.config".into(),
    ]
}

/// 判断 `expanded` 路径是否落在 `roots` 中的任意一个根目录下。
///
/// 同时用 canonical 路径（解析符号链接后的真实路径）和原始路径做前缀匹配,
/// 确保无论 `expanded` 走的是符号链接路径还是真实路径都能正确判断:
/// - 只用原始路径: 某人通过真实路径访问符号链接指向的目标时会漏判
/// - 只用 canonical 路径:  canonicalize 失败（路径不存在）时会误判
fn roots_contain(roots: &[PathBuf], expanded: &Path) -> bool {
    roots.iter().any(|root| {
        // 解析符号链接, 得到真实绝对路径; 失败时回退到原始路径
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        // 两种路径都试, 兼容符号链接场景
        expanded.starts_with(&canonical) || expanded.starts_with(root)
    })
}

/// 判断 child 路径是否在 parent 路径下。
///
/// 同 roots_contain 类似, 同时用 canonical 和原始路径做前缀匹配。
fn path_contains(parent: &Path, child: &Path) -> bool {
    let canonical_parent = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    let canonical_child = child.canonicalize().unwrap_or_else(|_| child.to_path_buf());
    canonical_child.starts_with(canonical_parent) || child.starts_with(parent)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationViolation {
    AutonomyAboveParent {
        child: AutonomyLevel,
        parent: AutonomyLevel,
    },

    ReadWriteRootNotInParent {
        path: PathBuf,
    },
    ReadOnlyRootNotInParent {
        path: PathBuf,
    },
    WriteOnlyRootNotInParent {
        path: PathBuf,
    },
    CommandNotInParent {
        command: String,
    },
    WorkspaceOnlyDisabledByChild,
    ForbiddenPathDroppedByChild {
        path: String,
    },
    ShellEnvPassthroughExpanded {
        variable: String,
    },
    MaxActionsExceeded {
        child: u32,
        parent: u32,
    },
    MaxCostExceeded {
        child: u32,
        parent: u32,
    },
    ShellTimeoutExceeded {
        child: u64,
        parent: u64,
    },
    BlockHighRiskCommandsDisabledByChild,
    RequireApprovalDisabledByChild,
}

impl Display for EscalationViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AutonomyAboveParent { child, parent } => {
                write!(f, "subagent autonomy={child:?} exceeds parent's {parent:?}")
            }
            Self::ReadWriteRootNotInParent { path } => write!(
                f,
                "subagent allowed_roots entry {path:?} is not contained within any of the parent's allowed_roots entries"
            ),
            Self::ReadOnlyRootNotInParent { path } => write!(
                f,
                "subagent allowed_roots_read_only entry {path:?} is not contained within the parent's allowed_roots or allowed_roots_read_only"
            ),
            Self::WriteOnlyRootNotInParent { path } => write!(
                f,
                "subagent allowed_roots_write_only entry {path:?} is not contained within the parent's allowed_roots or allowed_roots_write_only"
            ),
            Self::CommandNotInParent { command } => write!(
                f,
                "subagent allowed_commands entry {command:?} is not present on the parent's allowed_commands"
            ),
            Self::WorkspaceOnlyDisabledByChild => write!(
                f,
                "subagent attempts to disable workspace_only but the parent enforces it"
            ),
            Self::ForbiddenPathDroppedByChild { path } => write!(
                f,
                "subagent drops forbidden_paths entry {path:?} that the parent enforces"
            ),
            Self::ShellEnvPassthroughExpanded { variable } => write!(
                f,
                "subagent shell_env_passthrough entry {variable:?} is not present on the parent's list"
            ),
            Self::MaxActionsExceeded { child, parent } => write!(
                f,
                "subagent max_actions_per_hour={child} exceeds parent's {parent}"
            ),
            Self::MaxCostExceeded { child, parent } => write!(
                f,
                "subagent max_cost_per_day_cents={child} exceeds parent's {parent}"
            ),
            Self::ShellTimeoutExceeded { child, parent } => write!(
                f,
                "subagent shell_timeout_secs={child} exceeds parent's {parent}"
            ),
            Self::BlockHighRiskCommandsDisabledByChild => write!(
                f,
                "subagent attempts to set block_high_risk_commands=false but the parent enforces it"
            ),
            Self::RequireApprovalDisabledByChild => write!(
                f,
                "subagent attempts to set require_approval_for_medium_risk=false but the parent enforces it"
            ),
        }
    }
}

impl Error for EscalationViolation {}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            risk_profile_name: String::new(),
            delegation_policy: DelegationPolicy::default(),
            workspace_dir: PathBuf::from("."),
            config_path: None,
            workspace_only: true,
            allowed_commands: default_allowed_commands(),
            forbidden_paths: default_forbidden_paths(),
            allowed_roots: Vec::new(),
            allowed_roots_read_only: Vec::new(),
            allowed_roots_write_only: Vec::new(),
            max_actions_per_hour: 20,
            max_cost_per_day_cents: 500,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            shell_env_passthrough: vec![],
            shell_timeout_secs: 60,
            allowed_tools: None,
            excluded_tools: None,
            auto_approve: vec![],
            always_ask: vec![],
            sandbox_enabled: None,
            sandbox_backend: None,
            firejail_args: vec![],
            tracker: PerSenderTracker::new(),
        }
    }
}

/// 获取当前用户家目录。
///
/// Unix 读 `$HOME`; Windows 优先读 `%USERPROFILE%`, 回退到 `%HOME%`。
fn home_dir() -> Option<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
}

/// 展开 `~` 和 `~/foo` 为家目录绝对路径, 不匹配则原样返回。
fn expand_user_path(path: &str) -> PathBuf {
    // 单独的 `~` 直接替换为家目录
    if path == "~"
        && let Some(home) = home_dir()
    {
        return home;
    }

    // `~/foo` 去掉 `~/` 前缀拼到家目录后面
    if let Some(stripped) = path.strip_suffix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(stripped);
    }

    PathBuf::from(path)
}

/// 判断路径是否指向空设备（/dev/null 或 NUL）。
///
/// 安全策略中需特殊处理: 空设备不在任何 allowed_roots 下,
/// 但重定向输出到空设备是无害操作, 不应被路径检查拦截。
fn is_null_device(path: &Path) -> bool {
    #[cfg(not(target_os = "windows"))]
    {
        path == Path::new("/dev/null")
    }
    #[cfg(target_os = "windows")]
    {
        let s = path.to_string_lossy();
        let lower = s.to_ascii_lowercase();
        lower == "nul" || lower == r"\\.\nul"
    }
}

/// 将路径剥离盘符/根目录/当前目录前缀, 只保留 Normal 段拼成相对路径。
///
/// 遇到 `..` 直接返回 None (防止目录穿越逃逸)。
/// 结果为空也返回 None。
///
/// 例: `/home/user/project/src` -> `home/user/project/src`
fn rootless_path(path: &Path) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
            Component::ParentDir => return None,
            Component::Normal(part) => relative.push(part),
        }
    }

    if relative.as_os_str().is_empty() {
        None
    } else {
        Some(relative)
    }
}

/// 规范化路径的中间结构: 盘符 + 无根路径文本。
struct NormalizeRootlessPath {
    drive: Option<u8>,
    text: String,
}

/// 将路径规范化为无根形式, 提取盘符, 过滤 `.` 和空段, 拒绝 `..`。
///
/// 处理步骤:
/// 1. 去掉反斜杠 (统一为正斜杠语义)
/// 2. 去掉 Windows 扩展路径前缀 (`//?/` 和 `//?/UNC/`)
/// 3. 提取盘符 (如 `C:`), 存为小写字母
/// 4. 按 `/` 切分, 过滤空段和 `.` 段
/// 5. 包含 `..` 或结果为空则返回 None (防止目录穿越)
///
/// 用于安全策略中统一路径形式, 便于前缀匹配判断路径归属。
fn normalized_rootless_path_text(path: &Path) -> Option<NormalizeRootlessPath> {
    let mut text = path.to_string_lossy().replace('\\', "");
    if let Some(rest) = text.strip_prefix("//?/UNC/") {
        text = rest.to_string();
    } else if let Some(rest) = text.strip_prefix("//?/") {
        text = rest.to_string();
    }

    let mut drive = None;

    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        drive = Some(bytes[0].to_ascii_lowercase());
        text = text[2..].to_string();
    }

    let parts: Vec<&str> = text
        .trim_start_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();

    if parts.is_empty() || parts.contains(&"..") {
        None
    } else {
        Some(NormalizeRootlessPath { drive, text })
    }
}

/// 计算 path 相对于 workspace_dir 的后缀路径。
///
/// 将两者都规范化为无根形式后做前缀匹配,
/// 返回去掉 workspace 前缀后的相对路径。
/// 不同盘符直接返回 None。
fn workspace_prefixed_relative_suffix(path: &Path, workspace_dir: &Path) -> Option<PathBuf> {
    let path_text = normalized_rootless_path_text(path)?;
    let workspace_text = normalized_rootless_path_text(workspace_dir)?;

    if path_text.drive.is_some() && path_text.drive != workspace_text.drive {
        return None;
    }

    if path_text.text == workspace_text.text {
        return Some(PathBuf::new());
    }

    let prefix = format!("{}/", workspace_text.text);
    path_text
        .text
        .strip_prefix(&prefix)
        .map(|suffix| PathBuf::from(suffix.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

/// 跳过命令开头的环境变量赋值 (如 `FOO=bar BAZ=qux command`)。
///
/// 返回去掉所有前导 `KEY=value` 后的剩余命令字符串。
fn skip_env_assignments(s: &str) -> &str {
    let mut rest = s;
    loop {
        let Some(word) = rest.split_whitespace().next() else {
            return rest;
        };
        // 环境变量赋值: 包含 '=' 且以字母或下划线开头
        if word.contains('=')
            && word
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            // 跳过这个单词
            rest = rest[word.len()..].trim_start();
        } else {
            return rest;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    None,
    Single,
    Double,
}

/// 将命令按引号外的分隔符 (`;`, `|`, `&`, `\n`) 切分为段。
///
/// 引号内的分隔符不切分。支持 heredoc (`<<WORD...WORD`) 语法,
/// heredoc 内容被丢弃, 只保留起始行和后续命令。
fn split_unquoted_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = QuoteState::None;
    let mut escaped = false;
    // Heredoc 状态: 进入 heredoc 体时为 Some(delim)
    let mut heredoc_delimiter: Option<String> = None;
    // 累积 heredoc 体内的当前行, 用于终止符检测
    let mut heredoc_line_buf = String::new();
    // 正在读取 `<<` 后面的分隔符单词
    let mut reading_heredoc_word = false;
    let mut heredoc_word_buf = String::new();
    let mut chars = command.chars().peekable();

    let push_segment = |segments: &mut Vec<String>, current: &mut String| {
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            segments.push(trimmed.to_string());
        }
        current.clear();
    };

    while let Some(ch) = chars.next() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
                current.push(ch);
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    current.push(ch);
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    current.push(ch);
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                }
                current.push(ch);
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    if heredoc_delimiter.is_some() {
                        heredoc_line_buf.push(ch);
                    } else {
                        current.push(ch);
                    }
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    if heredoc_delimiter.is_some() {
                        heredoc_line_buf.push(ch);
                    } else {
                        current.push(ch);
                    }
                    continue;
                }

                // 正在读取 `<<` 后面的分隔符单词
                if reading_heredoc_word {
                    if ch == '\n' {
                        // 确定分隔符, 进入 heredoc 体
                        let raw = heredoc_word_buf.trim().trim_start_matches('-');
                        let delim = raw
                            .trim_matches(|c| c == '\'' || c == '"' || c == '\\')
                            .to_string();
                        if !delim.is_empty() {
                            heredoc_delimiter = Some(delim);
                        }
                        heredoc_word_buf.clear();
                        reading_heredoc_word = false;
                        // `<<WORD` 后的换行属于同一段
                        current.push(ch);
                    } else {
                        heredoc_word_buf.push(ch);
                        current.push(ch);
                    }
                    continue;
                }

                // 在 heredoc 体内: 不按换行切分, 丢弃内容, 只保留起始行和后续命令。
                // 这里是 heredoc 解析的唯一真相源 -- 引号感知的,
                // 所以引号内的 `<<WORD` 不会开启 heredoc, 无法隐藏后续真实路径参数。
                if let Some(delim) = heredoc_delimiter.as_deref() {
                    if ch == '\n' {
                        if heredoc_line_buf.trim() == delim {
                            // 遇到终止符行 -- heredoc 体结束
                            heredoc_delimiter = None;
                            heredoc_line_buf.clear();
                            push_segment(&mut segments, &mut current);
                        } else {
                            heredoc_line_buf.clear();
                        }
                    } else {
                        heredoc_line_buf.push(ch);
                    }
                    continue;
                }

                match ch {
                    '\'' => {
                        quote = QuoteState::Single;
                        current.push(ch);
                    }
                    '"' => {
                        quote = QuoteState::Double;
                        current.push(ch);
                    }
                    ';' | '\n' => push_segment(&mut segments, &mut current),
                    '|' => {
                        if chars.next_if_eq(&'|').is_some() {
                            // 消费完整的 `||`; 两个字符都是分隔符
                        }
                        push_segment(&mut segments, &mut current);
                    }
                    '&' => {
                        if chars.next_if_eq(&'&').is_some() {
                            // `&&` 是分隔符; 单个 `&` 单独处理
                            push_segment(&mut segments, &mut current);
                        } else {
                            current.push(ch);
                        }
                    }
                    '<' => {
                        current.push(ch);
                        // 检测 `<<` (heredoc), 但不匹配 `<<<` (here-string)
                        if chars.peek() == Some(&'<') {
                            let second = chars.next().unwrap();
                            current.push(second);
                            if chars.peek() != Some(&'<') {
                                reading_heredoc_word = true;
                            }
                            // `<<<` 不触发 heredoc 跟踪, 直接放行
                        }
                    }
                    _ => current.push(ch),
                }
            }
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }

    segments
}

/// 检测命令中是否包含引号外的单个 `&` (后台执行操作符)。
///
/// `&&` 不算, 已被消费。
fn contains_unquoted_single_ampersand(command: &str) -> bool {
    let mut quote = QuoteState::None;
    let mut escaped = false;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                }
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                match ch {
                    '\'' => quote = QuoteState::Single,
                    '"' => quote = QuoteState::Double,
                    // 必须消费第二个 '&', 防止 `&&` 被后续重新读取为单个 '&'
                    '&' if chars.next_if_eq(&'&').is_none() => {
                        return true;
                    }
                    _ => {}
                }
            }
        }
    }

    false
}

/// 检测命令中是否包含引号外的指定字符。
fn contains_unquoted_char(command: &str, target: char) -> bool {
    let mut quote = QuoteState::None;
    let mut escaped = false;

    for ch in command.chars() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                }
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                match ch {
                    '\'' => quote = QuoteState::Single,
                    '"' => quote = QuoteState::Double,
                    _ if ch == target => return true,
                    _ => {}
                }
            }
        }
    }

    false
}

/// 去掉文件描述符合并重定向 (如 `2>&1`, `1>&2`, `>&2`, `<&0`, `2<&-`, `>&-`)。
fn strip_fd_merge_redirects(command: &str) -> String {
    use std::sync::OnceLock;
    // 匹配模式: 2>&1, 1>&2, >&2, <&0, 2<&-, >&-
    static FD_MERGE_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = FD_MERGE_RE.get_or_init(|| regex::Regex::new(r"\d*[><]&[\d-]").unwrap());
    re.replace_all(command, "").to_string()
}

/// 检测命令中是否包含不安全的输出重定向 (`>`)。
///
/// 先去掉安全的重定向模式 (如 `>/dev/null`), 再去掉 fd 合并,
/// 最后检查是否还有剩余的 `>`。
fn contains_unsafe_output_redirect(command: &str) -> bool {
    // 先去掉安全的设备重定向模式 (带词边界强制), 再去掉 fd 合并, 最后检查剩余的 `>`
    use regex::Regex;
    use std::sync::OnceLock;

    static SAFE_OUTPUT_RE: OnceLock<Regex> = OnceLock::new();
    let re = SAFE_OUTPUT_RE.get_or_init(|| {
        // 匹配 >可选空格/dev/{null,zero,stdout,stderr} 后跟空白、行尾或 shell 操作符。
        // 设备名后若有 .、/ 或其他非操作符字符则不匹配 --
        // 防止 `2>/dev/stderr.log` 或 `>/dev/zero/path` 等绕过。
        // 终止符被捕获并保留在替换结果中。
        Regex::new(&format!(
            r"\d*>[ ]?/dev/({})(\s|[;&|)]|$)",
            safe_device_redirect_names_pattern()
        ))
            .unwrap()
    });

    let safe = re.replace_all(command, "$2").to_string();
    // 同时去掉 fd 合并重定向 (2>&1, 1>&2, >&N 等)
    let safe = strip_fd_merge_redirects(&safe);
    contains_unquoted_char(&safe, '>')
}

/// 检测命令中是否包含不安全的输入重定向 (`<`)。
///
/// 先去掉 here-string (`<<<`)、heredoc (`<<`) 和安全的 `/dev/*` 来源,
/// 再去掉 fd 合并, 最后检查是否还有剩余的 `<`。
fn contains_unquoted_input_redirect(command: &str) -> bool {
    // 先去掉 here-string (`<<<`), 再去掉 heredoc (`<<`), 再去掉安全的 /dev/* 来源
    // with word boundary enforcement.
    use regex::Regex;
    use std::sync::OnceLock;

    static SAFE_INPUT_RE: OnceLock<Regex> = OnceLock::new();
    let re =
        SAFE_INPUT_RE.get_or_init(|| Regex::new(r"<[ ]?/dev/(null|zero)(\s|[;&|)]|$)").unwrap());

    let safe = command.replace("<<<", "").replace("<<", "");
    let safe = re.replace_all(&safe, "$2").to_string();
    // 同时去掉 fd 合并重定向 (<&0, <&- 等), 防止留下裸的 `<`
    let safe = strip_fd_merge_redirects(&safe);
    contains_unquoted_char(&safe, '<')
}
/// 检测命令中是否包含引号外的 shell 变量展开 (`$VAR`, `${VAR}`, `$(cmd)` 等)。
fn contains_unquoted_shell_variable_expansion(command: &str) -> bool {
    let mut quote = QuoteState::None;
    let mut escaped = false;
    let chars: Vec<char> = command.chars().collect();

    for i in 0..chars.len() {
        let ch = chars[i];

        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
                continue;
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                    continue;
                }
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '\'' {
                    quote = QuoteState::Single;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::Double;
                    continue;
                }
            }
        }

        if ch != '$' {
            continue;
        }

        let Some(next) = chars.get(i + 1).copied() else {
            continue;
        };
        if next.is_ascii_alphanumeric()
            || matches!(
                next,
                '_' | '{' | '(' | '#' | '?' | '!' | '$' | '*' | '@' | '-'
            )
        {
            return true;
        }
    }

    false
}

/// 去掉 token 两端的引号 (单引号或双引号)。
fn strip_wrapping_quotes(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

/// 判断字符串是否看起来像一个路径。
///
/// 检查绝对路径、相对路径前缀 (`./`, `../`)、家目录 (`~`)、
/// 包含 `/` 的字符串, 以及 Windows 盘符/UNC 路径。
fn looks_like_path(candidate: &str) -> bool {
    candidate.starts_with('/')
        || candidate.starts_with("./")
        || candidate.starts_with("../")
        || candidate == "~"
        || candidate.starts_with("~/")
        || (candidate.starts_with('~') && candidate.contains('/'))
        || candidate == "."
        || candidate == ".."
        || candidate.contains('/')
        // Windows 路径模式: 盘符 (C:\, D:\) 和 UNC 路径 (\\server\share)
        || (cfg!(target_os = "windows")
        && (candidate
        .get(1..3)
        .is_some_and(|s| s == ":\\" || s == ":/")
        || candidate.starts_with("\\\\")))
}

/// 提取短选项附加的值 (如 `-f/etc/passwd` -> `/etc/passwd`)。
fn attached_short_option_value(token: &str) -> Option<&str> {
    // 示例:
    // -f/etc/passwd   -> /etc/passwd
    // -C../outside    -> ../outside
    // -I./include     -> ./include
    let body = token.strip_prefix('-')?;
    if body.starts_with('-') || body.len() < 2 {
        return None;
    }
    let mut chars = body.chars();
    chars.next();
    let value = chars.as_str().trim_start_matches('=').trim();
    if value.is_empty() { None } else { Some(value) }
}

/// 重定向参数解析结果。
enum RedirectionArgument<'a> {
    Target { prefix: &'a str, target: &'a str },
    NeedsNextToken { prefix: &'a str },
    FdOnly { prefix: &'a str },
    None,
}

/// 解析 token 中的重定向参数。
///
/// 提取 `>`/`<` 前的前缀和后面的目标路径或 fd。
fn parse_redirection_argument(token: &str) -> RedirectionArgument<'_> {
    let Some(marker_idx) = token.find(['<', '>']) else {
        return RedirectionArgument::None;
    };
    let prefix = token[..marker_idx].trim();
    let mut rest = &token[marker_idx + 1..];
    rest = rest.trim_start_matches(['<', '>']);
    if let Some(after_amp) = rest.strip_prefix('&') {
        let remaining = after_amp.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-');
        if remaining.is_empty() {
            return RedirectionArgument::FdOnly { prefix };
        }
    }
    rest = rest.trim_start_matches('&');
    rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        RedirectionArgument::NeedsNextToken { prefix }
    } else {
        RedirectionArgument::Target {
            prefix,
            target: trimmed,
        }
    }
}

/// 安全的重定向目标设备列表。
const SAFE_DEVICE_REDIRECT_TARGETS: [&str; 4] =
    ["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/zero"];

/// 生成正则用的安全设备名模式 (如 `null|zero|stdout|stderr`)。
fn safe_device_redirect_names_pattern() -> String {
    SAFE_DEVICE_REDIRECT_TARGETS
        .iter()
        .map(|target| target.trim_start_matches("/dev/"))
        .collect::<Vec<_>>()
        .join("|")
}

/// 判断重定向目标是否是安全设备 (去引号后匹配)。
fn is_safe_device_redirect_target(target: &str) -> bool {
    SAFE_DEVICE_REDIRECT_TARGETS.contains(&strip_wrapping_quotes(target).trim())
}

/// 从命令路径中提取 basename, 同时处理 Unix (`/`) 和 Windows (`\`) 分隔符。
///
/// 例: `C:\Git\bin\git.exe` -> `git.exe`
fn command_basename(raw: &str) -> &str {
    let after_fwd = raw.rsplit('/').next().unwrap_or(raw);
    after_fwd.rsplit('\\').next().unwrap_or(after_fwd)
}

/// 去掉 Windows 可执行文件后缀 (.exe, .cmd, .bat), 便于统一匹配白名单和风险表。
///
/// 非 Windows 平台直接返回原值。
fn strip_windows_exe_suffix(name: &str) -> &str {
    if cfg!(target_os = "windows") {
        name.strip_suffix(".exe")
            .or_else(|| name.strip_suffix(".cmd"))
            .or_else(|| name.strip_suffix(".bat"))
            .unwrap_or(name)
    } else {
        name
    }
}

/// 判断允许列表条目是否匹配给定可执行文件。
///
/// 匹配规则: 通配符 `*` 匹配所有; 路径型条目做精确匹配;
/// 命令名型条目按 basename 匹配 (Windows 下额外处理 .exe/.cmd/.bat 后缀)。
fn is_allowlist_entry_match(allowed: &str, executable: &str, executable_base: &str) -> bool {
    let allowed = strip_wrapping_quotes(allowed).trim();
    if allowed.is_empty() {
        return false;
    }

    // 显式通配符: 匹配任意命令名/路径
    if allowed == "*" {
        return true;
    }

    // 路径型白名单条目: `~` 展开后必须与可执行文件 token 精确匹配
    if looks_like_path(allowed) {
        let allowed_path = expand_user_path(allowed);
        let executable_path = expand_user_path(executable);
        return executable_path == allowed_path;
    }

    // 命令名型条目: 按 basename 匹配。
    // Windows 下额外处理: 白名单 "git" 可匹配 "git.exe" 等。
    if allowed == executable_base {
        return true;
    }

    #[cfg(target_os = "windows")]
    {
        let base_lower = executable_base.to_ascii_lowercase();
        let allowed_lower = allowed.to_ascii_lowercase();
        for ext in &[".exe", ".cmd", ".bat"] {
            if base_lower == format!("{allowed_lower}{ext}") {
                return true;
            }
            if allowed_lower == format!("{base_lower}{ext}") {
                return true;
            }
        }
    }

    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOperation {
    Read,
    Act,
}

impl SecurityPolicy {
    pub fn command_risk_level(&self, command: &str) -> CommandRiskLevel {
        let mut saw_medium = false;

        for segment in split_unquoted_segments(command) {
            let cmd_part = skip_env_assignments(&segment);
            let mut words = cmd_part.split_whitespace();
            let Some(base_raw) = words.next() else {
                continue;
            };

            let base_owned = command_basename(base_raw).to_ascii_lowercase();
            let base = strip_windows_exe_suffix(&base_raw);

            let args: Vec<String> = words.map(|w| w.to_ascii_lowercase()).collect();
            let joined_segment = cmd_part.to_ascii_lowercase();

            if matches!(
                base,
                "rm" | "mkfs" | "shutdown" | "reboot" | "halt" | "poweroff" | "sudo" | "su" | "chown"
                | "chmod" | "useradd" | "userdel" | "usermod" | "passwd" | "mount" | "umount" | "iptables"
                | "ufw" | "firewall-cmd" | "curl" | "wget" | "nc" | "ncat" | "netcat" | "scp" | "ssh" | "ftp" | "telnet"
            // Windows-specific high-risk commands
                | "del" | "rmdir" | "format" | "reg" | "net" | "runas" | "icacls"
                | "takeown" | "powershell" | "pwsh" | "wmic" | "sc" | "netsh"
            ) {
                return CommandRiskLevel::High;
            }

            if joined_segment.contains("rm -rf /")
                || joined_segment.contains("rm -fr /")
                || joined_segment.contains(":(){:|:&};:")
                || joined_segment.contains("del /s /q")
                || joined_segment.contains("rmdir /s /q")
                || joined_segment.contains("format c:")
            {
                return CommandRiskLevel::High;
            }
            let medium = match base {
                "git" => args.first().is_some_and(|verb| {
                    matches!(verb.as_str(), "commit" | "push" | "reset" | "clean"
                            | "rebase" | "merge" | "cherry-pick"
                            | "revert" | "branch"  | "switch" | "tag")
                }),
                "npm" | "pnpm" | "yarn" => args.first().is_some_and(
                    |verb| {
                        matches!(verb.as_str(), "install"| "add" | "remove" | "uninstall" | "update" | "publish")
                    }
                ),
                "cargo" => args.first().is_some_and(|verb| {
                    matches!(verb.as_str(), "add" | "remove" | "install" | "clean" | "publish")
                }),
                "touch" | "mkdir" | "mv" | "cp" | "ln"
                // Windows medium-risk equivalents
                | "copy" | "xcopy" | "robocopy" | "move" | "ren" | "rename" | "mklink" => true,
                _ => false,
            };
            saw_medium |= medium;
        }
        if saw_medium {
            CommandRiskLevel::Medium
        } else {
            CommandRiskLevel::Low
        }
    }

    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
    ) -> Result<CommandRiskLevel, String> {
        if !self.is_command_allowed(command) {
            return Err(format!("Command not allowed by secruity policy: {command}"));
        }
        let risk = self.command_risk_level(command);

        if risk == CommandRiskLevel::High {
            if self.block_high_risk_commands && !self.is_command_explicitly_allowed(command) {
                return Err("Command blocked: high-risk command is disallowed by policy".into());
            }
            if self.autonomy == AutonomyLevel::Supervised && !approved {
                return Err(
                    "Command requires explicit approval (approved=true): high-risk operation"
                        .into(),
                );
            }
        }

        if risk == CommandRiskLevel::Medium
            && self.autonomy == AutonomyLevel::Supervised
            && self.require_approval_for_medium_risk
            && !approved
        {
            return Err(
                "Command requires explicit approval (approved=true): medium-risk operation".into(),
            );
        }

        Ok(risk)
    }

    fn is_command_explicitly_allowed(&self, command: &str) -> bool {
        let segments = split_unquoted_segments(command);
        for segment in &segments {
            let cmd_part = skip_env_assignments(segment);
            let mut words = cmd_part.split_whitespace();
            let raw_executable = strip_wrapping_quotes(words.next().unwrap_or("")).trim();
            let executable = if let Some(idx) = raw_executable.find(['<', '>']) {
                &raw_executable[..idx]
            } else {
                raw_executable
            };
            let base_cmd_owned = command_basename(executable).to_ascii_lowercase();
            let base_cmd = strip_windows_exe_suffix(&base_cmd_owned);

            if base_cmd.is_empty() {
                continue;
            }

            let explicitly_listed = self.allowed_commands.iter().any(|allowed| {
                let allowed = strip_wrapping_quotes(allowed).trim();
                // Skip wildcard — it does not count as an explicit entry.
                if allowed.is_empty() || allowed == "*" {
                    return false;
                }
                is_allowlist_entry_match(allowed, executable, base_cmd)
            });

            if !explicitly_listed {
                return false;
            }
        }

        // At least one real command must be present.
        segments.iter().any(|s| {
            let s = skip_env_assignments(s.trim());
            s.split_whitespace().next().is_some_and(|w| !w.is_empty())
        })
    }


    pub fn is_command_allowed(&self, command: &str) -> bool {
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }

        let has_wildcard = self.allowed_commands.iter().any(|c| c.trim() == "*");
        if has_wildcard && !self.block_high_risk_commands {
            return true;
        }

        if command.contains('`')
            || contains_unquoted_shell_variable_expansion(command)
            || command.contains("<(")
            || command.contains(">(")
        {
            return false;
        }

        if contains_unsafe_output_redirect(command) {
            return false;
        }

        if contains_unquoted_input_redirect(command) {
            return false;
        }

        if command
            .split_whitespace()
            .any(|c| c == "tee" || c.starts_with("/tee"))
        {
            return false;
        }
        let ampersand_check = strip_fd_merge_redirects(command);
        if contains_unquoted_single_ampersand(&ampersand_check) {
            return false;
        }

        let segments = split_unquoted_segments(command);
        for segment in &segments {
            let cmd_part = skip_env_assignments(segment);

            let mut words = cmd_part.split_whitespace();
            let raw_executable = strip_wrapping_quotes(words.next().unwrap_or("")).trim();
            let executable = if let Some(idx) = raw_executable.find(['<', '>']) {
                &raw_executable[..idx]
            } else {
                raw_executable
            };

            let base_cmd_owned = command_basename(executable).to_ascii_lowercase();
            let base_cmd = strip_windows_exe_suffix(&base_cmd_owned);

            if base_cmd.is_empty() {
                continue;
            }

            if !self
                .allowed_commands
                .iter()
                .any(|c| is_allowlist_entry_match(c, executable, base_cmd))
            {
                return false;
            }

            let args_cased: Vec<String> = words.map(|w| w.to_string()).collect();
            let args: Vec<String> = args_cased.iter().map(|a| a.to_ascii_lowercase()).collect();
            if !self.is_args_safe(base_cmd, &args, &args_cased) {
                return false;
            }
        }

        segments.iter().any(|s| {
            let s = skip_env_assignments(s.trim());
            s.split_whitespace().next().is_some_and(|w| !w.is_empty())
        })
    }

    fn is_args_safe(&self, base: &str, args: &[String], args_cased: &[String]) -> bool {
        let base = base.to_ascii_lowercase();
        match base.as_str() {
            "find" => !args.iter().any(|a| a == "-exec" || a == "-ok"),
            "git" =>
                !args_cased.iter().any(|arg| arg == "-c")
                    && !args.iter().any(|arg| {
                    arg == "config"
                        || arg.starts_with("config.")
                        || arg == "alias"
                        || arg.starts_with("alias.")
                }),
            "python" | "python3" => {
                !args
                    .iter()
                    .any(|arg| arg.starts_with("-c") || arg.starts_with("-m"))
            }
            "node" => {
                !args.iter().any(|arg| {
                    arg.starts_with("-e")
                        || arg.starts_with("--eval")
                        || arg.starts_with("-p")
                        || arg.starts_with("--print")
                })
            }
            "pip" | "pip3" => {
                !args.iter().any(|arg| arg == "install" || arg == "download")
            }
            "npm" => {
                !args.iter().any(|arg| {
                    arg == "exec" || arg == "install" || arg == "i" || arg == "add" || arg == "ci"
                })
            }
            "cargo" => {
                !args.iter().any(|arg| arg == "install")
            }
            _ => true,
        }

    }

    pub fn forbidden_path_argument(&self, command: &str) -> Option<String> {
        let forbidden_candidate = |raw: &str| {
            let candidate = strip_wrapping_quotes(raw).trim();
            if candidate.is_empty() || candidate.contains("://") {
                return None;
            }
            if looks_like_path(candidate) && !self.is_path_allowed(candidate) {
                Some(candidate.to_string())
            } else {
                None
            }
        };
        let forbidden_non_redirect_candidate = |raw: &str| {
            let candidate = strip_wrapping_quotes(raw).trim();
            if candidate.is_empty() || candidate.contains("://") {
                return None;
            }
            if candidate.starts_with('-') {
                if let Some((_, value)) = candidate.split_once('=')
                    && let Some(blocked) = forbidden_candidate(value)
                {
                    return Some(blocked);
                }
                if let Some(value) = attached_short_option_value(candidate)
                    && let Some(blocked) = forbidden_candidate(value)
                {
                    return Some(blocked);
                }
                return None;
            }
            forbidden_candidate(candidate)
        };

        for segment in split_unquoted_segments(command) {
            let cmd_part = skip_env_assignments(&segment);
            let mut words = cmd_part.split_whitespace();
            let Some(executable) = words.next() else {
                continue;
            };

            let executable_redirect = parse_redirection_argument(strip_wrapping_quotes(executable));
            let mut next_is_redirect_target = false;
            // Cover inline forms like `cat</etc/passwd`.
            match executable_redirect {
                RedirectionArgument::Target { target, .. } => {
                    if !is_safe_device_redirect_target(target)
                        && let Some(blocked) = forbidden_candidate(target)
                    {
                        return Some(blocked);
                    }
                }
                RedirectionArgument::NeedsNextToken { .. } => {
                    next_is_redirect_target = true;
                }
                RedirectionArgument::FdOnly { .. } | RedirectionArgument::None => {}
            }

            for token in words {
                let candidate = strip_wrapping_quotes(token).trim();
                if candidate.is_empty() {
                    continue;
                }

                if next_is_redirect_target {
                    next_is_redirect_target = false;
                    if is_safe_device_redirect_target(candidate) {
                        continue;
                    }
                    if let Some(blocked) = forbidden_candidate(candidate) {
                        return Some(blocked);
                    }
                    continue;
                }

                if candidate.contains("://") {
                    continue;
                }

                match parse_redirection_argument(candidate) {
                    RedirectionArgument::Target { prefix, target } => {
                        if let Some(blocked) = forbidden_non_redirect_candidate(prefix) {
                            return Some(blocked);
                        }
                        if is_safe_device_redirect_target(target) {
                            continue;
                        }
                        if let Some(blocked) = forbidden_candidate(target) {
                            return Some(blocked);
                        }
                    }
                    RedirectionArgument::NeedsNextToken { prefix } => {
                        if let Some(blocked) = forbidden_non_redirect_candidate(prefix) {
                            return Some(blocked);
                        }
                        next_is_redirect_target = true;
                        continue;
                    }
                    RedirectionArgument::FdOnly { prefix } => {
                        if let Some(blocked) = forbidden_non_redirect_candidate(prefix) {
                            return Some(blocked);
                        }
                        continue;
                    }
                    RedirectionArgument::None => {}
                }

                // Handle option assignment forms like `--file=/etc/passwd`.
                if let Some(blocked) = forbidden_non_redirect_candidate(candidate) {
                    return Some(blocked);
                }
                if candidate.starts_with('-') {
                    continue;
                }
            }
        }

        None


    }

    pub fn is_path_allowed(&self, path: &str) -> bool {
        // Block null bytes (can truncate paths in C-backed syscalls)
        if path.contains('\0') {
            return false;
        }

        // Block path traversal: check for ".." as a path component
        if Path::new(path)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return false;
        }

        // Block URL-encoded traversal attempts (e.g. ..%2f)
        let lower = path.to_lowercase();
        if lower.contains("..%2f") || lower.contains("%2f..") {
            return false;
        }

        // Reject "~user" forms because the shell expands them at runtime and
        // they can escape workspace policy.
        if path.starts_with('~') && path != "~" && !path.starts_with("~/") {
            return false;
        }

        // Expand "~" for consistent matching with forbidden paths and allowlists.
        let expanded_path = expand_user_path(path);

        // The null device is always permitted regardless of workspace or
        // forbidden-path config; the rest of /dev remains blocked as usual.
        if is_null_device(&expanded_path) {
            return true;
        }

        // When workspace_only is set and the path is absolute, only allow it
        // if it falls within the workspace directory or an explicit allowed
        // root.  The workspace/allowed-root check runs BEFORE the forbidden
        // prefix list so that workspace paths under broad defaults like
        // "/home" are not rejected.  This mirrors the priority order in
        // `is_resolved_path_allowed`.
        if expanded_path.is_absolute() {
            let in_workspace = expanded_path.starts_with(&self.workspace_dir);
            let in_allowed_root = self
                .allowed_roots
                .iter()
                .any(|root| expanded_path.starts_with(root));
            // String-level safety check is shared between read and
            // write side tools, so accept paths under either grant
            // tier here. The grant-direction enforcement happens at
            // the resolved-path methods (`is_resolved_path_readable`
            // / `is_resolved_path_allowed`), which split read-only
            // and write-only entries into different code paths.
            let in_read_only_root = self
                .allowed_roots_read_only
                .iter()
                .any(|root| expanded_path.starts_with(root));
            let in_write_only_root = self
                .allowed_roots_write_only
                .iter()
                .any(|root| expanded_path.starts_with(root));

            if in_workspace || in_allowed_root || in_read_only_root || in_write_only_root {
                return true;
            }

            // Absolute path outside workspace/allowed roots — block when
            // workspace_only, or fall through to forbidden-prefix check.
            if self.workspace_only {
                return false;
            }
        }

        // Block forbidden paths using path-component-aware matching
        for forbidden in &self.forbidden_paths {
            let forbidden_path = expand_user_path(forbidden);
            if expanded_path.starts_with(forbidden_path) {
                return false;
            }
        }

        true
    }

    pub fn is_resolved_path_readable(&self, resolved: &Path) -> bool {
        // Universal POSIX device files: any operator running on Linux,
        // macOS, or BSD expects these to be readable. Adding them to
        // the per-agent config would be friction without security
        // benefit (they have no agent-relevant content).
        const POSIX_DEVICE_READS: &[&str] =
            &["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"];
        for device in POSIX_DEVICE_READS {
            if resolved == Path::new(device) {
                return true;
            }
        }

        // Workspace + read-write allowlist + read-only allowlist.
        // Inlined rather than delegating to `is_resolved_path_allowed`
        // so the write-only allowlist is intentionally NOT in scope
        // here.
        let workspace_root = self
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_dir.clone());
        if resolved.starts_with(&workspace_root) {
            return true;
        }
        for root in &self.allowed_roots {
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            if resolved.starts_with(&canonical) {
                return true;
            }
        }
        for root in &self.allowed_roots_read_only {
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            if resolved.starts_with(&canonical) {
                return true;
            }
        }
        for root in &self.allowed_roots_write_only {
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            if resolved.starts_with(&canonical) {
                return false;
            }
        }

        // Forbidden paths gate after the explicit allowlists so the
        // allowlists can coexist with broad default forbidden roots
        // such as `/home` and `/tmp`.
        for forbidden in &self.forbidden_paths {
            let forbidden_path = expand_user_path(forbidden);
            if resolved.starts_with(&forbidden_path) {
                return false;
            }
        }
        if !self.workspace_only {
            return true;
        }
        false
    }
    pub fn is_resolved_path_allowed(&self, resolved: &Path) -> bool {
        if is_null_device(resolved) {
            return true;
        }

        // Prefer canonical workspace root so `/a/../b` style config paths don't
        // cause false positives or negatives.
        let workspace_root = self
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_dir.clone());
        if resolved.starts_with(&workspace_root) {
            return true;
        }

        // Check extra allowed roots (e.g. shared skills directories) before
        // forbidden checks so explicit allowlists can coexist with broad
        // default forbidden roots such as `/home` and `/tmp`.
        for root in &self.allowed_roots {
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            if resolved.starts_with(&canonical) {
                return true;
            }
        }

        // Write-only cross-agent grants land here. The bot can write
        // under these paths but `is_resolved_path_readable` does not
        // see them — `AccessMode::Write` is one-way by design.
        for root in &self.allowed_roots_write_only {
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            if resolved.starts_with(&canonical) {
                return true;
            }
        }

        // For paths outside workspace/allowlist, block forbidden roots to
        // prevent symlink escapes and sensitive directory access.
        for forbidden in &self.forbidden_paths {
            let forbidden_path = expand_user_path(forbidden);
            if resolved.starts_with(&forbidden_path) {
                return false;
            }
        }

        // When workspace_only is disabled the user explicitly opted out of
        // workspace confinement after forbidden-path checks are applied.
        if !self.workspace_only {
            return true;
        }

        false
    }

    fn runtime_config_dirs(&self) -> Vec<PathBuf> {
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(parent) = self.config_path.as_deref().and_then(Path::parent) {
            dirs.push(canon(parent));
        }
        if let Some(parent) = self.workspace_dir.parent() {
            let dir = canon(parent);
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        dirs
    }

    pub fn is_runtime_config_path(&self, resolved: &Path) -> bool {
        let Some(file_name) = resolved.file_name().and_then(|value| value.to_str()) else {
            return false;
        };
        let is_config_name = file_name == "config.toml"
            || file_name == "config.toml.bak"
            || file_name.starts_with(".config.toml.tmp-");
        if !is_config_name {
            return false;
        }
        let Some(parent) = resolved.parent() else {
            return false;
        };
        self.runtime_config_dirs()
            .iter()
            .any(|dir| parent == dir.as_path())
    }

    pub fn runtime_config_violation_message(&self, resolved: &Path) -> String {
        format!(
            "Refusing to modify ZeroClaw runtime config/state file: {}. Use dedicated config tools or edit it manually outside the agent loop.",
            resolved.display()
        )
    }

    pub fn resolved_path_violation_message(&self, resolved: &Path) -> String {
        let guidance = if self.allowed_roots.is_empty() {
            "Add the directory to [autonomy].allowed_roots (for example: allowed_roots = [\"/absolute/path\"]), or move the file into the workspace."
        } else {
            "Add a matching parent directory to [autonomy].allowed_roots, or move the file into the workspace."
        };

        format!(
            "Resolved path escapes workspace allowlist: {}. {}",
            resolved.display(),
            guidance
        )
    }

    /// Check if autonomy level permits any action at all
    pub fn can_act(&self) -> bool {
        self.autonomy != AutonomyLevel::ReadOnly
    }

    // ── Tool Operation Gating ──────────────────────────────────────────────
    // Read operations bypass autonomy and rate checks because they have
    // no side effects. Act operations must pass both the autonomy gate
    // (not read-only) and the sliding-window rate limiter.

    /// Enforce policy for a tool operation.
    ///
    /// Read operations are always allowed by autonomy/rate gates.
    /// Act operations require non-readonly autonomy and available action budget.
    pub fn enforce_tool_operation(
        &self,
        operation: ToolOperation,
        operation_name: &str,
    ) -> Result<(), String> {
        match operation {
            ToolOperation::Read => Ok(()),
            ToolOperation::Act => {
                if !self.can_act() {
                    return Err(format!(
                        "Security policy: read-only mode, cannot perform '{operation_name}'"
                    ));
                }

                if !self.record_action() {
                    return Err("Rate limit exceeded: action budget exhausted".to_string());
                }

                Ok(())
            }
        }
    }

    /// Record an action for the current sender and check if rate-limited.
    /// Returns `true` if allowed, `false` if budget exhausted.
    pub fn record_action(&self) -> bool {
        self.tracker.record_for_current(self.max_actions_per_hour)
    }

    /// Check if the current sender would be rate-limited without recording.
    pub fn is_rate_limited(&self) -> bool {
        self.tracker
            .is_limited_for_current(self.max_actions_per_hour)
    }

    /// Resolve a user-provided path for tool use.
    ///
    /// Expands `~` prefixes and resolves relative paths against the workspace
    /// directory. This should be called **after** `is_path_allowed` to obtain
    /// the filesystem path that the tool actually operates on.
    pub fn resolve_tool_path(&self, path: &str) -> PathBuf {
        let expanded = expand_user_path(path);
        if expanded.is_absolute() {
            expanded
        } else if let Some(workspace_hint) = rootless_path(&self.workspace_dir) {
            if let Ok(stripped) = expanded.strip_prefix(&workspace_hint) {
                if stripped.as_os_str().is_empty() {
                    self.workspace_dir.clone()
                } else {
                    self.workspace_dir.join(stripped)
                }
            } else if let Some(stripped) =
                workspace_prefixed_relative_suffix(&expanded, &self.workspace_dir)
            {
                if stripped.as_os_str().is_empty() {
                    self.workspace_dir.clone()
                } else {
                    self.workspace_dir.join(stripped)
                }
            } else {
                self.workspace_dir.join(expanded)
            }
        } else {
            self.workspace_dir.join(expanded)
        }
    }

    /// Check whether the given raw path (before canonicalization)
    /// falls under an `allowed_roots` (read+write) OR
    /// `allowed_roots_write_only` entry. Tilde expansion is applied to
    /// the path before comparison. This is useful for tool-level
    /// pre-checks that want to allow absolute paths the policy
    /// explicitly permits to write.
    ///
    /// **Write-side semantics.** Use this from write-side tools
    /// (`file_write`, `git_operations`, shell). Read-side tools
    /// should use [`Self::is_under_any_allowed_root`] so a cross-agent
    /// `AccessMode::Read` grant allows the read.
    pub fn is_under_allowed_root(&self, path: &str) -> bool {
        let expanded = expand_user_path(path);
        if !expanded.is_absolute() {
            return false;
        }
        roots_contain(&self.allowed_roots, &expanded)
            || roots_contain(&self.allowed_roots_write_only, &expanded)
    }

    /// Check whether the given raw path falls under a read-only allowed
    /// root. Returns false for the read-write list; callers that want
    /// the union should use [`Self::is_under_any_allowed_root`].
    ///
    /// Populated for multi-agent: an agent's `workspace.access`
    /// entries with `AccessMode::Read` become read-only roots on the
    /// policy.
    #[must_use]
    pub fn is_under_read_only_allowed_root(&self, path: &str) -> bool {
        let expanded = expand_user_path(path);
        if !expanded.is_absolute() {
            return false;
        }
        roots_contain(&self.allowed_roots_read_only, &expanded)
    }

    /// Check whether the given raw path falls under
    /// `allowed_roots` (rw), `allowed_roots_read_only`, OR
    /// `allowed_roots_write_only`. Read-side tools (`file_read`,
    /// `glob_search`, `content_search`) call
    /// [`Self::is_resolved_path_readable`] for the resolved-path form,
    /// which intentionally excludes the write-only tier. This raw-path
    /// helper is the union of all three, used where read+write tools
    /// share an entry point and the resolved-path check splits the
    /// directionality afterward.
    #[must_use]
    pub fn is_under_any_allowed_root(&self, path: &str) -> bool {
        self.is_under_allowed_root(path) || self.is_under_read_only_allowed_root(path)
    }

    /// Verify this policy does not escalate any permission beyond
    /// `parent` (SubAgent inheritance subset check).
    ///
    /// Subset rules:
    /// - Every `allowed_roots` entry on `self` must appear on
    ///   `parent.allowed_roots`. (Read+write grants can never be
    ///   wider than the parent's read+write list.)
    /// - Every `allowed_roots_read_only` entry on `self` must appear
    ///   on `parent.allowed_roots` OR on
    ///   `parent.allowed_roots_read_only`. (A SubAgent can downgrade
    ///   a parent's rw root to read-only, but it cannot grant read
    ///   access to a path the parent could not even read.)
    /// - Every `allowed_commands` entry on `self` must appear on
    ///   `parent.allowed_commands`.
    /// - `self.workspace_only` must be `true` whenever
    ///   `parent.workspace_only` is `true`. A SubAgent cannot disable
    ///   workspace_only when the parent enforces it.
    /// - `self.max_actions_per_hour <= parent.max_actions_per_hour`
    ///   and `self.max_cost_per_day_cents <=
    ///   parent.max_cost_per_day_cents`. A SubAgent cannot raise the
    ///   parent's rate or cost ceiling.
    ///
    /// Returns `Err(EscalationViolation)` describing the first
    /// violation found. Callers should reject the spawn on `Err` so
    /// a misconfigured override never lands as a constructed policy.
    pub fn ensure_no_escalation_beyond(
        &self,
        parent: &SecurityPolicy,
    ) -> Result<(), EscalationViolation> {
        // Autonomy: child must not exceed parent. ReadOnly < Supervised
        // < Full per the AutonomyLevel ordering.
        if self.autonomy > parent.autonomy {
            return Err(EscalationViolation::AutonomyAboveParent {
                child: self.autonomy,
                parent: parent.autonomy,
            });
        }

        // Allowed roots: every child rw root must be CONTAINED in some
        // parent rw root (so a child of `/srv/app` under a parent of
        // `/srv` accepts; a child of `/srv` under a parent of
        // `/srv/app` does not). Containment, not exact equality, lets
        // the child legitimately narrow scope.
        for root in &self.allowed_roots {
            if !parent.allowed_roots.iter().any(|p| path_contains(p, root)) {
                return Err(EscalationViolation::ReadWriteRootNotInParent { path: root.clone() });
            }
        }
        for root in &self.allowed_roots_read_only {
            let in_parent_rw = parent.allowed_roots.iter().any(|p| path_contains(p, root));
            let in_parent_ro = parent
                .allowed_roots_read_only
                .iter()
                .any(|p| path_contains(p, root));
            if !in_parent_rw && !in_parent_ro {
                return Err(EscalationViolation::ReadOnlyRootNotInParent { path: root.clone() });
            }
        }
        for root in &self.allowed_roots_write_only {
            let in_parent_rw = parent.allowed_roots.iter().any(|p| path_contains(p, root));
            let in_parent_wo = parent
                .allowed_roots_write_only
                .iter()
                .any(|p| path_contains(p, root));
            if !in_parent_rw && !in_parent_wo {
                return Err(EscalationViolation::WriteOnlyRootNotInParent { path: root.clone() });
            }
        }
        for cmd in &self.allowed_commands {
            if !parent.allowed_commands.iter().any(|p| p == cmd) {
                return Err(EscalationViolation::CommandNotInParent {
                    command: cmd.clone(),
                });
            }
        }
        if parent.workspace_only && !self.workspace_only {
            return Err(EscalationViolation::WorkspaceOnlyDisabledByChild);
        }

        // Forbidden paths run the OPPOSITE direction from allowlists:
        // the parent's forbidden set must be a subset of the child's,
        // i.e. the child cannot drop a parent's forbidden entry.
        for parent_forbidden in &parent.forbidden_paths {
            if !self.forbidden_paths.iter().any(|c| c == parent_forbidden) {
                return Err(EscalationViolation::ForbiddenPathDroppedByChild {
                    path: parent_forbidden.clone(),
                });
            }
        }

        // shell_env_passthrough is a leak surface: every child entry
        // must already be on the parent's list.
        for var in &self.shell_env_passthrough {
            if !parent.shell_env_passthrough.iter().any(|p| p == var) {
                return Err(EscalationViolation::ShellEnvPassthroughExpanded {
                    variable: var.clone(),
                });
            }
        }

        if self.max_actions_per_hour > parent.max_actions_per_hour {
            return Err(EscalationViolation::MaxActionsExceeded {
                child: self.max_actions_per_hour,
                parent: parent.max_actions_per_hour,
            });
        }
        if self.max_cost_per_day_cents > parent.max_cost_per_day_cents {
            return Err(EscalationViolation::MaxCostExceeded {
                child: self.max_cost_per_day_cents,
                parent: parent.max_cost_per_day_cents,
            });
        }
        if self.shell_timeout_secs > parent.shell_timeout_secs {
            return Err(EscalationViolation::ShellTimeoutExceeded {
                child: self.shell_timeout_secs,
                parent: parent.shell_timeout_secs,
            });
        }
        if parent.block_high_risk_commands && !self.block_high_risk_commands {
            return Err(EscalationViolation::BlockHighRiskCommandsDisabledByChild);
        }
        if parent.require_approval_for_medium_risk && !self.require_approval_for_medium_risk {
            return Err(EscalationViolation::RequireApprovalDisabledByChild);
        }

        Ok(())
    }

    /// Legacy entry point: build a `SecurityPolicy` from a risk profile
    /// without a runtime profile. Budget caps default to zero (interpreted
    /// as "no enforcement"). Tests and pre-multi-agent callsites use this;
    /// production code should call `from_profiles` or `for_agent` so the
    /// runtime profile's budget caps actually take effect.
    pub fn from_risk_profile(
        risk_profile: &crate::schema::RiskProfileConfig,
        workspace_dir: &Path,
    ) -> Self {
        Self::from_profiles(risk_profile, None, workspace_dir)
    }

    /// Build a `SecurityPolicy` from a resolved risk + runtime profile pair.
    ///
    /// Authorization fields (autonomy level, allowlists, sandbox) come from
    /// the risk profile. Budget caps (`max_actions_per_hour`,
    /// `max_cost_per_day_cents`, `shell_timeout_secs`) come from the
    /// runtime profile but are enforced with parent-subset discipline on
    /// SubAgent spawn (see `ensure_no_escalation_beyond`).
    pub fn from_profiles(
        risk_profile: &crate::schema::RiskProfileConfig,
        runtime_profile: Option<&crate::schema::RuntimeProfileConfig>,
        workspace_dir: &Path,
    ) -> Self {
        // When autonomy is Full, disable workspace_only so the agent can
        // access paths outside the workspace. Forbidden-path checks still
        // apply, preventing access to sensitive system directories.
        // See issue #5463.
        let effective_workspace_only = if risk_profile.level == AutonomyLevel::Full {
            false
        } else {
            risk_profile.workspace_only
        };

        let runtime_default = crate::schema::RuntimeProfileConfig::default();
        let runtime = runtime_profile.unwrap_or(&runtime_default);

        Self {
            autonomy: risk_profile.level,
            risk_profile_name: String::new(),
            delegation_policy: risk_profile.delegation_policy.clone(),
            workspace_dir: workspace_dir.to_path_buf(),
            // Set by `for_agent` once the install root is known; the
            // profile-only constructor has no config path.
            config_path: None,
            workspace_only: effective_workspace_only,
            allowed_commands: risk_profile.allowed_commands.clone(),
            forbidden_paths: risk_profile.forbidden_paths.clone(),
            allowed_roots: risk_profile
                .allowed_roots
                .iter()
                .filter(|root| {
                    let t = root.trim();
                    !t.is_empty() && t != crate::UNSET_DISPLAY && t != "*"
                })
                .map(|root| {
                    let expanded = expand_user_path(root);
                    if expanded.is_absolute() {
                        expanded
                    } else {
                        workspace_dir.join(expanded)
                    }
                })
                .collect(),
            // RiskProfileConfig has no read-only or write-only roots
            // concept; the multi-agent runtime populates these lists
            // when it builds a per-agent policy from the
            // workspace.access map, turning `AccessMode::Read` and
            // `AccessMode::Write` entries into the corresponding
            // tiers.
            allowed_roots_read_only: Vec::new(),
            allowed_roots_write_only: Vec::new(),
            max_actions_per_hour: runtime.max_actions_per_hour,
            max_cost_per_day_cents: runtime.max_cost_per_day_cents,
            require_approval_for_medium_risk: risk_profile.require_approval_for_medium_risk,
            block_high_risk_commands: risk_profile.block_high_risk_commands,
            shell_env_passthrough: risk_profile.shell_env_passthrough.clone(),
            shell_timeout_secs: runtime.shell_timeout_secs,
            allowed_tools: if risk_profile.allowed_tools.is_empty() {
                None
            } else {
                Some(risk_profile.allowed_tools.clone())
            },
            excluded_tools: if risk_profile.excluded_tools.is_empty() {
                None
            } else {
                Some(risk_profile.excluded_tools.clone())
            },
            auto_approve: risk_profile.auto_approve.clone(),
            always_ask: risk_profile.always_ask.clone(),
            sandbox_enabled: risk_profile.sandbox_enabled,
            sandbox_backend: risk_profile.sandbox_backend.clone(),
            firejail_args: risk_profile.firejail_args.clone(),
            tracker: PerSenderTracker::new(),
        }
    }

    /// Resolve the risk + runtime profiles owned by `agent_alias` and build
    /// a `SecurityPolicy`. Bails when the agent isn't configured or when its
    /// `risk_profile` field doesn't name a configured profile — there is no
    /// global fallback, every security context is per-agent. Missing
    /// `runtime_profile` falls back to zero budgets (treated as "inherit /
    /// no enforcement"), matching the previous default when the budget
    /// fields lived on the risk profile.
    pub fn for_agent(config: &crate::schema::Config, agent_alias: &str) -> anyhow::Result<Self> {
        let risk_profile = config.risk_profile_for_agent(agent_alias).ok_or_else(|| {
            ::shadow_log::record!(
                ERROR,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Fail)
                    .with_outcome(::shadow_log::EventOutcome::Failure)
                    .with_attrs(::serde_json::json!({"agent_alias": agent_alias})),
                "SecurityPolicy::for_agent: agent has no resolvable risk_profile"
            );
            anyhow::Error::msg(format!(
                "agents.{agent_alias} has no resolvable risk_profile (load-time validation should have caught this)"
            ))
        })?;
        let runtime_profile = config.runtime_profile_for_agent(agent_alias);
        // Per-agent workspace becomes the SecurityPolicy boundary so
        // file_read/write/edit and the shell tool jail to the agent's
        // own dir, not the install-wide legacy path.
        let agent_workspace = config.agent_workspace_dir(agent_alias);
        // The per-agent workspace is the shell tool's spawn cwd and the file-tool
        // jail root. Create it here so every path that builds a per-agent policy
        // (agent loop, gateway, channels) has the directory present. A missing cwd
        // makes the shell tool's process spawn fail with ENOENT on a fresh agent.
        std::fs::create_dir_all(&agent_workspace).with_context(|| {
            format!(
                "SecurityPolicy::for_agent: failed to create agent workspace dir {}",
                agent_workspace.display()
            )
        })?;
        let mut policy = Self::from_profiles(risk_profile, runtime_profile, &agent_workspace);
        if let Some(agent_cfg) = config.agents.get(agent_alias) {
            policy.risk_profile_name = agent_cfg.risk_profile.trim().to_string();
        }
        // Protect the active runtime config from agent self-modification.
        // The per-agent workspace nests several levels under the install
        // root, so `workspace_dir.parent()` alone no longer points at the
        // directory holding `config.toml`. Record the real config path so
        // `is_runtime_config_path` guards it directly.
        policy.config_path = Some(config.config_path.clone());

        // Shared skills directory: every agent reads from
        // `<install>/shared/skills/` so the `read_skills` tool resolves
        // bundle directories no matter which bundle the agent is
        // assigned. Read-only — bundle writes go through the SkillsService
        // (gateway/CLI/TUI), not through the agent's filesystem tools.
        // Archive root (`shared/skills/_deleted/`) is excluded to keep it
        // out of agent context.
        policy
            .allowed_roots_read_only
            .push(config.shared_workspace_dir().join("skills"));

        // Cross-agent filesystem access: the agent's
        // [agents.<alias>.workspace.access] map declares which sibling
        // workspaces this agent may read or write. Resolve each
        // sibling's workspace dir and append to the appropriate
        // allowlist tier.
        if let Some(agent_cfg) = config.agents.get(agent_alias) {
            for (sibling_alias, mode) in &agent_cfg.workspace.access {
                let sibling_dir = config.agent_workspace_dir(sibling_alias.as_str());
                match mode {
                    AccessMode::Read => {
                        policy.allowed_roots_read_only.push(sibling_dir);
                    }
                    AccessMode::Write => {
                        policy.allowed_roots_write_only.push(sibling_dir);
                    }
                    AccessMode::ReadWrite => {
                        policy.allowed_roots.push(sibling_dir);
                    }
                }
            }

            // The escape-hatch flag retains its all-paths semantics —
            // agents that genuinely need to read or write outside any
            // per-agent scope opt in here. Defaults to false.
            if agent_cfg.workspace.unrestricted_filesystem {
                policy.workspace_only = false;
            }
        }

        Ok(policy)
    }

    /// Render a human-readable summary of the active security constraints
    /// suitable for injection into the LLM system prompt.
    ///
    /// Giving the LLM visibility into these constraints prevents it from
    /// wasting tokens on commands / paths that will be rejected at runtime.
    /// See issue #2404.
    pub fn prompt_summary(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();

        // Autonomy level
        let _ = writeln!(out, "**Autonomy level**: {:?}", self.autonomy);

        // Workspace constraint
        if self.workspace_only {
            let _ = writeln!(
                out,
                "**Workspace boundary**: file operations are restricted to `{}`.",
                self.workspace_dir.display()
            );
        }

        // Allowed roots
        if !self.allowed_roots.is_empty() {
            let roots: Vec<String> = self
                .allowed_roots
                .iter()
                .map(|p| format!("`{}`", p.display()))
                .collect();
            let _ = writeln!(out, "**Additional allowed paths**: {}", roots.join(", "));
        }

        // Allowed commands
        if !self.allowed_commands.is_empty() {
            let cmds: Vec<String> = self
                .allowed_commands
                .iter()
                .map(|c| format!("`{c}`"))
                .collect();
            let _ = writeln!(
                out,
                "**Allowed shell commands**: {}. \
                 You may execute these commands freely.",
                cmds.join(", ")
            );
        }

        // Forbidden paths
        if !self.forbidden_paths.is_empty() {
            let paths: Vec<String> = self
                .forbidden_paths
                .iter()
                .map(|p| format!("`{p}`"))
                .collect();
            let _ = writeln!(
                out,
                "**Forbidden paths**: {}. \
                 Avoid accessing these paths.",
                paths.join(", ")
            );
        }

        // Risk controls
        if self.block_high_risk_commands {
            let _ = writeln!(
                out,
                "Exercise caution with destructive commands (rm, kill, reboot, etc.)."
            );
        }
        if self.require_approval_for_medium_risk {
            let _ = writeln!(
                out,
                "**Medium-risk commands** require user approval before execution."
            );
        }

        // Rate limit
        let _ = writeln!(
            out,
            "**Rate limit**: max {} actions per hour per chat (each conversation has its own independent budget).",
            self.max_actions_per_hour
        );

        out
    }

}
