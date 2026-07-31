//! 飞书事件/消息类型定义 + 内容解析工具函数

use serde::Deserialize;
use std::borrow::Cow;

use crate::utils;

// ─── 事件类型 ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct LarkEvent {
    pub(super) header: LarkEventHeader,
    pub(super) event: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct LarkEventHeader {
    pub(super) event_type: String,
    #[allow(dead_code)]
    pub(super) event_id: String,
}

// ─── 消息接收载荷 ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct MsgReceivePayload {
    pub(super) sender: LarkSender,
    pub(super) message: LarkMessage,
}

#[derive(Debug, Deserialize)]
pub(super) struct LarkSender {
    pub(super) sender_id: LarkSenderId,
    #[serde(default)]
    pub(super) sender_type: String,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct LarkSenderId {
    pub(super) open_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LarkMessage {
    pub(super) message_id: String,
    pub(super) chat_id: String,
    pub(super) chat_type: String,
    pub(super) message_type: String,
    #[serde(default)]
    pub(super) content: String,
    #[serde(default)]
    pub(super) mentions: Vec<serde_json::Value>,
}

// ─── Post 富文本解析 ──────────────────────────────────────

pub(super) struct ParsedPostContent {
    pub(super) text: String,
    pub(super) mentioned_open_ids: Vec<String>,
}

pub(super) fn parse_post_content_details(content: &str) -> Option<ParsedPostContent> {
    let parsed = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let locale = parsed
        .get("zh_cn")
        .or_else(|| parsed.get("en_us"))
        .or_else(|| {
            parsed
                .as_object()
                .and_then(|m| m.values().find(|v| v.is_object()))
        })?;

    let mut text = String::new();
    let mut mentioned_open_ids = Vec::new();

    if let Some(title) = locale
        .get("title")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
    {
        text.push_str(title);
        text.push_str("\n\n");
    }

    if let Some(paragraphs) = locale.get("content").and_then(|c| c.as_array()) {
        for para in paragraphs {
            if let Some(elements) = para.as_array() {
                for el in elements {
                    match el.get("tag").and_then(|t| t.as_str()).unwrap_or("") {
                        "text" => {
                            if let Some(t) = el.get("text").and_then(|t| t.as_str()) {
                                text.push_str(t);
                            }
                        }
                        "a" => {
                            text.push_str(
                                el.get("text")
                                    .and_then(|t| t.as_str())
                                    .filter(|s| !s.is_empty())
                                    .or_else(|| el.get("href").and_then(|h| h.as_str()))
                                    .unwrap_or(""),
                            );
                        }
                        "at" => {
                            let n = el
                                .get("user_name")
                                .and_then(|n| n.as_str())
                                .or_else(|| el.get("user_id").and_then(|i| i.as_str()))
                                .unwrap_or("user");
                            text.push('@');
                            text.push_str(n);
                            if let Some(open_id) = el
                                .get("user_id")
                                .and_then(|i| i.as_str())
                                .map(str::trim)
                                .filter(|id| !id.is_empty())
                            {
                                mentioned_open_ids.push(open_id.to_string());
                            }
                        }
                        _ => {
                            if let Some(t) = el.get("text").and_then(|t| t.as_str()) {
                                text.push_str(t);
                            }
                        }
                    }
                }
                text.push('\n');
            }
        }
    }

    let result = text.trim().to_string();
    if result.is_empty() {
        None
    } else {
        Some(ParsedPostContent {
            text: result,
            mentioned_open_ids,
        })
    }
}

// ─── List 内容解析 ────────────────────────────────────────

pub(super) fn parse_list_content(content: &str) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(content).ok()?;

    let items = parsed
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| parsed.get("content").and_then(|v| v.as_array()))?;

    let mut lines = Vec::new();
    collect_list_items(items, &mut lines, 0);

    let result = lines.join("\n").trim().to_string();
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn collect_list_items(items: &[serde_json::Value], lines: &mut Vec<String>, depth: usize) {
    let indent = "  ".repeat(depth);
    for item in items {
        let (inline_elements, children) = if let Some(arr) = item.as_array() {
            (arr.as_slice(), None)
        } else if let Some(obj) = item.as_object() {
            let inlines = obj
                .get("content")
                .and_then(|v| v.as_array())
                .map(|a| a.as_slice())
                .unwrap_or(&[]);
            let kids = obj.get("children").and_then(|v| v.as_array());
            (inlines, kids)
        } else {
            continue;
        };

        let mut text = String::new();
        for el in inline_elements {
            if let Some(inner_arr) = el.as_array() {
                for inner_el in inner_arr {
                    extract_inline_text(inner_el, &mut text);
                }
            } else {
                extract_inline_text(el, &mut text);
            }
        }

        let trimmed = text.trim();
        if !trimmed.is_empty() {
            lines.push(format!("{indent}- {trimmed}"));
        }

        if let Some(kids) = children {
            collect_list_items(kids, lines, depth + 1);
        }
    }
}

fn extract_inline_text(el: &serde_json::Value, out: &mut String) {
    match el.get("tag").and_then(|t| t.as_str()).unwrap_or("") {
        "text" => {
            if let Some(t) = el.get("text").and_then(|t| t.as_str()) {
                out.push_str(t);
            }
        }
        "a" => {
            out.push_str(
                el.get("text")
                    .and_then(|t| t.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| el.get("href").and_then(|h| h.as_str()))
                    .unwrap_or(""),
            );
        }
        "at" => {
            let n = el
                .get("user_name")
                .and_then(|n| n.as_str())
                .or_else(|| el.get("user_id").and_then(|i| i.as_str()))
                .unwrap_or("user");
            out.push('@');
            out.push_str(n);
        }
        _ => {}
    }
}

// ─── Mention / Group 判断 ─────────────────────────────────

fn mention_matches_bot_open_id(mention: &serde_json::Value, bot_open_id: &str) -> bool {
    mention
        .pointer("/id/open_id")
        .or_else(|| mention.pointer("/open_id"))
        .and_then(|v| v.as_str())
        .is_some_and(|value| value == bot_open_id)
}

/// In group chats, only respond when the bot is explicitly @-mentioned.
pub(super) fn should_respond_in_group(
    mention_only: bool,
    bot_open_id: Option<&str>,
    mentions: &[serde_json::Value],
    post_mentioned_open_ids: &[String],
) -> bool {
    if !mention_only {
        return true;
    }
    let Some(bot_open_id) = bot_open_id.filter(|id| !id.is_empty()) else {
        return false;
    };
    if mentions.is_empty() && post_mentioned_open_ids.is_empty() {
        return false;
    }
    mentions
        .iter()
        .any(|mention| mention_matches_bot_open_id(mention, bot_open_id))
        || post_mentioned_open_ids
            .iter()
            .any(|id| id.as_str() == bot_open_id)
}

// ─── 图片/文件辅助 ────────────────────────────────────────

/// Maximum image size we will download and inline (10 MiB).
pub(super) const LARK_IMAGE_MAX_BYTES: usize = 10 * 1024 * 1024;

/// Maximum file size we will download and present as text (512 KiB).
pub(super) const LARK_FILE_MAX_BYTES: usize = 512 * 1024;

/// Image MIME types we support for inline base64 encoding.
pub(super) const LARK_SUPPORTED_IMAGE_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

pub(super) fn lark_detect_image_mime(content_type: Option<&str>, bytes: &[u8]) -> Option<String> {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Some("image/png".to_string());
    }
    if bytes.len() >= 3 && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg".to_string());
    }
    if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some("image/gif".to_string());
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp".to_string());
    }
    if bytes.len() >= 2 && bytes.starts_with(b"BM") {
        return Some("image/bmp".to_string());
    }
    content_type
        .and_then(|ct| ct.split(';').next())
        .map(|ct| ct.trim().to_lowercase())
        .filter(|ct| ct.starts_with("image/"))
}

pub(super) fn lark_is_text_filename(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "rs"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "java"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "go"
            | "rb"
            | "sh"
            | "bash"
            | "zsh"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "xml"
            | "html"
            | "css"
            | "sql"
            | "csv"
            | "tsv"
            | "log"
            | "cfg"
            | "ini"
            | "conf"
            | "env"
            | "dockerfile"
            | "makefile"
    )
}

pub(super) fn lark_inline_text_file_preview(text: Cow<'_, str>) -> String {
    if text.len() > 50_000 {
        let end = utils::floor_char_boundary(text.as_ref(), 50_000);
        format!("{}...\n[truncated]", &text[..end])
    } else {
        text.into_owned()
    }
}

pub(super) fn inferred_audio_filename(file_key: &str) -> String {
    const SUPPORTED_EXTENSIONS: &[&str] = &[".m4a", ".ogg", ".mp3", ".aac", ".wav"];
    let file_key_lower = file_key.to_lowercase();
    if SUPPORTED_EXTENSIONS
        .iter()
        .any(|ext| file_key_lower.ends_with(ext))
    {
        file_key.to_string()
    } else {
        "voice.m4a".to_string()
    }
}

/// Max byte size for a single audio download (25 MiB).
pub(super) const MAX_LARK_AUDIO_BYTES: u64 = 25 * 1024 * 1024;
