//! 守护进程前台运行逻辑
//!
//! 处理 `shadow daemon` 命令 -- 前台运行 channel 监听服务。
//!
//! 设计说明：
//! - daemon 命令是纯前台运行，直接阻塞在 tokio 主循环里
//! - 进程守护交给 OS init 系统（systemd/launchd），通过 `shadow service` 管理
//! - 参考ZeroClaw/Hermes的设计：应用只管业务逻辑，OS管进程生命周期
//!
//! 信号处理：
//! - SIGINT / Ctrl+C -> 优雅关闭（abort 所有 task）
//! - SIGTERM -> 优雅关闭
//! - SIGHUP -> 忽略（防止 SSH 断连杀死进程）
//!
//! channel 处理：
//! - 所有 channel 类型统一通过 collect_configured_channels 收集为 Vec<Arc<dyn Channel>>
//! - run() 不关心具体 channel 类型（Lark/Telegram/Discord...）
//! - 全局共享 mpsc，所有 channel 的 listen 写入同一个 tx
//! - 单一中央 dispatch loop 消费 rx，处理消息

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use shadow_channels::orchestrator::collect_configured_channels;
use shadow_config::Config;
use shadow_core::{Channel, ChannelMessage};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// daemon 命令参数
#[derive(Debug, Clone, Default)]
pub struct DaemonArgs {
    /// 指定 agent 名称（不指定则启动所有 enabled 的 agents）
    pub agent: Option<String>,
    /// 指定 channel 名称（不指定则监听所有配置的）
    pub name: Option<String>,
}

/// 从配置和参数中收集要启动的 agent 名称列表
///
/// - 如果指定了 `--agent`，只返回该 agent（不存在则报错）
/// - 否则返回所有 `enabled = true` 的 agents
/// - 如果列表为空，报错
pub fn collect_agents(config: &Config, agent_filter: &Option<String>) -> Result<Vec<String>> {
    let agents: Vec<String> = match agent_filter {
        Some(a) => {
            if config.agent(a).is_none() {
                anyhow::bail!(
                    "`shadow daemon --agent {a}` is not configured (no [agents.{a}] entry)"
                );
            }
            vec![a.clone()]
        }
        None => config
            .agents
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(name, _)| name.clone())
            .collect(),
    };

    if agents.is_empty() {
        anyhow::bail!("No enabled agents found. Configure agents in config file.");
    }

    Ok(agents)
}

/// 前台运行 daemon -- 启动 channel 监听，阻塞直到收到退出信号
///
/// 这是 `shadow daemon` 命令的核心入口。
/// systemd/launchd 的 ExecStart 就是调这个。
pub async fn run(config: Config, args: DaemonArgs) -> Result<()> {
    // 收集要启动的 agents
    let agents = collect_agents(&config, &args.agent)?;

    // 收集所有已配置的 channel 实例（统一为 Arc<dyn Channel>）
    let configured = collect_configured_channels(&config)?;

    info!(
        agents = ?agents,
        channels = ?configured.iter().map(|cc| format!("{}:{}", cc.display_name, cc.alias)).collect::<Vec<_>>(),
        "Shadow daemon starting (foreground)"
    );

    // 构建 alias -> channel 的映射表，供 dispatch loop 回复消息时查找
    let channel_map: HashMap<String, Arc<dyn Channel>> = configured
        .iter()
        .map(|cc| (cc.alias.clone(), cc.channel.clone()))
        .collect();

    for cc in &configured {
        println!("  ✓ {} channel '{}'", cc.display_name, cc.alias);
    }
    println!("Agents: {:?}", agents);
    println!("\nPress Ctrl+C to stop.\n");

    // 全局共享 mpsc -- 所有 channel 的 listen 写入同一个 tx
    let (tx, rx) = mpsc::channel::<ChannelMessage>(64);

    // 为每个 channel 启动 listen task
    let mut listen_handles = vec![];
    for cc in &configured {
        let tx = tx.clone();
        let ch = cc.channel.clone();
        let alias = cc.alias.clone();
        let display_name = cc.display_name.clone();
        let handle = tokio::spawn(async move {
            info!(alias = %alias, "Channel listener started");
            if let Err(e) = ch.listen(tx).await {
                error!(alias = %alias, "{display_name} channel listen error: {e}");
            }
            warn!(alias = %alias, "Channel listener exited");
        });
        listen_handles.push(handle);
    }
    // drop 掉原始 tx，所有 sender 都在 listener 手里
    // 当所有 listener 结束时 rx 会收到 None，dispatch loop 自然退出
    drop(tx);

    // 启动单一中央消息处理 loop
    let config_clone = config.clone();
    let agents_clone = agents.clone();
    let channel_map_clone = channel_map.clone();
    let dispatch_handle = tokio::spawn(async move {
        dispatch_loop(rx, &config_clone, &agents_clone, &channel_map_clone).await;
    });

    // 等待退出信号（SIGINT / SIGTERM）
    wait_for_exit_signal().await;
    info!("Received exit signal, shutting down...");
    println!("\nShutting down...");

    // 取消所有任务
    for h in &listen_handles {
        h.abort();
    }
    dispatch_handle.abort();

    info!("Shadow daemon stopped");
    Ok(())
}

/// 单一中央消息处理 loop
///
/// 消费所有 channel 的入站消息，调用 agent runtime 处理，然后回复。
async fn dispatch_loop(
    mut rx: mpsc::Receiver<ChannelMessage>,
    config: &Config,
    agents: &[String],
    channel_map: &HashMap<String, Arc<dyn Channel>>,
) {
    while let Some(msg) = rx.recv().await {
        info!(
            channel = %msg.channel,
            sender = %msg.sender,
            target = %msg.reply_target,
            "[{}] {} -> {}: {}",
            msg.channel, msg.sender, msg.reply_target, msg.content
        );

        // 使用第一个 agent 处理消息（简单策略，后续可改为路由）
        let agent_name = &agents[0];

        // 调用 agent runtime 处理消息
        let reply = match crate::agent::run(
            config.clone(),
            agent_name,
            Some(msg.content.clone()),
            None,
            false,
            None,
            None,
            crate::agent::AgentRuntimeOverrides::default(),
        )
        .await
        {
            Ok(reply) => reply,
            Err(e) => {
                error!("Agent runtime error: {e}");
                format!("Error: {e}")
            }
        };

        // 发送回复 -- 通过 channel alias 找到对应的 channel 实例
        let reply_channel = msg.channel_alias.as_deref().unwrap_or(&msg.channel);
        if let Some(ch) = channel_map.get(reply_channel) {
            let send_msg = shadow_core::SendMessage::new(reply, &msg.reply_target);
            if let Err(e) = ch.send(&send_msg).await {
                error!("Failed to send reply: {e}");
            }
        } else {
            // fallback: 尝试用 msg.channel 字段匹配
            if let Some(ch) = channel_map.get(&msg.channel) {
                let send_msg = shadow_core::SendMessage::new(reply, &msg.reply_target);
                if let Err(e) = ch.send(&send_msg).await {
                    error!("Failed to send reply: {e}");
                }
            } else {
                error!(
                    "Cannot find channel '{}' or '{}' in map to send reply",
                    reply_channel, msg.channel
                );
            }
        }
    }
}

/// 等待退出信号
///
/// - SIGINT (Ctrl+C) -> 关闭
/// - SIGTERM -> 关闭
/// - SIGHUP -> 忽略（防止 SSH 断连杀死进程）
#[cfg(unix)]
async fn wait_for_exit_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut sighup = signal(SignalKind::hangup()).expect("install SIGHUP handler");

    loop {
        tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT, shutting down...");
                break;
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down...");
                break;
            }
            _ = sighup.recv() => {
                info!("Received SIGHUP, ignoring (SSH disconnect protection)");
                // 忽略，继续等待
            }
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_exit_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("Received Ctrl+C, shutting down...");
}
