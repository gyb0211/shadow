//! 审批系统 — PendingApproval + card action + patch

use super::token::should_refresh_lark_tenant_token;
use shadow_core::channel::ChannelApprovalResponse;

use super::LarkChannel;

const LARK_DRAFT_RATE_LIMIT_CODE: i64 = 230_020;
const LARK_INVALID_ACCESS_TOKEN_CODE: i64 = 99_991_663;

pub(super) struct PendingApproval {
    pub(super) sender: tokio::sync::oneshot::Sender<ChannelApprovalResponse>,
    pub(super) message_id: String,
    pub(super) tool_name: String,
    pub(super) arguments_summary: String,
}

impl LarkChannel {
    pub(super) fn is_user_allowed(&self, open_id: &str) -> bool {
        let peers = (self.peer_resolver)();
        crate::allowlist::is_user_allowed(&peers, open_id, crate::allowlist::Match::Sensitive)
    }

    pub(super) async fn patch_or_send_once(
        &self,
        url: &str,
        body: &serde_json::Value,
        is_patch: bool,
    ) -> anyhow::Result<(reqwest::StatusCode, serde_json::Value)> {
        let token = self.get_tenant_access_token().await?;
        let builder = if is_patch {
            self.http_client().patch(url)
        } else {
            self.http_client().post(url)
        };
        let resp = builder
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

    pub(super) async fn patch_approval_card_resolved(
        &self,
        message_id: &str,
        tool_name: &str,
        arguments_summary: &str,
        decision: ChannelApprovalResponse,
    ) {
        let card = build_resolved_approval_card(tool_name, arguments_summary, decision.clone());
        let url = self.patch_message_url(message_id);
        let body = serde_json::json!({
            "content": card.to_string(),
        });

        ::shadow_log::record!(
            INFO,
            ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Send)
                .with_outcome(::shadow_log::EventOutcome::Unknown)
                .with_attrs(::serde_json::json!({
                    "message_id": message_id,
                    "decision": format!("{decision:?}"),
                })),
            "Lark: approval card PATCH dispatching"
        );

        let (status, response) = match self.patch_or_send_once(&url, &body, true).await {
            Ok(pair) => pair,
            Err(e) => {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Send)
                        .with_outcome(::shadow_log::EventOutcome::Failure)
                        .with_attrs(::serde_json::json!({
                            "message_id": message_id,
                            "error": e.to_string(),
                        })),
                    "Lark: approval card PATCH transport error"
                );
                return;
            }
        };

        let final_body = if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            match self.patch_or_send_once(&url, &body, true).await {
                Ok((retry_status, retry_response)) => {
                    if should_refresh_lark_tenant_token(retry_status, &retry_response) {
                        ::shadow_log::record!(
                            WARN,
                            ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Send)
                                .with_outcome(::shadow_log::EventOutcome::Failure)
                                .with_attrs(::serde_json::json!({
                                    "message_id": message_id,
                                    "body": retry_response.to_string(),
                                })),
                            "Lark: approval card PATCH still unauthorized after token refresh"
                        );
                        return;
                    }
                    retry_response
                }
                Err(e) => {
                    ::shadow_log::record!(
                        WARN,
                        ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Send)
                            .with_outcome(::shadow_log::EventOutcome::Failure)
                            .with_attrs(::serde_json::json!({
                                "message_id": message_id,
                                "error": e.to_string(),
                            })),
                        "Lark: approval card PATCH retry transport error"
                    );
                    return;
                }
            }
        } else {
            response
        };

        let code = extract_lark_response_code(&final_body).unwrap_or(0);
        if code == LARK_DRAFT_RATE_LIMIT_CODE {
            ::shadow_log::record!(
                WARN,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Send)
                    .with_outcome(::shadow_log::EventOutcome::Failure)
                    .with_attrs(::serde_json::json!({
                        "message_id": message_id,
                        "code": LARK_DRAFT_RATE_LIMIT_CODE,
                    })),
                "Lark: approval card PATCH rate-limited"
            );
        } else if code != 0 {
            ::shadow_log::record!(
                WARN,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Send)
                    .with_outcome(::shadow_log::EventOutcome::Failure)
                    .with_attrs(::serde_json::json!({
                        "message_id": message_id,
                        "code": code,
                        "status": status.to_string(),
                        "body": final_body.to_string(),
                    })),
                "Lark: approval card PATCH soft-failed"
            );
        } else {
            ::shadow_log::record!(
                INFO,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Send)
                    .with_outcome(::shadow_log::EventOutcome::Success)
                    .with_attrs(::serde_json::json!({
                        "message_id": message_id,
                        "status": status.to_string(),
                    })),
                "Lark: approval card PATCH succeeded"
            );
        }
    }

    pub(super) async fn handle_card_action_event(
        &self,
        event_payload: &serde_json::Value,
    ) -> anyhow::Result<()> {
        // Diagnostic: emit a SANITIZED copy of the inbound payload at DEBUG
        ::shadow_log::record!(
            DEBUG,
            ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Receive).with_attrs(
                ::serde_json::json!({
                    "sanitized_payload": sanitize_card_action_payload(event_payload),
                })
            ),
            "card.action.trigger sanitized payload"
        );

        let value = event_payload
            .pointer("/action/value")
            .or_else(|| event_payload.pointer("/action/behaviors/0/value"))
            .ok_or_else(|| {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Reject)
                        .with_outcome(::shadow_log::EventOutcome::Failure),
                    "card.action.trigger: missing event.action.value or event.action.behaviors[0].value"
                );
                anyhow::Error::msg(
                    "card.action.trigger: missing event.action.value or event.action.behaviors[0].value",
                )
            })?;

        let approval_id = value
            .get("approval_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Reject)
                        .with_outcome(::shadow_log::EventOutcome::Failure),
                    "card.action.trigger: missing approval_id in value"
                );
                anyhow::Error::msg("card.action.trigger: missing approval_id in value")
            })?;

        let decision_str = value
            .get("decision")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Reject)
                        .with_outcome(::shadow_log::EventOutcome::Failure),
                    "card.action.trigger: missing decision in value"
                );
                anyhow::Error::msg("card.action.trigger: missing decision in value")
            })?;

        let decision = match decision_str {
            "approve" => ChannelApprovalResponse::Approve,
            "deny" => ChannelApprovalResponse::Deny,
            "always" => ChannelApprovalResponse::AlwaysApprove,
            other => {
                ::shadow_log::record!(
                    WARN,
                    ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                        .with_outcome(::shadow_log::EventOutcome::Unknown)
                        .with_attrs(::serde_json::json!({"decision_str": other})),
                    "Lark: unknown approval decision — treating as deny"
                );
                ChannelApprovalResponse::Deny
            }
        };

        let pending = self.pending_approvals.lock().await.remove(approval_id);
        let Some(pending) = pending else {
            ::shadow_log::record!(
                INFO,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                    .with_outcome(::shadow_log::EventOutcome::Unknown)
                    .with_attrs(::serde_json::json!({
                        "approval_id": approval_id,
                        "decision": format!("{decision:?}"),
                    })),
                "Lark: card action for unknown/expired approval_id"
            );
            return Ok(());
        };

        ::shadow_log::record!(
            INFO,
            ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Receive)
                .with_outcome(::shadow_log::EventOutcome::Success)
                .with_attrs(::serde_json::json!({
                    "approval_id": approval_id,
                    "decision": format!("{decision:?}"),
                    "message_id": pending.message_id,
                    "has_message_id": !pending.message_id.is_empty(),
                })),
            "Lark: card action received"
        );

        let _ = pending.sender.send(decision.clone());

        if !pending.message_id.is_empty() {
            self.patch_approval_card_resolved(
                &pending.message_id,
                &pending.tool_name,
                &pending.arguments_summary,
                decision,
            )
            .await;
        }

        Ok(())
    }
}

// ─── 辅助函数 ─────────────────────────────────────────────

fn sanitize_card_action_payload(event_payload: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;

    let mut sanitized = event_payload.clone();

    if let Some(token) = sanitized.get_mut("token")
        && !token.is_null()
    {
        *token = Value::String("REDACTED_TOKEN".to_string());
    }

    if let Some(Value::Object(operator)) = sanitized.get_mut("operator") {
        for (key, placeholder) in [
            ("open_id", "REDACTED_OPERATOR_OPEN_ID"),
            ("union_id", "REDACTED_OPERATOR_UNION_ID"),
            ("user_id", "REDACTED_OPERATOR_USER_ID"),
            ("tenant_key", "REDACTED_OPERATOR_TENANT_KEY"),
        ] {
            if operator.contains_key(key) {
                operator.insert(key.to_string(), Value::String(placeholder.to_string()));
            }
        }
    }

    if let Some(Value::Object(context)) = sanitized.get_mut("context") {
        for (key, placeholder) in [
            ("open_chat_id", "REDACTED_OPEN_CHAT_ID"),
            ("open_message_id", "REDACTED_OPEN_MESSAGE_ID"),
        ] {
            if context.contains_key(key) {
                context.insert(key.to_string(), Value::String(placeholder.to_string()));
            }
        }
    }

    sanitized
}

fn build_resolved_approval_card(
    tool_name: &str,
    arguments_summary: &str,
    decision: ChannelApprovalResponse,
) -> serde_json::Value {
    let (banner_emoji, banner_text, header_template) = match decision {
        ChannelApprovalResponse::Approve => ("✅", "Approved", "green"),
        ChannelApprovalResponse::AlwaysApprove => ("✅✅", "Approved (always)", "green"),
        ChannelApprovalResponse::Deny => ("❌", "Denied", "red"),
        ChannelApprovalResponse::DenyWithEdit { .. } => {
            unreachable!("DenyWithEdit is only valid for ACP channels")
        }
    };

    serde_json::json!({
        "schema": "2.0",
        "config": { "wide_screen_mode": true },
        "header": {
            "template": header_template,
            "title": {
                "tag": "plain_text",
                "content": format!("{banner_emoji} Tool approval — {banner_text}")
            }
        },
        "body": {
            "elements": [
                {
                    "tag": "markdown",
                    "content": format!(
                        "**Tool:** `{tool_name}`\n\n{arguments_summary}\n\n---\n\n**{banner_emoji} {banner_text}**"
                    )
                }
            ]
        }
    })
}

pub(super) fn extract_lark_response_code(body: &serde_json::Value) -> Option<i64> {
    body.get("code").and_then(|c| c.as_i64())
}

pub(super) fn is_lark_invalid_access_token(body: &serde_json::Value) -> bool {
    extract_lark_response_code(body) == Some(LARK_INVALID_ACCESS_TOKEN_CODE)
}
