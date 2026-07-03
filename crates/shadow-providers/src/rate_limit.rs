//! TokenBucket -- 简单的 RPM 限流器
//!
//! Reliable 层在调用 inner.chat() 前先 acquire_token().await,
//! 超过 requests_per_minute 时阻塞等待.
//!
//! 算法: 经典 token bucket. 一个 bucket 持有 N 个 token,
//! 每次 acquire 消费 1 个; 没有 token 时按 refill 速率等待.
//! RPM=0 表示无限流 (直接返回).

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::trace;

/// Token bucket -- 按 RPM (requests per minute) 限流
pub struct TokenBucket {
    /// 容量 = RPM
    capacity: u32,
    /// 当前 token 数 (允许 burst, 上限 capacity)
    tokens: Mutex<f64>,
    /// 上次 refill 时间
    last_refill: Mutex<Instant>,
    /// 每秒 refill 多少 token (= capacity / 60)
    refill_per_sec: f64,
}

impl TokenBucket {
    /// 构造 -- capacity=RPM, 初始 token=capacity (允许首分钟 burst)
    #[must_use]
    pub fn new(requests_per_minute: u32) -> Self {
        let refill_per_sec = f64::from(requests_per_minute) / 60.0;
        Self {
            capacity: requests_per_minute,
            tokens: Mutex::new(f64::from(requests_per_minute)),
            last_refill: Mutex::new(Instant::now()),
            refill_per_sec,
        }
    }

    /// 是否启用限流 (RPM=0 → false)
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.capacity > 0
    }

    /// 取一个 token; 不够则 sleep 等待 refill
    pub async fn acquire(&self) {
        if !self.is_enabled() {
            return;
        }
        loop {
            let wait = self.try_acquire_one();
            match wait {
                None => return, // 拿到了
                Some(dur) => {
                    trace!(?dur, "rate limit 等待");
                    sleep(dur).await;
                }
            }
        }
    }

    /// 尝试取一个 token. 返回 None=成功; Some(dur)=需等待 dur 后重试.
    fn try_acquire_one(&self) -> Option<Duration> {
        let mut tokens = self.tokens.lock().unwrap();
        let mut last = self.last_refill.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(*last);
        // refill: 按时间比例补充 token, 但不超过 capacity
        let refill = elapsed.as_secs_f64() * self.refill_per_sec;
        if refill > 0.0 {
            *tokens = (*tokens + refill).min(f64::from(self.capacity));
            *last = now;
        }
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            None
        } else {
            // 算出还需多久才有 1.0 token
            let need = 1.0 - *tokens;
            let secs = need / self.refill_per_sec.max(f64::MIN_POSITIVE);
            Some(Duration::from_secs_f64(secs))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn rpm_zero_disabled() {
        let bucket = TokenBucket::new(0);
        assert!(!bucket.is_enabled());
        // try_acquire_one 在 RPM=0 时... capacity=0, refill=0
        // 永远没有 token, 但 is_enabled=false 表示调用方应跳过
    }

    #[test]
    fn rpm_nonzero_enabled() {
        let bucket = TokenBucket::new(60);
        assert!(bucket.is_enabled());
    }

    #[tokio::test]
    async fn acquire_within_capacity_no_wait() {
        let bucket = TokenBucket::new(60); // 60 RPM = 1 token/sec
        // 初始 60 token, 前 60 次应立即成功
        for _ in 0..60 {
            // 用 Instant 测量 -- 应在毫秒级
            let before = Instant::now();
            bucket.acquire().await;
            assert!(before.elapsed() < Duration::from_millis(50));
        }
    }

    #[tokio::test]
    async fn acquire_beyond_capacity_waits() {
        let bucket = Arc::new(TokenBucket::new(60)); // 1 token/sec
        // 先消耗所有 token
        for _ in 0..60 {
            bucket.acquire().await;
        }
        // 第 61 次需要等约 1 秒
        let before = Instant::now();
        bucket.acquire().await;
        let elapsed = before.elapsed();
        // 应该等了至少 900ms (允许一点抖动)
        assert!(
            elapsed >= Duration::from_millis(900),
            "expected wait >= 900ms, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn acquire_refills_over_time() {
        let bucket = TokenBucket::new(120); // 2 token/sec
        // 消耗全部 120 个
        for _ in 0..120 {
            bucket.acquire().await;
        }
        // 等 600ms 后应该能拿 1+ 个 token
        sleep(Duration::from_millis(600)).await;
        let before = Instant::now();
        bucket.acquire().await;
        // 应该立即返回 (已经攒了 token)
        assert!(before.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn burst_then_steady_state() {
        // 高 RPM 验证 burst 后稳定速率
        let bucket = TokenBucket::new(60);
        for _ in 0..60 {
            bucket.acquire().await;
        }
        // 连续 acquire 3 个, 每个应等约 1 秒
        let start = Instant::now();
        for _ in 0..3 {
            bucket.acquire().await;
        }
        let elapsed = start.elapsed();
        // 3 个 token 约 2-3 秒 (第 1 个可能快, 后两个稳定)
        assert!(
            elapsed >= Duration::from_millis(1900),
            "expected >= 1.9s, got {:?}",
            elapsed
        );
    }
}
