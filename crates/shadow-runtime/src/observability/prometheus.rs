use prometheus::{
    Encoder, Gauge, GaugeVec, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec,
    IntGauge, Opts, Registry, TextEncoder,
};
use shadow_core::kennel::observer::{Observer, ObserverEvent, ObserverMetric};
use std::any::Any;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, OnceLock};

pub struct PrometheusObserver {
    registry: Registry,
    agent_starts: IntCounterVec,
    llm_requests: IntCounterVec,
    tokens_input_total: IntCounterVec,
    tokens_output_total: IntCounterVec,
    tool_calls: IntCounterVec,
    channel_messages: IntCounterVec,
    heartbeat_ticks: IntCounter,
    errors: IntCounterVec,
    cache_hits: IntCounterVec,
    cache_misses: IntCounterVec,
    cache_tokens_saved: IntCounterVec,

    agent_duration: HistogramVec,
    tool_duration: HistogramVec,
    request_latency: Histogram,

    tokens_used: IntGauge,
    active_sessions: GaugeVec,
    queue_depth: GaugeVec,

    deployments_total: IntCounterVec,
    deployment_lead_time: Histogram,
    deployment_failure_rate: Gauge,
    recovery_time: Histogram,
    mttr: Gauge,
    deploy_success_count: AtomicU64,
    deploy_failure_count: AtomicU64,
}

impl Default for PrometheusObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl PrometheusObserver {
    pub fn new() -> Self {
        let registry = Registry::new();

        // ---- Counters / CounterVec ----
        let agent_starts = IntCounterVec::new(
            Opts::new("agent_starts_total", "Total agent invocations"),
            &["model_provider", "model"],
        )
        .expect("valid metric");

        let llm_requests = IntCounterVec::new(
            Opts::new("llm_request_total", "Total LLM requests"),
            &["model_provider", "model", "success"],
        )
        .expect("valid metric");

        let tokens_input_total = IntCounterVec::new(
            Opts::new("tokens_input_total", "Total input tokens consumed"),
            &["model_provider", "model"],
        )
        .expect("valid metric");

        let tokens_output_total = IntCounterVec::new(
            Opts::new("tokens_output_total", "Total output tokens consumed"),
            &["model_provider", "model"],
        )
        .expect("valid metric");

        let tool_calls = IntCounterVec::new(
            Opts::new("tool_calls_total", "Total tool calls"),
            &["tool", "success"],
        )
        .expect("valid metric");

        let channel_messages = IntCounterVec::new(
            Opts::new("channel_messages_total", "Total channel messages"),
            &["channel", "direction"],
        )
        .expect("valid metric");

        let heartbeat_ticks =
            IntCounter::with_opts(Opts::new("heartbeat_ticks_total", "Total heartbeat ticks"))
                .expect("valid metric");

        let errors = IntCounterVec::new(
            Opts::new("errors_total", "Total errors by component"),
            &["component"],
        )
        .expect("valid metric");

        let cache_hits = IntCounterVec::new(
            Opts::new("cache_hits_total", "Total cache hits"),
            &["cache_type"],
        )
        .expect("valid metric");

        let cache_misses = IntCounterVec::new(
            Opts::new("cache_misses_total", "Total cache misses"),
            &["cache_type"],
        )
        .expect("valid metric");

        let cache_tokens_saved = IntCounterVec::new(
            Opts::new(
                "cache_tokens_saved_total",
                "Total tokens saved by cache hits",
            ),
            &["cache_type"],
        )
        .expect("valid metric");

        let deployments_total = IntCounterVec::new(
            Opts::new("deployments_total", "Total deployments by status"),
            &["status"],
        )
        .expect("valid metric");

        // ---- Gauges ----
        let tokens_used =
            IntGauge::with_opts(Opts::new("tokens_used", "Last reported token usage"))
                .expect("valid metric");

        let active_sessions =
            GaugeVec::new(Opts::new("active_sessions", "Current active sessions"), &[])
                .expect("valid metric");

        let queue_depth =
            GaugeVec::new(Opts::new("queue_depth", "Current message queue depth"), &[])
                .expect("valid metric");

        let deployment_failure_rate = Gauge::with_opts(Opts::new(
            "deployment_failure_rate",
            "Deployment failure rate (0.0-1.0)",
        ))
        .expect("valid metric");

        let mttr = Gauge::with_opts(Opts::new(
            "mttr_seconds",
            "Mean Time To Recovery in seconds",
        ))
        .expect("valid metric");

        // ---- Histograms ----
        let agent_duration = HistogramVec::new(
            HistogramOpts::new("agent_duration_seconds", "Agent turn duration")
                .buckets(vec![0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0]),
            &["model_provider", "model"],
        )
        .expect("valid metric");

        let tool_duration = HistogramVec::new(
            HistogramOpts::new("tool_duration_seconds", "Tool call duration")
                .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0]),
            &["tool", "success"],
        )
        .expect("valid metric");

        let request_latency = Histogram::with_opts(
            HistogramOpts::new("request_latency_seconds", "Single request latency")
                .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0]),
        )
        .expect("valid metric");

        let deployment_lead_time = Histogram::with_opts(
            HistogramOpts::new("deployment_lead_time_seconds", "Deployment lead time")
                .buckets(vec![60.0, 300.0, 600.0, 1800.0, 3600.0, 7200.0, 86400.0]),
        )
        .expect("valid metric");

        let recovery_time = Histogram::with_opts(
            HistogramOpts::new("recovery_time_seconds", "Recovery time")
                .buckets(vec![10.0, 30.0, 60.0, 300.0, 600.0, 1800.0, 3600.0]),
        )
        .expect("valid metric");

        // ---- Register all (clone 因为 register 接收 Box<dyn ...>) ----
        macro_rules! reg {
            ($m:expr) => {
                registry.register(Box::new($m)).ok()
            };
        }
        reg!(agent_starts.clone());
        reg!(llm_requests.clone());
        reg!(tokens_input_total.clone());
        reg!(tokens_output_total.clone());
        reg!(tool_calls.clone());
        reg!(channel_messages.clone());
        reg!(heartbeat_ticks.clone());
        reg!(errors.clone());
        reg!(cache_hits.clone());
        reg!(cache_misses.clone());
        reg!(cache_tokens_saved.clone());
        reg!(deployments_total.clone());
        reg!(tokens_used.clone());
        reg!(active_sessions.clone());
        reg!(queue_depth.clone());
        reg!(deployment_failure_rate.clone());
        reg!(mttr.clone());
        reg!(agent_duration.clone());
        reg!(tool_duration.clone());
        reg!(request_latency.clone());
        reg!(deployment_lead_time.clone());
        reg!(recovery_time.clone());

        Self {
            registry,
            agent_starts,
            llm_requests,
            tokens_input_total,
            tokens_output_total,
            tool_calls,
            channel_messages,
            heartbeat_ticks,
            errors,
            cache_hits,
            cache_misses,
            cache_tokens_saved,
            agent_duration,
            tool_duration,
            request_latency,
            tokens_used,
            active_sessions,
            queue_depth,
            deployments_total,
            deployment_lead_time,
            deployment_failure_rate,
            recovery_time,
            mttr,
            deploy_success_count: AtomicU64::new(0),
            deploy_failure_count: AtomicU64::new(0),
        }
    }

    pub fn encode(&self) -> String {
        let encoder = TextEncoder::new();
        let families = self.registry.gather();
        let mut buf = Vec::new();
        encoder.encode(&families, &mut buf).unwrap_or_default();
        String::from_utf8(buf).unwrap_or_default()
    }

    pub fn shared() -> Arc<Self> {
        static SINGLETON: OnceLock<Arc<PrometheusObserver>> = OnceLock::new();
        SINGLETON.get_or_init(|| Arc::new(Self::new())).clone()
    }

    /// 暴露 registry 给 HTTP exporter (如 prometheus_exporter) 使用
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// 重新计算发布失败率 = failures / (successes + failures)
    fn recompute_failure_rate(&self) {
        let s = self.deploy_success_count.load(Relaxed) as f64;
        let f = self.deploy_failure_count.load(Relaxed) as f64;
        let total = s + f;
        if total > 0.0 {
            self.deployment_failure_rate.set(f / total);
        }
    }
}

impl Observer for PrometheusObserver {
    fn record_event(&self, event: &ObserverEvent) {
        use ObserverEvent::*;
        match event {
            AgentStart {
                model_provider,
                model,
                ..
            } => {
                self.agent_starts
                    .with_label_values(&[model_provider, model])
                    .inc();
            }
            LlmRequest {
                model_provider,
                model,
                ..
            } => {
                // 请求开始时尚不知 success, 用 "pending" 占位
                self.llm_requests
                    .with_label_values(&[model_provider.as_str(), model.as_str(), "pending"])
                    .inc();
            }
            LlmResponse {
                model_provider,
                model,
                duration,
                success,
                input_tokens,
                output_tokens,
                ..
            } => {
                let success_str = if *success { "true" } else { "false" };
                self.llm_requests
                    .with_label_values(&[model_provider.as_str(), model.as_str(), success_str])
                    .inc();
                if let Some(inp) = input_tokens {
                    self.tokens_input_total
                        .with_label_values(&[model_provider, model])
                        .inc_by(*inp);
                }
                if let Some(out) = output_tokens {
                    self.tokens_output_total
                        .with_label_values(&[model_provider, model])
                        .inc_by(*out);
                }
                self.request_latency.observe(duration.as_secs_f64());
            }
            AgentEnd {
                model_provider,
                model,
                duration,
                token_used,
                ..
            } => {
                self.agent_duration
                    .with_label_values(&[model_provider, model])
                    .observe(duration.as_secs_f64());
                if let Some(usage) = token_used {
                    self.tokens_used
                        .set((usage.input_tokens + usage.output_tokens) as i64);
                }
            }
            ToolCallStart { .. } => {
                // 起始无独立 metric; 留作未来扩展
            }
            ToolCall {
                tool,
                duration,
                success,
                ..
            } => {
                let success_str = if *success { "true" } else { "false" };
                self.tool_calls
                    .with_label_values(&[tool.as_str(), success_str])
                    .inc();
                self.tool_duration
                    .with_label_values(&[tool.as_str(), success_str])
                    .observe(duration.as_secs_f64());
            }
            MemoryRecall { .. } | MemoryStore { .. } | RagRetrieve { .. } => {
                // 当前 PrometheusObserver 未为记忆/RAG 定义 metric, 留作未来扩展
            }
            TurnComplete => {
                // 无对应 metric
            }
            ChannelMessage { channel, direction } => {
                self.channel_messages
                    .with_label_values(&[channel, direction])
                    .inc();
            }
            HeartbeatTick => {
                self.heartbeat_ticks.inc();
            }
            CacheHit {
                cache_type,
                tokens_saved,
            } => {
                self.cache_hits.with_label_values(&[cache_type]).inc();
                self.cache_tokens_saved
                    .with_label_values(&[cache_type])
                    .inc_by(*tokens_saved);
            }
            CacheMiss { cache_type } => {
                self.cache_misses.with_label_values(&[cache_type]).inc();
            }
            Error { component, .. } => {
                self.errors.with_label_values(&[component]).inc();
            }
            DeploymentStart { .. } => {
                self.deployments_total.with_label_values(&["started"]).inc();
            }
            DeploymentComplete { .. } => {
                self.deploy_success_count.fetch_add(1, Relaxed);
                self.deployments_total
                    .with_label_values(&["completed"])
                    .inc();
                self.recompute_failure_rate();
            }
            DeploymentFail { .. } => {
                self.deploy_failure_count.fetch_add(1, Relaxed);
                self.deployments_total.with_label_values(&["failed"]).inc();
                self.recompute_failure_rate();
            }
            RecoveryComplete { .. } => {
                // MTTR 更新由 record_metric(RecoveryTime) 处理
            }
            HistoryTrimmed { .. } => {
                // 无对应 metric, 留作未来扩展
            }
            // ObserverEvent 是 #[non_exhaustive], 未来新增变体落到这里
            _ => {}
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        use ObserverMetric::*;
        match metric {
            RequestLatency(d) => {
                self.request_latency.observe(d.as_secs_f64());
            }
            TokenUsed(n) => {
                self.tokens_used.set(*n as i64);
            }
            ActiveSessions(n) => {
                let labels: [&str; 0] = [];
                self.active_sessions
                    .with_label_values(&labels)
                    .set(*n as f64);
            }
            QueueDepth(n) => {
                let labels: [&str; 0] = [];
                self.queue_depth.with_label_values(&labels).set(*n as f64);
            }
            DeploymentLeadTime(d) => {
                self.deployment_lead_time.observe(d.as_secs_f64());
            }
            RecoveryTime(d) => {
                let secs = d.as_secs_f64();
                self.recovery_time.observe(secs);
                // // MTTR 累计平均: prev*(n-1)/n + cur/n
                // // 分母用 deploy_failure_count 作为恢复次数的近似
                // // (RecoveryComplete 通常跟随 DeploymentFail)
                // let n = self.deploy_failure_count.load(Relaxed).max(1) as f64;
                // let prev = self.mttr.get();
                // let new_mttr = if prev == 0.0 {
                //     secs
                // } else {
                //     (prev * (n - 1.0) + secs) / n
                // };
                self.mttr.set(secs);
            }
        }
    }

    fn name(&self) -> &str {
        "prometheus"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
