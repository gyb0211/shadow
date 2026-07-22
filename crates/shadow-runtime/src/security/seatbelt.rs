use crate::security::Sandbox;
use std::path::{Path, PathBuf};
use std::process::Command;

/// macOS Seatbelt 沙箱实现 -- 基于 macOS 内置的 `sandbox-exec` 工具。
///
/// 通过生成 Seatbelt 策略文件 (.sb) 并用 `sandbox-exec -f policy.sb` 包装命令,
/// 实现文件系统/网络/进程的隔离。默认拒绝所有, 只放行工作目录和必要系统路径。
#[derive(Debug, Clone)]
pub struct SeatbeltSandbox {
    /// 策略文件存放目录 (/tmp/shadow-seatbelt)
    policy_dir: PathBuf,
    /// 生成的策略文件路径 (每个实例唯一)
    policy_path: PathBuf,
}

impl SeatbeltSandbox {
    /// 创建沙箱实例, 绑定到指定工作目录。
    ///
    /// - workspace 为 None 时使用当前目录, 回退到 /tmp
    /// - 生成策略文件写入临时目录, 后续 wrap_command 会引用它
    pub fn with_workspace(workspace: Option<&Path>) -> std::io::Result<Self> {
        if !Self::is_installed() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "sandbox-exec not found (requires macOS)",
            ));
        }

        let policy_dir = std::env::temp_dir().join("shadow-seatbelt");
        std::fs::create_dir_all(&policy_dir)?;

        let session_id = uuid::Uuid::new_v4();
        let policy_path = policy_dir.join(format!("{session_id}.sb"));

        let workspace = workspace
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp")));
        let policy = generate_policy(&workspace);
        std::fs::write(&policy_path, &policy)?;

        Ok(Self {
            policy_dir,
            policy_path,
        })
    }

    /// 检测系统是否安装了 sandbox-exec。
    ///
    /// 先查 /usr/bin/sandbox-exec 是否存在, 不在则尝试用 no-network profile 执行 true 验证。
    fn is_installed() -> bool {
        Path::new("/usr/bin/sandbox-exec").exists()
            || Command::new("sandbox-exec")
                .arg("-n")
                .arg("no-network")
                .arg("true")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
    }
}

impl Sandbox for SeatbeltSandbox {
    /// 将原始命令包装为 `sandbox-exec -f policy.sb <原命令>` 形式。
    fn wrap_command(&self, cmd: &mut Command) -> std::io::Result<()> {
        let program = cmd.get_program().to_string_lossy().to_string();
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        let mut sandbox_cmd = Command::new("sandbox-exec");
        sandbox_cmd.arg("-f");
        sandbox_cmd.arg(&self.policy_path);
        sandbox_cmd.arg(&program);
        sandbox_cmd.args(&args);
        // 用 sandbox-exec 命令替换原始命令
        *cmd = sandbox_cmd;
        Ok(())
    }

    fn is_available(&self) -> bool {
        Self::is_installed() && self.policy_path.exists()
    }

    fn name(&self) -> &str {
        "sandbox-exec"
    }

    fn description(&self) -> &str {
        "macOS Seatbelt sandbox (built-in sandbox-exec)"
    }
}

/// 将字符串转义为 Seatbelt 策略字面量格式。
///
/// Seatbelt 的字符串字面量用双引号包裹, 需转义反斜杠、双引号和控制字符。
/// 控制字符替换为 `?` 以防破坏策略文件语法。
fn seatbelt_string_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str(r"\\"),
            '"' => escaped.push_str(r#"\""#),
            '\n' => escaped.push_str(r"\n"),
            '\r' => escaped.push_str(r"\r"),
            '\t' => escaped.push_str(r"\t"),
            c if c.is_control() => escaped.push('?'),
            c => escaped.push(c),
        }
    }
    escaped
}

/// 生成 Seatbelt 沙箱策略文件内容 (.sb 格式)。
///
/// 策略原则: 默认拒绝一切 (deny default), 然后按需放行:
/// - 进程: 允许执行、fork、自身信号
/// - 文件读: 系统路径 (/usr, /bin, /System 等) + 工作目录 + 临时目录 + 用户配置
/// - 文件写: 仅工作目录 + 临时目录 + /dev/null + /dev/tty
/// - 网络: 默认禁止, 仅允许 DNS (mDNSResponder) 和 localhost 出站连接
/// - Mach/IPC: 允许日志、通知、安全服务等基础 mach 服务
fn generate_policy(workspace: &Path) -> String {
    let workspace_str = seatbelt_string_literal(&workspace.to_string_lossy());
    format!(
        r#"(version 1)

;; 默认拒绝所有操作
(deny default)

;; ── 进程执行 ────────────────────────────────────────────────
;; 允许进程执行所需的基础操作
(allow process-exec)
(allow process-fork)
(allow signal (target self))

;; ── 文件系统读 ──────────────────────────────────────────────
;; 允许读取系统库、框架和可执行文件
(allow file-read*
    (subpath "/usr")
    (subpath "/bin")
    (subpath "/sbin")
    (subpath "/Library")
    (subpath "/System")
    (subpath "/private/var")
    (subpath "/dev")
    (subpath "/etc")
    (subpath "/Applications")
    (subpath "/opt")
    (subpath "/nix")
    (literal "/")
    (subpath "/var"))

;; 允许读取工作目录
(allow file-read* (subpath "{workspace}"))

;; 允许读取临时目录 (策略文件本身也需要)
(allow file-read* (subpath "/tmp"))
(allow file-read* (subpath "/private/tmp"))
(allow file-read*
    (regex #"^/private/var/folders/"))

;; 允许读取用户家目录下的配置文件 (如 .cargo, .gitconfig 等)
(allow file-read*
    (regex #"^/Users/[^/]+/\\."))

;; ── 文件系统写 ──────────────────────────────────────────────
;; 仅允许写入工作目录和临时目录
(allow file-write*
    (subpath "{workspace}"))
(allow file-write*
    (subpath "/tmp")
    (subpath "/private/tmp"))
(allow file-write*
    (regex #"^/private/var/folders/"))
(allow file-write* (subpath "/dev/null"))
(allow file-write* (subpath "/dev/tty"))

;; ── 网络 ────────────────────────────────────────────────────
;; 默认禁止所有网络 (继承自 deny default)
;; 仅允许 DNS 解析
(allow network-outbound
    (remote unix-socket (path-literal "/var/run/mDNSResponder")))
(allow system-socket)

;; 仅允许 localhost 出站连接 (用于本地开发服务器)
;; 注意: macOS sandbox-exec 的 (remote ip ...) 过滤器只接受
;; "localhost:*" 或 "*:port" 格式 -- 直接写 IP 地址会导致整个策略解析失败
(allow network-outbound
    (remote ip "localhost:*"))

;; ── Mach / IPC ─────────────────────────────────────────────
;; 允许进程执行所需的基础 mach 服务
(allow mach-lookup
    (global-name "com.apple.system.logger")
    (global-name "com.apple.system.notification_center")
    (global-name "com.apple.SecurityServer")
    (global-name "com.apple.CoreServices.coreservicesd"))

;; ── Sysctl / 其他 ───────────────────────────────────────────
(allow sysctl-read)
(allow mach-task-name)
"#,
        workspace = workspace_str,
    )
}
