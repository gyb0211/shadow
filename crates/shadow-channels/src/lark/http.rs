//! HTTP 客户端 + URL 构建 + Token 管理 + 消息发送

use super::media::{
    LarkOutgoingMediaKind, LarkPreparedMediaMessage, LarkResolvedMediaMarker,
    build_lark_file_upload_form, build_lark_image_upload_form,
};
use super::platform::LarkPlatform;
use super::token::{
    CachedTenantToken, ensure_lark_send_success, extract_lark_token_ttl_seconds,
    next_token_refresh_deadline, should_refresh_lark_tenant_token,
};
use reqwest::StatusCode;
use reqwest::multipart::Form;
use serde_json::Value;
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Instant;
use tokio::sync::RwLock;

use super::LarkChannel;

impl LarkChannel {
    pub(super) fn http_client(&self) -> reqwest::Client {
        shadow_config::build_channel_proxy_client(
            self.platform.proxy_service_key(),
            self.proxy_url.as_deref(),
        )
    }

    pub(super) fn api_base(&self) -> &str {
        self.platform.api_base()
    }

    pub(super) fn ws_base(&self) -> &'static str {
        self.platform.ws_base()
    }

    pub(super) fn channel_name(&self) -> &'static str {
        self.platform.channel_name()
    }

    pub(super) fn tenant_access_token_url(&self) -> String {
        format!("{}/auth/v3/tenant_access_token/internal", self.api_base())
    }

    pub(super) fn bot_info_url(&self) -> String {
        format!("{}/bot/v3/info", self.api_base())
    }

    pub(super) fn send_message_url(&self) -> String {
        format!("{}/im/v1/messages?receive_id_type=chat_id", self.api_base())
    }

    /// PATCH endpoint for updating the content of a previously-sent message
    /// (used to flip an approval card from its interactive state to its
    /// resolved/banner state after the user clicks a button).
    pub(super) fn patch_message_url(&self, message_id: &str) -> String {
        format!("{}/im/v1/messages/{message_id}", self.api_base())
    }

    // ─── Token ────────────────────────────────────────────────

    pub(super) async fn invalidate_token(&self) {
        let mut cached = self.tenant_token.write().await;
        *cached = None;
    }

    pub(super) async fn get_tenant_access_token(&self) -> anyhow::Result<String> {
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

    // ─── Multipart upload ─────────────────────────────────────

    pub(super) async fn post_multipart_once(
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

    // ─── Media prepare + upload ───────────────────────────────

    pub(super) async fn prepare_lark_media_marker(
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

    pub(super) async fn upload_lark_image(
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

    pub(super) async fn upload_lark_file(
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

    // ─── JSON send with token refresh ─────────────────────────

    pub(super) async fn send_text_once(
        &self,
        url: &str,
        token: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<(reqwest::StatusCode, serde_json::Value)> {
        let resp = self
            .http_client()
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let raw = resp.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap_or_else(|_| serde_json::json!({ "raw": raw }));
        Ok((status, parsed))
    }

    pub(super) async fn send_json_with_token_refresh(
        &self,
        url: &str,
        token: &mut String,
        body: &serde_json::Value,
        context: &str,
    ) -> anyhow::Result<()> {
        let (status, response) = self.send_text_once(url, token, body).await?;

        if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            *token = self.get_tenant_access_token().await?;
            let (retry_status, retry_response) = self.send_text_once(url, token, body).await?;

            if should_refresh_lark_tenant_token(retry_status, &retry_response) {
                anyhow::bail!(
                    "send failed after token refresh: status={retry_status}, body={retry_response}"
                );
            }

            ensure_lark_send_success(retry_status, &retry_response, context)?;
        } else {
            ensure_lark_send_success(status, &response, context)?;
        }

        Ok(())
    }

    pub(super) async fn send_lark_media_message(
        &self,
        token: &mut String,
        recipient: &str,
        media: &LarkPreparedMediaMessage,
    ) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "receive_id": recipient,
            "msg_type": media.msg_type,
            "content": media.content.to_string(),
        });
        let url = self.send_message_url();
        self.send_json_with_token_refresh(&url, token, &body, "media send")
            .await
    }

    // ─── Bot Open ID ──────────────────────────────────────────

    pub(super) fn resolved_bot_open_id(&self) -> Option<String> {
        self.resolved_bot_open_id
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(super) fn set_resolved_bot_open_id(&self, open_id: Option<String>) {
        if let Ok(mut guard) = self.resolved_bot_open_id.write() {
            *guard = open_id;
        }
    }

    pub(super) async fn fetch_bot_open_id_with_token(
        &self,
        token: &str,
    ) -> anyhow::Result<(StatusCode, Value)> {
        let resp = self
            .http_client()
            .get(self.bot_info_url())
            .bearer_auth(token)
            .send()
            .await?;
        let status = resp.status();
        let body = resp
            .json::<serde_json::Value>()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        Ok((status, body))
    }

    pub(super) async fn refresh_bot_open_id(&self) -> anyhow::Result<Option<String>> {
        let token = self.get_tenant_access_token().await?;
        let (status, body) = self.fetch_bot_open_id_with_token(&token).await?;

        let body = if should_refresh_lark_tenant_token(status, &body) {
            self.invalidate_token().await;
            let refreshed = self.get_tenant_access_token().await?;
            let (retry_status, retry_body) = self.fetch_bot_open_id_with_token(&refreshed).await?;
            if !retry_status.is_success() {
                anyhow::bail!(
                    "bot info request failed after token refresh: status={status}, body={body}"
                )
            }
            retry_body
        } else {
            if !status.is_success() {
                anyhow::bail!("bot info request failed: status={status}, body={body}")
            }
            body
        };

        let code = body.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            anyhow::bail!("bot info failed: code={code}, body={body}");
        }

        let bot_open_id = body
            .pointer("/bot/open_id")
            .or_else(|| body.pointer("/data/bot/open_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);

        self.set_resolved_bot_open_id(bot_open_id.clone());
        Ok(bot_open_id)
    }

    pub(super) async fn ensure_bot_open_id(&self) {
        if !self.mention_only || self.resolved_bot_open_id().is_some() {
            return;
        }

        match self.refresh_bot_open_id().await {
            Ok(Some(open_id)) => {
                shadow_log::record!(
                    INFO,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_attrs(serde_json::json!({"open_id": open_id})),
                    "refresh and resolved bot open_id"
                );
            }
            Ok(None) => {
                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_outcome(shadow_log::EventOutcome::Unknown),
                    "refresh bot open_id missing from /bot/v3/info response; mention_only group messages will be ignored"
                );
            }

            Err(err) => {
                shadow_log::record!(
                    INFO,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_attrs(serde_json::json!({"err": err.to_string()})),
                    "failed to refresh and resolved bot open_id; mention_only group messages will be ignored"
                );
            }
        }
    }
}
