use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::sync::atomic::AtomicBool;
use shadow_config::Config;
use shadow_core::{Channel, Observer};
use crate::lark::LarkChannel;

type CronChannelRegistry = Arc<HashMap<String, Arc<dyn Channel>>>;

static CRON_CHANNEL_REGISTRY: RwLock<Option<CronChannelRegistry>> =
    RwLock::new(None);

struct ChannelNotifyObserver {
    inner: Arc<dyn Observer>,
    tx: tokio::sync::mpsc::Sender<String>,
    tool_used: AtomicBool,
}


pub async fn deliver_announcement(
    config: &Config,
    channel: &str,
    target: &str,
    thread_id: Option<String>,
    output: &str,
) -> anyhow::Result<()> {
    use shadow_core::channel::SendMessage;
    let _ = config;

    // todo 交付前扫描密钥泄漏


    let safe_output = output.to_string();

    let make_msg = |s: &str| SendMessage::new(s, target).in_thread(thread_id.clone());

    let registry_snapshot = CRON_CHANNEL_REGISTRY
        .read().unwrap_or_else(|c| c.into_inner()).clone();

    if let Some(registry) = registry_snapshot
        && let Some(ch) = registry.get(channel.to_ascii_lowercase().as_str()) {
        return ch.send(&make_msg(&safe_output)).await;
    }

    let (raw_type, alias) = channel.split_once('.').ok_or_else(|| {
        anyhow::Error::msg(format!(
            "delivery channel {channel:?} must be a dotted <type>.<alias> ref (e.g. telegram.work)"
        ))
    })?;

    let channel_type = raw_type.to_ascii_lowercase();

    let not_configured = || {
        shadow_log::record!(
            ERROR,
            shadow_log::Event::new(
                module_path!(),
                shadow_log::Action::Fail
            ).with_outcome(shadow_log::EventOutcome::Failure),
            &format!("[channels.{channel_type}.{alias}] not configured")
        )
    };

    match channel_type.as_str() {
        "lark" | "feishu" => {
            // [channels.lark.<alias>]
            let lk = config.channels.lark.get(alias).ok_or_else(|| {
                shadow_log::record!(
                    ERROR,
                    shadow_log::Event::new(module_path!(),
                    shadow_log::Action::Fail)
                    .with_outcome(shadow_log::EventOutcome::Failure),
                    &format!(
                        "[channels.lark.<alias>] not configured (cron channel \"{channel_type}.{alias}\")"
                    )
                );

                anyhow::Error::msg(
                    format!(
                        "[channels.lark.<alias>] not configured (cron channel \"{channel_type}.{alias}\")"
                    )
                )
            })?;


            if channel_type == "lark" && lk.use_feishu {
                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(),
                    shadow_log::Action::Note)
                    .with_outcome(shadow_log::EventOutcome::Unknown),
                    &format!(
                        "cron channel=\"lark.{alias}\" with [channels.lark.<alias>] use_feishu=true \
                        fallback to one-shot channel construction; perfer channel=\"feishu.{alias}\" \
                        to reuse the live Feishu handle from start_channels"
                    )
                );
            }

            let peers = config.channel_external_peers("lark", alias);
            let peer_resolver: Arc<dyn Fn() -> Vec<String> + Send + Sync> = Arc::new(move || peers.clone());

            let ch = LarkChannel::from_config(lk, alias, peer_resolver);

            shadow_core::channel::Channel::send(&ch, &make_msg(&safe_output)).await?;
        }

        other => anyhow::bail!("unsupported delivery channel: {other}, please check build feature or valid channel_type")
    }
    
    Ok(())
}