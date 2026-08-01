//! 守护进程（daemon）模块
//!
//! 提供 `shadow daemon` 命令的核心逻辑：前台运行 channel 监听服务。
//!
//! 设计参考 ZeroClaw / Hermes 的架构：
//! - daemon 命令是纯前台运行，不做 fork / setsid
//! - 进程守护交给 OS init 系统（systemd/launchd）
//! - 通过 `shadow service` 子命令管理 OS service
//!
//! channel 处理：
//! - run() 不关心具体 channel 类型，只跟 Arc<dyn Channel> 打交道
//! - channel 收集逻辑在 shadow_channels::orchestrator::collect_configured_channels
//! - 新增 channel 类型时只需改 collect_configured_channels，不需要改 runner

pub mod runner;

pub use runner::{collect_agents, run, DaemonArgs};
