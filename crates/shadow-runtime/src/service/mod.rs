//! OS 服务管理模块
//!
//! 提供 `shadow service` 子命令的核心逻辑：
//! 通过 OS init 系统（systemd/launchd）管理 shadow daemon 的生命周期。
//!
//! 支持的平台：
//! - macOS: launchd (~/Library/LaunchAgents/com.shadow.daemon.plist)
//! - Linux: systemd --user (~/.config/systemd/user/shadow.service)
//!
//! 设计参考 ZeroClaw 的 crates/zeroclaw-runtime/src/service/mod.rs，
//! 但做了大幅精简：
//! - 不支持 OpenRC、Windows
//! - 不支持多 profile（只管理默认实例）
//! - 不支持 Homebrew var 目录
//! - 不创建专用系统用户

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// launchd 服务标签 / systemd unit 名称
const SERVICE_LABEL: &str = "com.shadow.daemon";
const SERVICE_BASE: &str = "shadow";

// ============================================================
// 公开 API
// ============================================================

/// 安装 OS 服务（生成 systemd unit / launchd plist）
pub fn install() -> Result<()> {
    if cfg!(target_os = "macos") {
        install_macos()
    } else if cfg!(target_os = "linux") {
        install_linux()
    } else {
        bail!("Service management is supported on macOS and Linux only")
    }
}

/// 启动服务
pub fn start() -> Result<()> {
    if cfg!(target_os = "macos") {
        start_macos()
    } else if cfg!(target_os = "linux") {
        start_linux()
    } else {
        bail!("Service management is supported on macOS and Linux only")
    }
}

/// 停止服务
pub fn stop() -> Result<()> {
    if cfg!(target_os = "macos") {
        stop_macos()
    } else if cfg!(target_os = "linux") {
        stop_linux()
    } else {
        bail!("Service management is supported on macOS and Linux only")
    }
}

/// 重启服务
pub fn restart() -> Result<()> {
    if cfg!(target_os = "macos") {
        // launchd 没有 restart，先停再启
        stop_macos()?;
        start_macos()?;
        println!("✅ Service restarted");
        Ok(())
    } else if cfg!(target_os = "linux") {
        run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run_checked(
            Command::new("systemctl").args(["--user", "restart", &format!("{SERVICE_BASE}.service")]),
        )?;
        println!("✅ Service restarted");
        Ok(())
    } else {
        bail!("Service management is supported on macOS and Linux only")
    }
}

/// 查看服务状态
pub fn status() -> Result<()> {
    if cfg!(target_os = "macos") {
        status_macos()
    } else if cfg!(target_os = "linux") {
        status_linux()
    } else {
        bail!("Service management is supported on macOS and Linux only")
    }
}

/// 查看日志
pub fn logs(lines: usize, follow: bool) -> Result<()> {
    if cfg!(target_os = "macos") {
        logs_macos(lines, follow)
    } else if cfg!(target_os = "linux") {
        logs_linux(lines, follow)
    } else {
        bail!("Service log viewing is supported on macOS and Linux only")
    }
}

/// 卸载服务（删除 unit/plist 文件）
pub fn uninstall() -> Result<()> {
    if cfg!(target_os = "macos") {
        uninstall_macos()
    } else if cfg!(target_os = "linux") {
        uninstall_linux()
    } else {
        bail!("Service management is supported on macOS and Linux only")
    }
}

/// 判断服务是否正在运行
pub fn is_running() -> bool {
    if cfg!(target_os = "macos") {
        run_capture(Command::new("launchctl").arg("list"))
            .map(|out| out.lines().any(|l| l.contains(SERVICE_LABEL)))
            .unwrap_or(false)
    } else if cfg!(target_os = "linux") {
        run_capture(
            Command::new("systemctl").args(["--user", "is-active", &format!("{SERVICE_BASE}.service")]),
        )
        .map(|out| out.trim() == "active")
        .unwrap_or(false)
    } else {
        false
    }
}

// ============================================================
// macOS (launchd) 实现
// ============================================================

/// macOS plist 文件路径: ~/Library/LaunchAgents/com.shadow.daemon.plist
pub fn macos_service_file() -> Result<PathBuf> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

/// shadow 配置目录 (~/.shadow/)
fn shadow_home() -> Result<PathBuf> {
    Ok(home_dir()?.join(".shadow"))
}

/// 获取用户 HOME 目录（不依赖外部 crate）
pub fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable is not set")
}

fn install_macos() -> Result<()> {
    let file = macos_service_file()?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    // 确保日志目录存在
    let logs_dir = shadow_home()?.join("logs");
    fs::create_dir_all(&logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");

    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let plist = render_macos_plist(&exe, &stdout, &stderr);

    fs::write(&file, plist)?;
    println!("✅ Installed launchd service: {}", file.display());
    println!("   Start with: shadow service start");
    Ok(())
}

/// 渲染 macOS launchd plist XML
fn render_macos_plist(exe: &Path, stdout: &Path, stderr: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        exe = xml_escape(&exe.display().to_string()),
        stdout = xml_escape(&stdout.display().to_string()),
        stderr = xml_escape(&stderr.display().to_string()),
    )
}

fn start_macos() -> Result<()> {
    let plist = macos_service_file()?;
    run_checked(Command::new("launchctl").arg("load").arg("-w").arg(&plist))?;
    run_checked(Command::new("launchctl").arg("start").arg(SERVICE_LABEL))?;
    println!("✅ Service started");
    Ok(())
}

fn stop_macos() -> Result<()> {
    let plist = macos_service_file()?;
    let _ = run_checked(Command::new("launchctl").arg("stop").arg(SERVICE_LABEL));
    let _ = run_checked(Command::new("launchctl").arg("unload").arg("-w").arg(&plist));
    println!("✅ Service stopped");
    Ok(())
}

fn status_macos() -> Result<()> {
    let out = run_capture(Command::new("launchctl").arg("list"))?;
    let running = out.lines().any(|line| line.contains(SERVICE_LABEL));
    println!(
        "Service: {}",
        if running {
            "✅ running/loaded"
        } else {
            "❌ not loaded"
        }
    );
    println!("Unit: {}", macos_service_file()?.display());
    Ok(())
}

fn logs_macos(lines: usize, follow: bool) -> Result<()> {
    let logs_dir = shadow_home()?.join("logs");
    let stderr_log = logs_dir.join("daemon.stderr.log");
    let stdout_log = logs_dir.join("daemon.stdout.log");

    let log_file = if stderr_log.exists() {
        stderr_log
    } else if stdout_log.exists() {
        stdout_log
    } else {
        bail!(
            "No log files found in {}. Is the service installed?",
            logs_dir.display()
        );
    };

    tail_file(&log_file, lines, follow)
}

fn uninstall_macos() -> Result<()> {
    // 先停再删
    let _ = stop_macos();
    let file = macos_service_file()?;
    if file.exists() {
        fs::remove_file(&file)
            .with_context(|| format!("Failed to remove {}", file.display()))?;
    }
    println!("✅ Service uninstalled ({})", file.display());
    Ok(())
}

// ============================================================
// Linux (systemd) 实现
// ============================================================

/// systemd unit 文件路径: ~/.config/systemd/user/shadow.service
pub fn linux_service_file() -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{SERVICE_BASE}.service")))
}

fn install_linux() -> Result<()> {
    let file = linux_service_file()?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let unit = format!(
        "[Unit]\n\
         Description=Shadow daemon\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} daemon\n\
         Restart=always\n\
         RestartSec=3\n\
         Environment=HOME=%h\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = exe.display()
    );

    fs::write(&file, unit)?;
    let _ = run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]));
    let _ = run_checked(Command::new("systemctl").args(["--user", "enable", &format!("{SERVICE_BASE}.service")]));
    println!("✅ Installed systemd user service: {}", file.display());
    println!("   Start with: shadow service start");
    Ok(())
}

fn start_linux() -> Result<()> {
    run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run_checked(
        Command::new("systemctl").args(["--user", "start", &format!("{SERVICE_BASE}.service")]),
    )?;
    println!("✅ Service started");
    Ok(())
}

fn stop_linux() -> Result<()> {
    let _ = run_checked(
        Command::new("systemctl").args(["--user", "stop", &format!("{SERVICE_BASE}.service")]),
    );
    println!("✅ Service stopped");
    Ok(())
}

fn status_linux() -> Result<()> {
    let out = run_capture(
        Command::new("systemctl").args(["--user", "is-active", &format!("{SERVICE_BASE}.service")]),
    )
    .unwrap_or_else(|_| "unknown".into());
    println!("Service state: {}", out.trim());
    println!("Unit: {}", linux_service_file()?.display());
    Ok(())
}

fn logs_linux(lines: usize, follow: bool) -> Result<()> {
    let mut args = vec![
        "--user".to_string(),
        "-u".to_string(),
        format!("{SERVICE_BASE}.service"),
        "-n".to_string(),
        lines.to_string(),
        "--no-pager".to_string(),
    ];
    if follow {
        args.push("-f".to_string());
    }
    let status = Command::new("journalctl")
        .args(&args)
        .status()
        .context("Failed to run journalctl")?;
    if !status.success() {
        bail!("journalctl exited with non-zero status");
    }
    Ok(())
}

fn uninstall_linux() -> Result<()> {
    // 先停再删
    let _ = stop_linux();
    let file = linux_service_file()?;
    if file.exists() {
        fs::remove_file(&file)
            .with_context(|| format!("Failed to remove {}", file.display()))?;
    }
    let _ = run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]));
    println!("✅ Service uninstalled ({})", file.display());
    Ok(())
}

// ============================================================
// 辅助函数
// ============================================================

/// 执行命令，检查退出码，失败时返回 stderr 内容
fn run_checked(command: &mut Command) -> Result<()> {
    let output = command.output().context("Failed to spawn command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Command failed: {}", stderr.trim());
    }
    Ok(())
}

/// 执行命令，捕获 stdout（如果 stdout 为空则取 stderr）
fn run_capture(command: &mut Command) -> Result<String> {
    let output = command.output().context("Failed to spawn command")?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    Ok(text)
}

/// XML 特殊字符转义（plist 文件用）
pub fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 用系统 tail 命令查看日志文件
pub fn tail_file(path: &Path, lines: usize, follow: bool) -> Result<()> {
    let mut args = vec!["-n".to_string(), lines.to_string()];
    if follow {
        args.push("-f".to_string());
    }
    let status = Command::new("tail")
        .args(&args)
        .arg(path)
        .status()
        .context("Failed to run tail")?;
    if !status.success() {
        bail!("tail exited with non-zero status");
    }
    Ok(())
}
