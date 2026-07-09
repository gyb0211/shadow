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

