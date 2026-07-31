//! WebSocket 监听 + Protobuf 帧 + WS 响应类型

use super::LarkChannel;
use super::event::{LarkEvent, parse_list_content, parse_post_content_details};
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use shadow_core::{ChannelMessage, Role};

const WS_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(300);

// ─── Protobuf 帧 ──────────────────────────────────────────

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct PbHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct PbFrame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<PbHeader>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
}

impl PbFrame {
    pub(super) fn header_value<'a>(&'a self, key: &str) -> &'a str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }
}

// ─── WS 端点响应类型 ──────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct WsEndpointResp {
    pub(super) code: i32,
    #[serde(default)]
    pub(super) msg: Option<String>,
    #[serde(default)]
    pub(super) data: Option<WsEndpoint>,
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
pub(super) struct WsClientConfig {
    #[serde(rename = "PingInterval")]
    pub(super) ping_interval: Option<u64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct WsEndpoint {
    #[serde(rename = "URL")]
    pub(super) url: String,
    #[serde(rename = "ClientConfig")]
    pub(super) client_config: Option<WsClientConfig>,
}

// ─── WS 辅助 ──────────────────────────────────────────────

fn should_refresh_last_recv(msg: &WsMsg) -> bool {
    matches!(msg, WsMsg::Binary(_) | WsMsg::Ping(_) | WsMsg::Pong(_))
}

// ─── LarkChannel WS 方法 ──────────────────────────────────

impl LarkChannel {
    pub(super) async fn get_ws_endpoint(&self) -> anyhow::Result<(String, WsClientConfig)> {
        let resp = self
            .http_client()
            .post(format!("{}/callback/ws/endpoint", self.ws_base()))
            .header("locale", self.platform.locale_header())
            .json(&serde_json::json!({
                "AppID": self.app_id,
                "AppSecret": self.app_secret,
            }))
            .send()
            .await?
            .json::<WsEndpointResp>()
            .await?;

        if resp.code != 0 {
            anyhow::bail!(
                "WS endpoint failed: code={} msg:{}",
                resp.code,
                resp.msg.as_deref().unwrap_or("(none)")
            );
        }

        let endpoint = resp
            .data
            .ok_or_else(|| anyhow::Error::msg("WS endpoint: empty data"))?;
        Ok((endpoint.url, endpoint.client_config.unwrap_or_default()))
    }

    pub(super) async fn listen_ws(&self, rx: Sender<ChannelMessage>) -> anyhow::Result<()> {
        self.ensure_bot_open_id().await;
        let (wss_url, client_config) = self.get_ws_endpoint().await?;
        let service_id = wss_url
            .split('?')
            .nth(1)
            .and_then(|qs| {
                qs.split('&')
                    .find(|kv| kv.starts_with("service_id="))
                    .and_then(|kv| kv.split('=').nth(1))
                    .and_then(|v| v.parse::<i32>().ok())
            })
            .unwrap_or(0);

        shadow_log::record!(
            INFO,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_attrs(serde_json::json!({"wss_url": wss_url})),
            "connecting to"
        );

        let (ws_stream, _) = shadow_config::ws_connect_with_proxy(
            &wss_url,
            "channel.lark",
            self.proxy_url.as_deref(),
        )
        .await?;

        let (mut write, mut read) = ws_stream.split();
        shadow_log::record!(
            INFO,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_attrs(serde_json::json!({"service_id": service_id})),
            "Ws connected (service_id=)"
        );

        let mut ping_secs = client_config.ping_interval.unwrap_or(120).max(0);
        let mut hb_interval = tokio::time::interval(Duration::from_secs(ping_secs));
        let mut timeout_check = tokio::time::interval(Duration::from_secs(10));
        hb_interval.tick().await;

        let mut seq: u64 = 0;
        let mut last_recv = Instant::now();

        seq = seq.wrapping_add(1);
        let initial_ping = PbFrame {
            seq_id: seq,
            log_id: 0,
            service: service_id,
            method: 0,
            headers: vec![PbHeader {
                key: "type".to_string(),
                value: "ping".to_string(),
            }],
            payload: None,
        };

        if write
            .send(WsMsg::Binary(initial_ping.encode_to_vec().into()))
            .await
            .is_err()
        {
            anyhow::bail!("initial ping failed");
        }

        type FragEntry = (Vec<Option<Vec<u8>>>, Instant);
        let mut frag_cache: HashMap<String, FragEntry> = HashMap::new();

        loop {
            tokio::select! {
                biased;

                _ = hb_interval.tick() => {
                    seq = seq.wrapping_add(1);
                    let ping = PbFrame {
                        seq_id: seq,
                        log_id: 0,
                        service: service_id,
                        method: 0,
                        headers: vec![PbHeader {
                            key: "type".to_string(),
                            value: "ping".to_string(),
                        }],
                        payload: None,
                    };
                    if write
                        .send(WsMsg::Binary(ping.encode_to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }

                    let cutoff = Instant::now()
                        .checked_sub(Duration::from_secs(300))
                        .unwrap_or(Instant::now());

                    frag_cache.retain(|_, (_, ts)| *ts > cutoff);
                }

                _ = timeout_check.tick() => {
                    if last_recv.elapsed() > WS_HEARTBEAT_TIMEOUT {
                        break;
                    }
                }

                msg = read.next() => {
                    let raw = match msg {
                        Some(Ok(ws_msg)) => {
                            if should_refresh_last_recv(&ws_msg){
                                last_recv = Instant::now();
                            }
                            match ws_msg {
                                WsMsg::Binary(b) => b,
                                WsMsg::Ping(p) => {let _ = write.send(WsMsg::Pong(p)).await; continue;}
                                WsMsg::Close(_) => {
                                    shadow_log::record!(
                                        INFO,
                                        shadow_log::Event::new(module_path!(), shadow_log::Action::Note),
                                        "Ws closed — reconnecting"
                                    );
                                    break;
                                }
                                _=> continue,
                            }
                        }
                        None => {
                            shadow_log::record!(
                                INFO,
                                shadow_log::Event::new(module_path!(), shadow_log::Action::Note),
                                "Ws closed — reconnecting"
                            );
                            break;
                        }
                        Some(Err(e)) => {
                            shadow_log::record!(
                                ERROR,
                                shadow_log::Event::new(module_path!(), shadow_log::Action::Fail)
                                    .with_outcome(shadow_log::EventOutcome::Failure)
                                    .with_attrs(serde_json::json!({"error": e.to_string()})),
                                "Ws closed — reconnecting"
                            );
                            break;
                        }
                    };

                    let frame = match PbFrame::decode(&raw[..]) {
                        Ok(f) => f,
                        Err(e) => {
                            shadow_log::record!(
                                ERROR,
                                shadow_log::Event::new(module_path!(), shadow_log::Action::Fail)
                                    .with_outcome(shadow_log::EventOutcome::Failure)
                                    .with_attrs(serde_json::json!({"error": e.to_string()})),
                                "proto decode"
                            );
                            continue;
                        }
                    };

                    if frame.method == 0 {
                        if frame.header_value("type") == "pong"
                        && let Some(p) = &frame.payload
                        && let Ok(cfg) = serde_json::from_slice::<WsClientConfig>(p)
                        && let Some(secs) = cfg.ping_interval {
                            let secs = secs.max(10);
                            if secs != ping_secs {
                                ping_secs = secs;
                                hb_interval = tokio::time::interval(Duration::from_secs(ping_secs));
                                shadow_log::record!(
                                    INFO,
                                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note).with_attrs(serde_json::json!({"ping_secs": ping_secs})),
                                    "ping interval -> s"
                                );
                                continue;
                            }
                        }
                    }

                    // DATA frame
                    let msg_type = frame.header_value("type").to_string();
                    let msg_id   = frame.header_value("message_id").to_string();
                    let sum      = frame.header_value("sum").parse::<usize>().unwrap_or(1);
                    let seq_num  = frame.header_value("seq").parse::<usize>().unwrap_or(0);

                    {
                        let mut ack = frame.clone();
                        ack.payload = Some(br#"{"code":200,"headers":{},"data":[]}"#.to_vec());
                        ack.headers.push(PbHeader { key: "biz_rt".into(), value: "0".into() });
                        let _ = write.send(WsMsg::Binary(ack.encode_to_vec().into())).await;
                    }

                    let sum = if sum == 0 { 1 } else { sum };
                    let payload: Vec<u8> = if sum == 1 || msg_id.is_empty() || seq_num >= sum {
                        frame.payload.clone().unwrap_or_default()
                    } else {
                        let entry = frag_cache
                            .entry(msg_id.clone())
                            .or_insert_with(|| (vec![None; sum], Instant::now()));
                        if entry.0.len() != sum {
                            *entry = (vec![None; sum], Instant::now());
                        }
                        entry.0[seq_num] = frame.payload.clone();
                        if entry.0.iter().all(|s| s.is_some()) {
                            let full: Vec<u8> = entry
                                .0
                                .iter()
                                .flat_map(|s| s.as_deref().unwrap_or(&[]))
                                .copied()
                                .collect();
                            frag_cache.remove(&msg_id);
                            full
                        } else {
                            continue;
                        }
                    };

                    if msg_type != "event" {
                        continue;
                    }

                    let event: LarkEvent = match serde_json::from_slice(&payload) {
                        Ok(e) => e,
                        Err(e) => {
                            ::shadow_log::record!(ERROR, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Fail).with_outcome(::shadow_log::EventOutcome::Failure).with_attrs(::serde_json::json!({"error": format!("{}", e)})), "event JSON");
                            continue;
                        }
                    };
                    match event.header.event_type.as_str() {
                        "im.message.receive_v1" => {}
                        "card.action.trigger" => {
                            if let Err(e) = self.handle_card_action_event(&event.event).await {
                                ::shadow_log::record!(
                                    WARN,
                                    ::shadow_log::Event::new(
                                        module_path!(),
                                        ::shadow_log::Action::Dispatch
                                    )
                                    .with_outcome(::shadow_log::EventOutcome::Failure)
                                    .with_attrs(::serde_json::json!({"error": e.to_string()})),
                                    "Lark WS: card action dispatch error"
                                );
                            }
                            continue;
                        }
                        _ => continue,
                    }

                    let event_payload = event.event;

                    let recv: super::event::MsgReceivePayload = match serde_json::from_value(event_payload.clone()) {
                        Ok(r) => r,
                        Err(e) => {
                            ::shadow_log::record!(ERROR, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Fail).with_outcome(::shadow_log::EventOutcome::Failure).with_attrs(::serde_json::json!({"error": format!("{}", e)})), "payload parse");
                            continue;
                        }
                    };

                    if recv.sender.sender_type == "app" || recv.sender.sender_type == "bot" { continue; }

                    let sender_open_id = recv.sender.sender_id.open_id.as_deref().unwrap_or("");
                    if !self.is_user_allowed(sender_open_id) {
                        ::shadow_log::record!(WARN, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note).with_outcome(::shadow_log::EventOutcome::Unknown).with_attrs(::serde_json::json!({"sender_open_id": sender_open_id})), "WS: ignoring (not in peer group)");
                        continue;
                    }

                    let lark_msg = &recv.message;

                    // Dedup
                    {
                        let now = Instant::now();
                        let mut seen = self.ws_seen_ids.write().await;
                        seen.retain(|_, t| now.duration_since(*t) < Duration::from_secs(30 * 60));
                        if seen.contains_key(&lark_msg.message_id) {
                            ::shadow_log::record!(DEBUG, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note), &format!("WS: dup {}", lark_msg.message_id));
                            continue;
                        }
                        seen.insert(lark_msg.message_id.clone(), now);
                    }

                    // Decode content by type
                    let (text, _post_mentioned_open_ids) = match lark_msg.message_type.as_str() {
                        "text" => {
                            let v: serde_json::Value = match serde_json::from_str(&lark_msg.content) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            match v.get("text").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) {
                                Some(t) => (t.to_string(), Vec::new()),
                                None => continue,
                            }
                        }
                        "post" => match parse_post_content_details(&lark_msg.content) {
                            Some(details) => (details.text, details.mentioned_open_ids),
                            None => continue,
                        },
                        "image" => {
                            let v: serde_json::Value = match serde_json::from_str(&lark_msg.content) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let image_key = match v.get("image_key").and_then(|k| k.as_str()) {
                                Some(k) => k.to_string(),
                                None => {
                                    ::shadow_log::record!(DEBUG, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note), "WS: image message missing image_key");
                                    continue;
                                }
                            };
                            match self.download_image_as_marker(&lark_msg.message_id, &image_key).await {
                                Some(marker) => (marker, Vec::new()),
                                None => {
                                    ::shadow_log::record!(WARN, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note).with_outcome(::shadow_log::EventOutcome::Unknown).with_attrs(::serde_json::json!({"image_key": image_key})), "WS: failed to download image");
                                    (format!("[IMAGE:{image_key} | download failed]"), Vec::new())
                                }
                            }
                        }
                        "file" => {
                            let v: serde_json::Value = match serde_json::from_str(&lark_msg.content) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let file_key = match v.get("file_key").and_then(|k| k.as_str()) {
                                Some(k) => k.to_string(),
                                None => {
                                    ::shadow_log::record!(DEBUG, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note), "WS: file message missing file_key");
                                    continue;
                                }
                            };
                            let file_name = v.get("file_name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown_file")
                                .to_string();
                            match self.download_file_as_content(&lark_msg.message_id, &file_key, &file_name).await {
                                Some(content) => (content, Vec::new()),
                                None => {
                                    ::shadow_log::record!(WARN, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note).with_outcome(::shadow_log::EventOutcome::Unknown).with_attrs(::serde_json::json!({"file_key": file_key})), "WS: failed to download file");
                                    (format!("[ATTACHMENT:{file_name} | download failed]"), Vec::new())
                                }
                            }
                        }
                        "audio" => {
                            let Some(manager) = self.transcription_manager.as_ref() else {
                                ::shadow_log::record!(DEBUG, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note), &format!("WS: audio message in {} (transcription not configured)", lark_msg.chat_id));
                                continue;
                            };
                            let transcript = self.try_transcribe_audio_message(
                                &lark_msg.message_id,
                                &lark_msg.content,
                                manager,
                            ).await;
                            let Some(text) = transcript else { continue; };
                            (text, Vec::new())
                        }
                        "list" => match parse_list_content(&lark_msg.content) {
                            Some(t) => (t, Vec::new()),
                            None => {
                                ::shadow_log::record!(DEBUG, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note), "WS: list message with no extractable text");
                                continue;
                            }
                        },
                        _ => {
                            ::shadow_log::record!(DEBUG, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note), &format!("WS: skipping unsupported type '{}'", lark_msg.message_type));
                            continue;
                        }
                    };

                    // Build ChannelMessage and send to rx
                    let channel_msg = ChannelMessage {
                        id: lark_msg.message_id.clone(),
                        sender: sender_open_id.to_string(),
                        content: text,
                        reply_target: lark_msg.chat_id.clone(),
                        channel: self.channel_name().to_string(),
                        channel_alias: Some(self.alias.clone()),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        thread_ts: None,
                        interruption_scope_id: None,
                        attachments: vec![],
                        subject: None,
                        internal_sop_event: None,
                        passive_context: false,
                        explicitly_addressed: false,
                        conversation_scope: shadow_core::channel::ChannelConversationScope::default(),
                    };

                    if rx.send(channel_msg).await.is_err() {
                        ::shadow_log::record!(
                            WARN,
                            ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                                .with_outcome(::shadow_log::EventOutcome::Failure),
                            "WS: rx channel closed, stopping listener"
                        );
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}
