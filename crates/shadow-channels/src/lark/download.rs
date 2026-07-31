//! 资源下载（音频/图片/文件）+ 转录

use base64::Engine;

use super::event::{
    LARK_FILE_MAX_BYTES, LARK_IMAGE_MAX_BYTES, LARK_SUPPORTED_IMAGE_MIMES, MAX_LARK_AUDIO_BYTES,
    inferred_audio_filename, lark_detect_image_mime, lark_inline_text_file_preview,
    lark_is_text_filename,
};
use super::token::should_refresh_lark_tenant_token;
use crate::transcription::TranscriptionManager;

use super::LarkChannel;

impl LarkChannel {
    // ─── Audio ────────────────────────────────────────────────

    pub(super) async fn download_audio_resource(
        &self,
        message_id: &str,
        file_key: &str,
    ) -> anyhow::Result<(Vec<u8>, String)> {
        let url = format!(
            "{}/im/v1/messages/{message_id}/resources/{file_key}?type=file",
            self.api_base()
        );
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let body: serde_json::Value =
                serde_json::from_str(&body_text).unwrap_or_else(|_| serde_json::json!({}));

            if should_refresh_lark_tenant_token(status, &body) {
                self.invalidate_token().await;
                let token = self.get_tenant_access_token().await?;
                let resp = self
                    .http_client()
                    .get(&url)
                    .header("Authorization", format!("Bearer {token}"))
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    anyhow::bail!(
                        "audio download failed after token refresh: {}",
                        resp.status()
                    );
                }
                let bytes = Self::stream_audio_bytes(resp).await?;
                return Ok((bytes, inferred_audio_filename(file_key)));
            }

            anyhow::bail!("audio download failed: {}", status);
        }
        let bytes = Self::stream_audio_bytes(resp).await?;
        Ok((bytes, inferred_audio_filename(file_key)))
    }

    async fn stream_audio_bytes(mut resp: reqwest::Response) -> anyhow::Result<Vec<u8>> {
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            body.extend_from_slice(&chunk);
            if body.len() as u64 > MAX_LARK_AUDIO_BYTES {
                anyhow::bail!("audio download exceeds {} byte limit", MAX_LARK_AUDIO_BYTES);
            }
        }
        Ok(body)
    }

    pub(super) async fn try_transcribe_audio_message(
        &self,
        message_id: &str,
        content: &str,
        manager: &TranscriptionManager,
    ) -> Option<String> {
        let file_key = serde_json::from_str::<serde_json::Value>(content)
            .ok()
            .and_then(|v| {
                v.get("file_key")
                    .and_then(|k| k.as_str())
                    .map(str::to_owned)
            })?;

        let (audio_data, filename) = match self.download_audio_resource(message_id, &file_key).await
        {
            Ok(result) => result,
            Err(e) => {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(
                            ::serde_json::json!({"error": format!("{}", e), "message_id": message_id})
                        ),
                    "audio download failed for"
                );
                return None;
            }
        };

        match manager.transcribe(&audio_data, &filename).await {
            Ok(transcript) => {
                ::shadow_log::record!(
                    DEBUG,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_attrs(::serde_json::json!({"message_id": message_id})),
                    "audio transcribed for"
                );
                Some(transcript)
            }
            Err(e) => {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(
                            ::serde_json::json!({"error": format!("{}", e), "message_id": message_id})
                        ),
                    "transcription failed for"
                );
                None
            }
        }
    }

    // ─── Image download ───────────────────────────────────────

    fn image_resource_url(&self, message_id: &str, image_key: &str) -> String {
        format!(
            "{}/im/v1/messages/{message_id}/resources/{image_key}?type=image",
            self.api_base()
        )
    }

    pub(super) async fn download_image_as_marker(
        &self,
        message_id: &str,
        image_key: &str,
    ) -> Option<String> {
        let url = self.image_resource_url(message_id, image_key);
        let mut retried_token = false;

        loop {
            let token = match self.get_tenant_access_token().await {
                Ok(t) => t,
                Err(e) => {
                    ::shadow_log::record!(
                        WARN,
                        ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                            .with_outcome(::shadow_log::EventOutcome::Unknown)
                            .with_attrs(::serde_json::json!({"error": format!("{}", e)})),
                        "failed to get token for image download"
                    );
                    return None;
                }
            };

            let resp = match self
                .http_client()
                .get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    ::shadow_log::record!(
                        WARN,
                        ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                            .with_outcome(::shadow_log::EventOutcome::Unknown)
                            .with_attrs(
                                ::serde_json::json!({"error": format!("{}", e), "image_key": image_key})
                            ),
                        "image download request failed for"
                    );
                    return None;
                }
            };

            if resp.status() == reqwest::StatusCode::UNAUTHORIZED && !retried_token {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(::serde_json::json!({"image_key": image_key})),
                    "image download 401, refreshing token and retrying once"
                );
                drop(resp);
                self.invalidate_token().await;
                retried_token = true;
                continue;
            }

            if !resp.status().is_success() {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown),
                    &format!(
                        "image download failed for {image_key}: status={}",
                        resp.status()
                    )
                );
                return None;
            }

            if let Some(cl) = resp.content_length()
                && cl > LARK_IMAGE_MAX_BYTES as u64
            {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(::serde_json::json!({"image_key": image_key, "cl": cl})),
                    "image too large for : bytes exceeds limit"
                );
                return None;
            }

            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);

            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    ::shadow_log::record!(
                        WARN,
                        ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                            .with_outcome(::shadow_log::EventOutcome::Unknown)
                            .with_attrs(
                                ::serde_json::json!({"error": format!("{}", e), "image_key": image_key})
                            ),
                        "image body read failed for"
                    );
                    return None;
                }
            };

            if bytes.is_empty() || bytes.len() > LARK_IMAGE_MAX_BYTES {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown),
                    &format!(
                        "image body empty or too large for {image_key}: {} bytes",
                        bytes.len()
                    )
                );
                return None;
            }

            let mime = lark_detect_image_mime(content_type.as_deref(), &bytes)?;
            if !LARK_SUPPORTED_IMAGE_MIMES.contains(&mime.as_str()) {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(::serde_json::json!({"image_key": image_key, "mime": mime})),
                    "unsupported image MIME for"
                );
                return None;
            }

            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Some(format!("[IMAGE:data:{mime};base64,{encoded}]"));
        }
    }

    // ─── File download ────────────────────────────────────────

    fn file_download_url(&self, message_id: &str, file_key: &str) -> String {
        format!(
            "{}/im/v1/messages/{message_id}/resources/{file_key}?type=file",
            self.api_base()
        )
    }

    /// Download a file from the Lark API and return a text content marker.
    /// For text-like files, the content is inlined. For binary files, a summary is returned.
    pub(super) async fn download_file_as_content(
        &self,
        message_id: &str,
        file_key: &str,
        file_name: &str,
    ) -> Option<String> {
        let token = match self.get_tenant_access_token().await {
            Ok(t) => t,
            Err(e) => {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(::serde_json::json!({"error": format!("{}", e)})),
                    "failed to get token for file download"
                );
                return None;
            }
        };

        let url = self.file_download_url(message_id, file_key);
        let resp = match self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(
                            ::serde_json::json!({"error": format!("{}", e), "file_key": file_key})
                        ),
                    "file download request failed for"
                );
                return None;
            }
        };

        if !resp.status().is_success() {
            ::shadow_log::record!(
                WARN,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                    .with_outcome(::shadow_log::EventOutcome::Unknown),
                &format!(
                    "file download failed for {file_key}: status={}",
                    resp.status()
                )
            );
            return None;
        }

        if let Some(cl) = resp.content_length()
            && cl > LARK_FILE_MAX_BYTES as u64
        {
            ::shadow_log::record!(
                WARN,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                    .with_outcome(::shadow_log::EventOutcome::Unknown)
                    .with_attrs(::serde_json::json!({"file_key": file_key, "cl": cl})),
                "file too large for : bytes exceeds limit"
            );
            return Some(format!(
                "[ATTACHMENT:{file_name} | size={cl} bytes | too large to inline]"
            ));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(
                            ::serde_json::json!({"error": format!("{}", e), "file_key": file_key})
                        ),
                    "file body read failed for"
                );
                return None;
            }
        };

        if bytes.is_empty() {
            ::shadow_log::record!(
                WARN,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                    .with_outcome(::shadow_log::EventOutcome::Unknown)
                    .with_attrs(::serde_json::json!({"file_key": file_key})),
                "file body is empty for"
            );
            return None;
        }

        // If the content is image-like, return as image marker
        if content_type.starts_with("image/")
            && bytes.len() <= LARK_IMAGE_MAX_BYTES
            && let Some(mime) = lark_detect_image_mime(Some(&content_type), &bytes)
            && LARK_SUPPORTED_IMAGE_MIMES.contains(&mime.as_str())
        {
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Some(format!("[IMAGE:data:{mime};base64,{encoded}]"));
        }

        // If the file looks like text, inline it
        if bytes.len() <= LARK_FILE_MAX_BYTES
            && !bytes.contains(&0)
            && (content_type.starts_with("text/")
                || content_type.contains("json")
                || content_type.contains("xml")
                || content_type.contains("yaml")
                || content_type.contains("javascript")
                || content_type.contains("csv")
                || lark_is_text_filename(file_name))
        {
            let text = String::from_utf8_lossy(&bytes);
            let truncated = lark_inline_text_file_preview(text);
            let ext = file_name.rsplit('.').next().unwrap_or("text");
            return Some(format!("[FILE:{file_name}]\n```{ext}\n{truncated}\n```"));
        }

        Some(format!(
            "[ATTACHMENT:{file_name} | mime={content_type} | size={} bytes]",
            bytes.len()
        ))
    }
}
