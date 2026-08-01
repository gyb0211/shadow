//! LarkChannel 核心结构体 + 构造函数 + Channel/Attributable trait 实现

use super::approval::PendingApproval;
use super::card::{
    LARK_CARD_MARKDOWN_MAX_BYTES, build_interactive_card_body, split_markdown_chunks,
};
use super::media::{lark_outgoing_media_from_marker, resolve_lark_media_marker};
use super::platform::LarkPlatform;
use super::token::CachedTenantToken;
use crate::transcription::TranscriptionManager;
use crate::utils;
use async_trait::async_trait;
use shadow_config::LarkConfig;
use shadow_config::channel::{LarkReceiveMode, StreamMode};
use shadow_core::{Attributable, Channel, ChannelKind, ChannelMessage, Role, SendMessage};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Instant;
use tokio::sync::mpsc::Sender;
use tokio::sync::{Mutex, RwLock};

pub struct LarkChannel {
    pub(super) app_id: String,
    pub(super) app_secret: String,
    pub(super) verification_token: String,
    pub(super) port: Option<u16>,
    pub(super) alias: String,
    pub(super) peer_resolver: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    pub(super) resolved_bot_open_id: Arc<StdRwLock<Option<String>>>,
    pub(super) mention_only: bool,
    pub(super) platform: LarkPlatform,
    pub(super) receive_mode: LarkReceiveMode,
    pub(super) tenant_token: Arc<RwLock<Option<CachedTenantToken>>>,
    pub(super) ws_seen_ids: Arc<RwLock<HashMap<String, Instant>>>,
    pub(super) proxy_url: Option<String>,
    pub(super) workspace_dir: Option<PathBuf>,
    pub(super) transcription: Option<shadow_config::channel::TranscriptionConfig>,
    pub(super) transcription_manager: Option<Arc<TranscriptionManager>>,
    pub(super) pending_approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
    pub(super) approval_timeout_secs: u64,
    pub(super) per_user_session: bool,
    pub(super) ack_reactions: bool,
    pub(super) reaction_ids: Arc<Mutex<HashMap<(String, String), String>>>,
    pub(super) stream_mode: StreamMode,
    pub(super) draft_update_interval_ms: u64,
    pub(super) last_draft_edit: Arc<Mutex<HashMap<String, Instant>>>,
}

impl LarkChannel {
    fn new_with_platform(
        app_id: String,
        app_secret: String,
        verification_token: String,
        port: Option<u16>,
        alias: impl Into<String>,
        peer_resolver: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
        mention_only: bool,
        platform: LarkPlatform,
    ) -> Self {
        Self {
            app_id,
            app_secret,
            verification_token,
            port,
            alias: alias.into(),
            peer_resolver,
            resolved_bot_open_id: Arc::new(Default::default()),
            mention_only,
            platform,
            receive_mode: Default::default(),
            tenant_token: Arc::new(Default::default()),
            ws_seen_ids: Arc::new(Default::default()),
            proxy_url: None,
            workspace_dir: None,
            transcription: None,
            transcription_manager: None,
            pending_approvals: Arc::new(Default::default()),
            approval_timeout_secs: 120,
            per_user_session: false,
            ack_reactions: false,
            reaction_ids: Arc::new(Default::default()),
            stream_mode: Default::default(),
            draft_update_interval_ms: 100,
            last_draft_edit: Arc::new(Default::default()),
        }
    }

    pub fn from_config(
        config: &LarkConfig,
        alias: impl Into<String>,
        peer_resolver: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    ) -> Self {
        let platform = if config.use_feishu {
            LarkPlatform::Feishu
        } else {
            LarkPlatform::Lark
        };

        let mut ch = Self::new_with_platform(
            config.app_id.clone(),
            config.app_secret.clone(),
            config.verification_token.clone().unwrap_or_default(),
            config.port,
            alias,
            peer_resolver,
            config.mention_only,
            platform,
        );

        ch.receive_mode = config.receive_mode.clone();
        ch.proxy_url = config.proxy_url.clone();
        ch
    }

    /// 尝试启用本地 whisper.cpp 语音转文本
    ///
    /// 成功则设置 transcription_manager，失败则返回错误（channel 状态不变）
    pub fn try_enable_local_transcription(&mut self) -> anyhow::Result<()> {
        match crate::transcription::TranscriptionManager::with_local_whisper() {
            Ok(manager) => {
                self.transcription_manager = Some(Arc::new(manager));
                ::shadow_log::record!(
                    INFO,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note),
                    "local whisper transcription enabled for lark channel"
                );
                Ok(())
            }
            Err(e) => {
                Err(anyhow::anyhow!("Failed to init local whisper: {}", e))
            }
        }
    }

    /// 启用本地 whisper.cpp 语音转文本（builder 风格）
    pub fn with_local_transcription(mut self) -> anyhow::Result<Self> {
        self.try_enable_local_transcription()?;
        Ok(self)
    }

    fn resolve_sender<'a>(&self, chat_id: &'a str, sender_open_id: Option<&'a str>) -> &'a str {
        if self.per_user_session {
            match sender_open_id {
                Some(oid) if !oid.is_empty() => oid,
                _ => chat_id,
            }
        } else {
            chat_id
        }
    }
}

impl Attributable for LarkChannel {
    fn role(&self) -> Role {
        Role::Channel(ChannelKind::Lark)
    }

    fn alias(&self) -> &str {
        &self.alias
    }
}

#[async_trait]
impl Channel for LarkChannel {
    fn name(&self) -> &str {
        self.channel_name()
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let mut token = self.get_tenant_access_token().await?;
        let url = self.send_message_url();
        let (text_content, raw_markers) = utils::parse_attachment_markers(&message.content);
        let markers = raw_markers
            .into_iter()
            .filter_map(|(kind, target)| lark_outgoing_media_from_marker(kind, target))
            .collect::<Vec<_>>();

        let resolved_markers = markers
            .iter()
            .map(|m| resolve_lark_media_marker(m, self.workspace_dir.as_deref()))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut prepared_media = Vec::with_capacity(resolved_markers.len());
        for marker in &resolved_markers {
            prepared_media.push(self.prepare_lark_media_marker(&mut token, marker).await?);
        }

        if !text_content.is_empty() || markers.is_empty() {
            let chunks = split_markdown_chunks(&text_content, LARK_CARD_MARKDOWN_MAX_BYTES);
            for chunk in &chunks {
                let body = build_interactive_card_body(&message.recipient, chunk);
                self.send_json_with_token_refresh(&url, &mut token, &body, "text send")
                    .await?;
            }
        }

        for media in &prepared_media {
            self.send_lark_media_message(&mut token, &message.recipient, media)
                .await?;
        }

        Ok(())
    }

    async fn listen(&self, tx: Sender<ChannelMessage>) -> anyhow::Result<()> {
        match self.receive_mode {
            LarkReceiveMode::Websocket => self.listen_ws(tx).await,
            LarkReceiveMode::Webhook => anyhow::bail!("unsupported webhook, wait update."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channel(platform: LarkPlatform) -> LarkChannel {
        LarkChannel::new_with_platform(
            "app_id".into(),
            "secret".into(),
            "token".into(),
            None,
            "test",
            Arc::new(|| vec![]),
            false,
            platform,
        )
    }

    #[test]
    fn lark_url_builders() {
        let ch = make_channel(LarkPlatform::Lark);
        assert_eq!(
            ch.tenant_access_token_url(),
            "https://open.larksuite.com/open-apis/auth/v3/tenant_access_token/internal"
        );
        assert_eq!(
            ch.send_message_url(),
            "https://open.larksuite.com/open-apis/im/v1/messages?receive_id_type=chat_id"
        );
        assert_eq!(
            ch.bot_info_url(),
            "https://open.larksuite.com/open-apis/bot/v3/info"
        );
        assert_eq!(
            ch.patch_message_url("om_abc123"),
            "https://open.larksuite.com/open-apis/im/v1/messages/om_abc123"
        );
        assert_eq!(ch.channel_name(), "lark");
    }

    #[test]
    fn feishu_url_builders() {
        let ch = make_channel(LarkPlatform::Feishu);
        assert_eq!(
            ch.tenant_access_token_url(),
            "https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal"
        );
        assert_eq!(
            ch.send_message_url(),
            "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id"
        );
        assert_eq!(ch.channel_name(), "feishu");
    }

    #[test]
    fn from_config_lark() {
        let config = LarkConfig {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            verification_token: Some("vt".into()),
            port: Some(8080),
            use_feishu: false,
            mention_only: false,
            proxy_url: None,
            ..Default::default()
        };
        let ch = LarkChannel::from_config(&config, "alias", Arc::new(|| vec![]));
        assert_eq!(ch.alias, "alias");
        assert_eq!(ch.app_id, "test_app");
        assert_eq!(ch.channel_name(), "lark");
    }

    #[test]
    fn from_config_feishu() {
        let config = LarkConfig {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            use_feishu: true,
            ..Default::default()
        };
        let ch = LarkChannel::from_config(&config, "f", Arc::new(|| vec![]));
        assert_eq!(ch.channel_name(), "feishu");
    }

    #[test]
    fn attributable_role_and_alias() {
        let ch = make_channel(LarkPlatform::Lark);
        assert_eq!(ch.alias(), "test");
        assert_eq!(ch.role(), Role::Channel(ChannelKind::Lark));
    }

    #[test]
    fn channel_trait_name() {
        let ch = make_channel(LarkPlatform::Feishu);
        assert_eq!(ch.name(), "feishu");
    }
}
