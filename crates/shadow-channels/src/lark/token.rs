use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(super) struct CachedTenantToken {
    pub(super) value: String,
    pub(super) refresh_after: Instant,
}

const LARK_INVALID_ACCESS_TOKEN_CODE: i64 = 99_991_663;
const LARK_DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(7200);
const LARK_TOKEN_REFRESH_SKEW: Duration = Duration::from_secs(120);

pub(super) fn should_refresh_lark_tenant_token(status: reqwest::StatusCode, body: &Value) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED || is_lark_invalid_access_token(body)
}

pub(super) fn ensure_lark_send_success(
    status: reqwest::StatusCode,
    body: &Value,
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

pub(super) fn extract_lark_token_ttl_seconds(body: &Value) -> u64 {
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

pub(super) fn next_token_refresh_deadline(now: Instant, ttl_seconds: u64) -> Instant {
    let ttl = Duration::from_secs(ttl_seconds.max(1));
    let refresh_in = ttl
        .checked_sub(LARK_TOKEN_REFRESH_SKEW)
        .unwrap_or(Duration::from_secs(1));
    now + refresh_in
}

fn extract_lark_response_code(body: &Value) -> Option<i64> {
    body.get("code").and_then(|c| c.as_i64())
}

fn is_lark_invalid_access_token(body: &Value) -> bool {
    extract_lark_response_code(body) == Some(LARK_INVALID_ACCESS_TOKEN_CODE)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- should_refresh_lark_tenant_token ----

    #[test]
    fn refresh_on_unauthorized_status() {
        let body = serde_json::json!({"code": 0});
        assert!(should_refresh_lark_tenant_token(
            reqwest::StatusCode::UNAUTHORIZED,
            &body
        ));
    }

    #[test]
    fn refresh_on_invalid_token_code() {
        let body = serde_json::json!({"code": LARK_INVALID_ACCESS_TOKEN_CODE});
        assert!(should_refresh_lark_tenant_token(
            reqwest::StatusCode::OK,
            &body
        ));
    }

    #[test]
    fn no_refresh_on_success() {
        let body = serde_json::json!({"code": 0});
        assert!(!should_refresh_lark_tenant_token(
            reqwest::StatusCode::OK,
            &body
        ));
    }

    // ---- ensure_lark_send_success ----

    #[test]
    fn ensure_success_ok() {
        let body = serde_json::json!({"code": 0});
        assert!(ensure_lark_send_success(reqwest::StatusCode::OK, &body, "test").is_ok());
    }

    #[test]
    fn ensure_fail_on_http_error() {
        let body = serde_json::json!({"code": 0});
        assert!(
            ensure_lark_send_success(reqwest::StatusCode::INTERNAL_SERVER_ERROR, &body, "test")
                .is_err()
        );
    }

    #[test]
    fn ensure_fail_on_nonzero_code() {
        let body = serde_json::json!({"code": 230001});
        assert!(ensure_lark_send_success(reqwest::StatusCode::OK, &body, "test").is_err());
    }

    // ---- extract_lark_token_ttl_seconds ----

    #[test]
    fn ttl_from_expire_u64() {
        let body = serde_json::json!({"expire": 7200});
        assert_eq!(extract_lark_token_ttl_seconds(&body), 7200);
    }

    #[test]
    fn ttl_from_expires_in() {
        let body = serde_json::json!({"expires_in": 3600});
        assert_eq!(extract_lark_token_ttl_seconds(&body), 3600);
    }

    #[test]
    fn ttl_missing_falls_back_to_default() {
        let body = serde_json::json!({});
        assert_eq!(extract_lark_token_ttl_seconds(&body), 7200);
    }

    #[test]
    fn ttl_clamped_to_min_one() {
        let body = serde_json::json!({"expire": 0u64});
        assert_eq!(extract_lark_token_ttl_seconds(&body), 1);
    }

    // ---- next_token_refresh_deadline ----

    #[test]
    fn refresh_deadline_subtracts_skew() {
        let now = Instant::now();
        let deadline = next_token_refresh_deadline(now, 7200);
        // 7200 - 120 = 7080 seconds ahead
        assert!(deadline > now);
        assert!(deadline <= now + std::time::Duration::from_secs(7081));
    }

    #[test]
    fn refresh_deadline_when_ttl_below_skew() {
        let now = Instant::now();
        let deadline = next_token_refresh_deadline(now, 60);
        // 60 < 120 → falls back to 1 second
        assert!(deadline > now);
        assert!(deadline <= now + std::time::Duration::from_secs(2));
    }
}
