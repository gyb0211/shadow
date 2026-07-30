use crate::transcription::TranscriptionManager;
use crate::utils;
use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use serde_json::Value;
use shadow_config::LarkConfig;
use shadow_config::channel::{LarkReceiveMode, StreamMode, TranscriptionConfig};
use shadow_core::channel::ChannelApprovalResponse;
use shadow_core::{Attributable, Channel, ChannelKind, ChannelMessage, Role, SendMessage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::sync::mpsc::Sender;
use tokio::sync::{Mutex, RwLock};

struct PendingApproval {
    sender: tokio::sync::oneshot::Sender<ChannelApprovalResponse>,
    message_id: String,
    tool_name: String,
    arguments_summary: String,
}

#[derive(Debug, Clone)]
struct CachedTenantToken {
    value: String,
    refresh_after: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LarkPlatform {
    Lark,
    Feishu,
}
const FEISHU_BASE_URL: &str = "https://open.feishu.cn/open-apis";
const FEISHU_WS_BASE_URL: &str = "https://open.feishu.cn";
const LARK_BASE_URL: &str = "https://open.larksuite.com/open-apis";
const LARK_WS_BASE_URL: &str = "https://open.larksuite.com";
impl LarkPlatform {
    fn channel_name(self) -> &'static str {
        match self {
            LarkPlatform::Lark => "lark",
            LarkPlatform::Feishu => "feishu",
        }
    }
    fn api_base(self) -> &'static str {
        match self {
            Self::Lark => LARK_BASE_URL,
            Self::Feishu => FEISHU_BASE_URL,
        }
    }

    fn ws_base(self) -> &'static str {
        match self {
            Self::Lark => LARK_WS_BASE_URL,
            Self::Feishu => FEISHU_WS_BASE_URL,
        }
    }
}

pub struct LarkChannel {
    app_id: String,
    app_secret: String,
    verification_token: String,
    port: Option<u16>,
    alias: String,
    peer_resolver: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    resolved_bot_open_id: Arc<StdRwLock<Option<String>>>,
    mention_only: bool,
    platform: LarkPlatform,
    receive_mode: LarkReceiveMode,
    tenant_token: Arc<RwLock<Option<CachedTenantToken>>>,
    ws_seen_ids: Arc<RwLock<HashMap<String, Instant>>>,
    proxy_url: Option<String>,
    workspace_dir: Option<PathBuf>,
    transcription: Option<TranscriptionConfig>,
    transcription_manager: Option<TranscriptionManager>,
    pending_approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
    approval_timeout_secs: u64,
    per_user_session: bool,
    ack_reactions: bool,
    reaction_ids: Arc<Mutex<HashMap<(String, String), String>>>,
    stream_mode: StreamMode,
    draft_update_interval_ms: u64,
    last_draft_edit: Arc<Mutex<HashMap<String, Instant>>>,
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


    fn http_client(&self) -> reqwest::Client {
        shadow_config::build_channel_proxy_client(
            self.platform.proxy_service_key(),
            self.proxy_url.as_deref(),
        )
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
    fn api_base(&self) -> &str {
        // #[cfg(test)]
        // if let Some(ref url) = self.api_base_override {
        //     return url.as_str();
        // }
        self.platform.api_base()
    }
    fn channel_name(&self) -> &'static str {
        self.platform.channel_name()
    }

    fn tenant_access_token_url(&self) -> String {
        format!("{}/auth/v3/tenant_access_token/internal", self.api_base())
    }

    fn bot_info_url(&self) -> String {
        format!("{}/bot/v3/info", self.api_base())
    }

    fn send_message_url(&self) -> String {
        format!("{}/im/v1/messages?receive_id_type=chat_id", self.api_base())
    }

    /// PATCH endpoint for updating the content of a previously-sent message
    /// (used to flip an approval card from its interactive state to its
    /// resolved/banner state after the user clicks a button).
    fn patch_message_url(&self, message_id: &str) -> String {
        format!("{}/im/v1/messages/{message_id}", self.api_base())
    }

    async fn prepare_lark_media_marker(
        &self,
        token: &mut String,
        marker: &LarkResolvedMediaMarker,
    ) -> anyhow::Result<LarkPreparedMediaMessage> {
        let (msg_type, content) = match marker.kind {
            LarkOutgoingMediaKind::Image => {
                let image_key = self.upload_lark_image(token, marker).await?;
                ("image", serde_json::json!({"image_key": image_key}))
            }
            LarkOutgoingMediaKind::File { file_type } => {
                let file_key = self.upload_lark_file(token, marker, file_type).await?;
                ("file", serde_json::json!({"file_key": file_key}))
            }
        };

        Ok(LarkPreparedMediaMessage { msg_type, content })
    }
    async fn post_multipart_once(
        &self,
        url: &str,
        token: &str,
        form: Form,
    ) -> anyhow::Result<(reqwest::StatusCode, serde_json::Value)> {
        let resp = self
            .http_client()
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .multipart(form)
            .send()
            .await?;
        let status = resp.status();
        let raw = resp.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap_or_else(|_| serde_json::json!({ "raw": raw }));
        Ok((status, parsed))
    }
    async fn invalidate_token(&self) {
        let mut cached = self.tenant_token.write().await;
        *cached = None;
    }
    async fn get_tenant_access_token(&self) -> anyhow::Result<String> {
        // Check cache first
        {
            let cached = self.tenant_token.read().await;
            if let Some(ref token) = *cached
                && Instant::now() < token.refresh_after
            {
                return Ok(token.value.clone());
            }
        }

        let url = self.tenant_access_token_url();
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });

        let resp = self.http_client().post(&url).json(&body).send().await?;
        let status = resp.status();
        let data: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            anyhow::bail!("tenant_access_token request failed: status={status}, body={data}");
        }

        let code = data.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = data
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("tenant_access_token failed: {msg}");
        }

        let token = data
            .get("tenant_access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Reject)
                        .with_outcome(::shadow_log::EventOutcome::Failure),
                    "missing tenant_access_token in response"
                );
                anyhow::Error::msg("missing tenant_access_token in response")
            })?
            .to_string();

        let ttl_seconds = extract_lark_token_ttl_seconds(&data);
        let refresh_after = next_token_refresh_deadline(Instant::now(), ttl_seconds);

        // Cache it with proactive refresh metadata.
        {
            let mut cached = self.tenant_token.write().await;
            *cached = Some(CachedTenantToken {
                value: token.clone(),
                refresh_after,
            });
        }

        Ok(token)
    }

    async fn upload_lark_image(
        &self,
        token: &mut String,
        marker: &LarkResolvedMediaMarker,
    ) -> anyhow::Result<String> {
        let url = format!("{}/im/v1/images", self.api_base());
        let form = build_lark_image_upload_form(marker).await?;
        let (status, response) = self.post_multipart_once(&url, token, form).await?;
        let response = if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            *token = self.get_tenant_access_token().await?;
            let retry_form = build_lark_image_upload_form(marker).await?;
            let (retry_status, retry_response) =
                self.post_multipart_once(&url, token, retry_form).await?;
            if should_refresh_lark_tenant_token(retry_status, &retry_response) {
                anyhow::bail!(
                    "upload image failed after token refresh: status={retry_status}, body={retry_response}"
                );
            }
            ensure_lark_send_success(retry_status, &retry_response, "upload image")?;
            retry_response
        } else {
            ensure_lark_send_success(status, &response, "upload image")?;
            response
        };

        response
            .pointer("/data/image_key")
            .or_else(|| response.get("image_key"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::Error::msg("Lark/Feishu image upload returned no image_key"))
    }

    async fn upload_lark_file(
        &self,
        token: &mut String,
        marker: &LarkResolvedMediaMarker,
        file_type: &'static str,
    ) -> anyhow::Result<String> {
        let url = format!("{}/im/v1/files", self.api_base());
        let form = build_lark_file_upload_form(marker, file_type).await?;
        let (status, response) = self.post_multipart_once(&url, token, form).await?;
        let response = if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            *token = self.get_tenant_access_token().await?;
            let retry_form = build_lark_file_upload_form(marker, file_type).await?;
            let (retry_status, retry_response) =
                self.post_multipart_once(&url, token, retry_form).await?;
            if should_refresh_lark_tenant_token(retry_status, &retry_response) {
                anyhow::bail!(
                    "upload file failed after token refresh: status={retry_status}, body={retry_response}"
                );
            }
            ensure_lark_send_success(retry_status, &retry_response, "upload file")?;
            retry_response
        } else {
            ensure_lark_send_success(status, &response, "upload file")?;
            response
        };

        response
            .pointer("/data/file_key")
            .or_else(|| response.get("file_key"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::Error::msg("Lark/Feishu file upload returned no file_key"))
    }
}

struct LarkPreparedMediaMessage {
    msg_type: &'static str,
    content: Value,
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
            self.send_larg_media_message(&token, &message.recipient, media)
                .await?;
        }

        Ok(())
    }

    async fn listen(&self, tx: Sender<ChannelMessage>) -> anyhow::Result<()> {
        todo!()
    }
}

fn lark_outgoing_media_from_marker(
    kind: String,
    target: String,
) -> Option<LarkOutgoingMediaMarker> {
    Some(LarkOutgoingMediaMarker {
        kind: LarkOutgoingMediaKind::from_marker_kind(&kind)?,
        target,
    })
}

fn resolve_lark_media_marker(
    marker: &LarkOutgoingMediaMarker,
    workspace_dir: Option<&Path>,
) -> anyhow::Result<LarkResolvedMediaMarker> {
    let path = validate_lark_marker_target(&marker.target, workspace_dir)?;
    let metadata = std::fs::metadata(&path).map_err(|err| {
        anyhow::Error::msg(format!(
            "read Lark/Feishu marker target metadata failed: {err}"
        ))
    })?;
    if !metadata.is_file() {
        anyhow::bail!("read Lark/Feishu marker target is not a file");
    }

    if metadata.len() == 0 {
        anyhow::bail!("read Lark/Feishu marker target is empty");
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_string();

    Ok(LarkResolvedMediaMarker {
        kind: marker.kind,
        path,
        file_name,
    })
}

fn validate_lark_marker_target(
    target: &str,
    workspace_dir: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Lark/Feishu marker target is empty");
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("data:")
        || lower.starts_with("file:")
        || lower.contains("://")
    {
        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_attrs(serde_json::json!({"reason": "disallowed_scheme"})),
            "lark: marker target uses disallowed scheme"
        );
        anyhow::bail!("Lark/Feishu marker target uses a disallowed scheme");
    }

    let workspace = workspace_dir.ok_or_else(|| {
        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_attrs(serde_json::json!({"reason": "no_workspace_dir"})),
            "lark: marker target has no workspace_dir"
        );
        anyhow::bail!("Lark/Feishu channel was started without a workspace_dir");
    })?;

    let workspace = std::fs::canonicalize(workspace).map_err(|err| {
        anyhow::Error::msg(format!(
            "canonicalize Lark/Feishu workspace_dir failed: {err}"
        ))
    })?;
    let candidate = Path::new(trimmed);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace.join(candidate)
    };

    let candidate = std::fs::canonicalize(&candidate).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            shadow_log::record!(
                WARN,
                shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                    .with_attrs(serde_json::json!({"reason": "not found"})),
                "lark: marker target not found on disk"
            );
            anyhow::Error::msg("Lark/Feishu marker target not found on disk")
        } else {
            anyhow::Error::msg(format!(
                "canonicalize Lark/Feishu marker target failed: {err}"
            ))
        }
    })?;

    if !candidate.starts_with(&workspace) {
        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_attrs(serde_json::json!({"reason": "outside_workspace"})),
            "lark: marker target escapes workspaces"
        );
        anyhow::bail!("Lark/Feishu marker target resolves outside workspace_dir");
    }

    Ok(candidate)
}

#[derive(Debug, Clone)]
struct LarkResolvedMediaMarker {
    kind: LarkOutgoingMediaKind,
    path: PathBuf,
    file_name: String,
}

#[derive(Debug, Clone)]
struct LarkOutgoingMediaMarker {
    kind: LarkOutgoingMediaKind,
    target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LarkOutgoingMediaKind {
    Image,
    File { file_type: &'static str },
}

impl LarkOutgoingMediaKind {
    fn from_marker_kind(kind: &str) -> Option<Self> {
        match kind.trim().to_ascii_uppercase().as_str() {
            "IMAGE" | "PHOTO" => Some(Self::Image),
            "DOCUMENT" | "FILE" => Some(Self::File {
                file_type: "stream",
            }),
            "VIDEO" => Some(Self::File { file_type: "mp4" }),
            "AUDIO" | "VOICE" => Some(Self::File { file_type: "opus" }),
            _ => None,
        }
    }
}

async fn build_lark_image_upload_form(marker: &LarkResolvedMediaMarker) -> anyhow::Result<Form> {
    let bytes = fs::read(&marker.path).await.map_err(|err| {
        anyhow::Error::msg(format!(
            "read Lark/Feishu image marker target failed: {err}"
        ))
    })?;
    Ok(Form::new().text("image_type", "message").part(
        "image",
        Part::bytes(bytes).file_name(marker.file_name.clone()),
    ))
}

fn should_refresh_lark_tenant_token(status: reqwest::StatusCode, body: &serde_json::Value) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED || is_lark_invalid_access_token(body)
}

fn extract_lark_response_code(body: &serde_json::Value) -> Option<i64> {
    body.get("code").and_then(|c| c.as_i64())
}

fn is_lark_invalid_access_token(body: &serde_json::Value) -> bool {
    extract_lark_response_code(body) == Some(LARK_INVALID_ACCESS_TOKEN_CODE)
}

const LARK_INVALID_ACCESS_TOKEN_CODE: i64 = 99_991_663;
fn ensure_lark_send_success(
    status: reqwest::StatusCode,
    body: &serde_json::Value,
    context: &str,
) -> anyhow::Result<()> {
    if !status.is_success() {
        anyhow::bail!("send failed {context}: status={status}, body={body}");
    }

    let code = extract_lark_response_code(body).unwrap_or(0);
    if code != 0 {
        anyhow::bail!("send failed {context}: code={code}, body={body}");
    }

    Ok(())
}


async fn build_lark_file_upload_form(
    marker: &LarkResolvedMediaMarker,
    file_type: &'static str,
) -> anyhow::Result<Form> {
    let bytes = fs::read(&marker.path).await.map_err(|err| {
        anyhow::Error::msg(format!("read Lark/Feishu file marker target failed: {err}"))
    })?;
    Ok(Form::new()
        .text("file_type", file_type)
        .text("file_name", marker.file_name.clone())
        .part(
            "file",
            Part::bytes(bytes).file_name(marker.file_name.clone()),
        ))
}

fn extract_lark_token_ttl_seconds(body: &serde_json::Value) -> u64 {
    let ttl = body
        .get("expire")
        .or_else(|| body.get("expires_in"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            body.get("expire")
                .or_else(|| body.get("expires_in"))
                .and_then(|v| v.as_i64())
                .and_then(|v| u64::try_from(v).ok())
        })
        .unwrap_or(LARK_DEFAULT_TOKEN_TTL.as_secs());
    ttl.max(1)
}

const LARK_DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(7200);
