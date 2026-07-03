//! ChatError -- 结构化的 LLM 调用错误
//!
//! ReliableModelProvider 用此类型分类错误, 决定是否重试 / 切 key / 触发 fallback.
//! 仅在 shadow-providers 内部使用, 不污染 shadow-core.
//!
//! Compat 层 (OpenAiProvider 等) 把 HTTP 错误转成 ChatError 后, 用 anyhow::Error::new 包裹.
//! Reliable 层通过 `err.downcast_ref::<ChatError>()` 提取分类信息.

use std::fmt;

/// 错误类别 -- 决定 Reliable 层的重试策略
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryClass {
    /// 短暂错误 (5xx, 网络抖动) -- 退避后重试
    Transient,
    /// 限流 (429) -- 退避后重试, 尊重 Retry-After (如果有)
    RateLimit {
        /// 服务器建议的等待时间 (秒); None 表示未提供
        retry_after_secs: Option<u32>,
    },
    /// 认证失败 (401, 403) -- 切换 key 立即重试; 无可切 key 则视为 terminal
    Auth,
    /// 永久错误 (400 bad request, 404 not found) -- 不重试
    Permanent,
    /// 网络错误 (连接失败, DNS 解析失败) -- 退避后重试
    Network,
}

/// 结构化 LLM 错误
///
/// 由 Compat 层发出, Reliable 层消费. Router 层透明传递.
pub struct ChatError {
    /// HTTP 状态码 (网络错误时为 None)
    pub status: Option<u16>,
    /// 错误描述 (来自响应体或网络错误消息)
    pub message: String,
    /// 重试分类
    pub class: RetryClass,
}

impl ChatError {
    /// 从 HTTP status + body 构造 -- 自动分类
    #[must_use]
    pub fn from_status(status: reqwest::StatusCode, body: String) -> Self {
        let code = status.as_u16();
        let class = match code {
            429 => RetryClass::RateLimit { retry_after_secs: None },
            401 | 403 => RetryClass::Auth,
            400 | 404 | 405..=428 | 430..=499 => RetryClass::Permanent,
            500..=599 => RetryClass::Transient,
            _ => RetryClass::Permanent,
        };
        Self {
            status: Some(code),
            message: format!("HTTP {status}: {body}"),
            class,
        }
    }

    /// 网络错误 (无 HTTP 响应)
    #[must_use]
    pub fn network(message: impl Into<String>) -> Self {
        Self {
            status: None,
            message: message.into(),
            class: RetryClass::Network,
        }
    }

    /// 是否可重试 (按类别)
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self.class {
            RetryClass::Transient | RetryClass::RateLimit { .. } | RetryClass::Network => true,
            RetryClass::Auth | RetryClass::Permanent => false,
        }
    }

    /// 建议的退避秒数 (RateLimit + 有 Retry-After 时才返回)
    #[must_use]
    pub fn retry_after_secs(&self) -> Option<u32> {
        match self.class {
            RetryClass::RateLimit { retry_after_secs } => retry_after_secs,
            _ => None,
        }
    }

    /// 是否为认证错误 (用于触发 key 轮换)
    #[must_use]
    pub fn is_auth_error(&self) -> bool {
        matches!(self.class, RetryClass::Auth)
    }
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(code) => write!(f, "ChatError({:?}, status={}, msg={})", self.class, code, self.message),
            None => write!(f, "ChatError({:?}, msg={})", self.class, self.message),
        }
    }
}

impl fmt::Debug for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatError")
            .field("status", &self.status)
            .field("class", &self.class)
            .field("message", &self.message)
            .finish()
    }
}

impl std::error::Error for ChatError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_429_as_rate_limit() {
        let err = ChatError::from_status(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "rate limited".to_string(),
        );
        assert!(err.is_retryable());
        assert!(matches!(err.class, RetryClass::RateLimit { .. }));
    }

    #[test]
    fn classify_500_as_transient() {
        let err = ChatError::from_status(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "oops".to_string(),
        );
        assert!(err.is_retryable());
        assert_eq!(err.class, RetryClass::Transient);
    }

    #[test]
    fn classify_400_as_permanent() {
        let err = ChatError::from_status(
            reqwest::StatusCode::BAD_REQUEST,
            "bad".to_string(),
        );
        assert!(!err.is_retryable());
        assert_eq!(err.class, RetryClass::Permanent);
    }

    #[test]
    fn classify_401_as_auth() {
        let err = ChatError::from_status(
            reqwest::StatusCode::UNAUTHORIZED,
            "no key".to_string(),
        );
        assert!(!err.is_retryable());
        assert!(err.is_auth_error());
    }

    #[test]
    fn classify_403_as_auth() {
        let err = ChatError::from_status(
            reqwest::StatusCode::FORBIDDEN,
            "denied".to_string(),
        );
        assert!(err.is_auth_error());
    }

    #[test]
    fn network_error_is_retryable() {
        let err = ChatError::network("connection refused");
        assert!(err.is_retryable());
        assert_eq!(err.class, RetryClass::Network);
        assert!(err.status.is_none());
    }

    #[test]
    fn retry_after_secs_only_for_rate_limit() {
        let rl = ChatError {
            status: Some(429),
            message: "rl".to_string(),
            class: RetryClass::RateLimit { retry_after_secs: Some(30) },
        };
        assert_eq!(rl.retry_after_secs(), Some(30));

        let transient = ChatError::from_status(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "x".to_string(),
        );
        assert_eq!(transient.retry_after_secs(), None);
    }
}
