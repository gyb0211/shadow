//! 渠道 trait -- 消息平台集成

use crate::kennel::attribution::{Attributable, Role};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 入站消息
#[derive(Debug, Clone, Default)]
pub struct ChannelMessage {
    pub id: String,
    pub sender: String,
    pub content: String,
    pub reply_target: String,
    pub channel: String,
    pub channel_alias: Option<String>,
    pub timestamp: u64,
    pub thread_ts: Option<String>,
    pub interruption_scope_id: Option<String>,
    pub attachments: Vec<MediaAttachment>,

    pub subject: Option<String>,
    pub internal_sop_event: Option<String>,
    pub passive_context: bool,
    pub explicitly_addressed: bool,
    pub conversation_scope: ChannelConversationScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelConversationScope {
    /// Isolate history by channel, room/reply target, thread, and sender.
    #[default]
    Sender,
    /// Share history for everyone in the room/reply target.
    ReplyTarget,
}

#[derive(Debug, Clone)]
pub struct MediaAttachment {
    /// Original file name (e.g. `voice.ogg`, `photo.jpg`).
    pub file_name: String,
    /// Raw bytes of the attachment.
    pub data: Vec<u8>,
    /// MIME type if known (e.g. `audio/ogg`, `image/jpeg`).
    pub mime_type: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Image,
    Video,
    Unknown,
}
impl MediaAttachment {
    /// Load an attachment from a file path on disk.
    ///
    /// # Caller path-validation contract
    ///
    /// This method reads the path supplied by the caller verbatim.  **Callers
    /// are responsible for validating or constraining `path` before calling
    /// this function when the path originates from untrusted input** (e.g. a
    /// user message, an HTTP request body, or any external data source).  No
    /// sandboxing or path canonicalization is performed here.
    ///
    /// Read errors are propagated as `Err` rather than silently producing an
    /// empty attachment, so the caller can decide how to handle missing or
    /// unreadable files.
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let p = std::path::Path::new(path);
        let data = std::fs::read(p)?;
        let file_name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .to_string();
        let mime_type = match p.extension().and_then(|e| e.to_str()) {
            Some("pdf") => Some("application/pdf".to_string()),
            Some("xlsx") => Some(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
            ),
            Some("docx") => Some(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    .to_string(),
            ),
            Some("csv") => Some("text/csv".to_string()),
            Some("png") => Some("image/png".to_string()),
            Some("jpg") | Some("jpeg") => Some("image/jpeg".to_string()),
            Some("txt") => Some("text/plain".to_string()),
            _ => Some("application/octet-stream".to_string()),
        };
        Ok(Self {
            file_name,
            data,
            mime_type,
        })
    }

    /// Classify this attachment into a [`MediaKind`].
    pub fn kind(&self) -> MediaKind {
        // Try MIME type first.
        if let Some(ref mime) = self.mime_type {
            let lower = mime.to_ascii_lowercase();
            if lower.starts_with("audio/") {
                return MediaKind::Audio;
            }
            if lower.starts_with("image/") {
                return MediaKind::Image;
            }
            if lower.starts_with("video/") {
                return MediaKind::Video;
            }
        }

        // Fall back to file extension.
        let ext = self
            .file_name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            "flac" | "mp3" | "mpeg" | "mpga" | "m4a" | "ogg" | "oga" | "opus" | "wav" | "webm" => {
                MediaKind::Audio
            }
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "heic" | "tiff" | "svg" => {
                MediaKind::Image
            }
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" => MediaKind::Video,
            _ => MediaKind::Unknown,
        }
    }
}

/// 出站消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessage {
    pub content: String,
    pub recipient: String,
    pub thread_ts: Option<String>,
}

impl SendMessage {
    pub fn new(content: impl Into<String>, recipient: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            recipient: recipient.into(),
            thread_ts: None,
        }
    }

    pub fn in_thread(mut self, thread_ts: Option<String>) -> Self {
        self.thread_ts = thread_ts;
        self
    }
}

/// 渠道 trait
///
/// 每个消息平台实现此 trait (CLI/Telegram/Discord...)
#[async_trait]
pub trait Channel: Attributable {
    /// 渠道名称
    fn name(&self) -> &str;

    /// 发送消息
    async fn send(&self, message: &SendMessage) -> Result<()>;

    /// 是否支持审批请求
    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelApprovalResponse {
    Approve,
    Deny,
    #[serde(rename = "always")]
    AlwaysApprove,
    #[serde(rename = "deny_with_edit")]
    DenyWithEdit {
        replacement: String,
    },
}
