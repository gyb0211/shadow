use crate::{ProviderDispatch, sanitize_api_error};
use anyhow::anyhow;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;
use shadow_core::{
    Attributable, ChatMessage, ChatRequest, ChatResponse, ModelInfo, ModelProvider,
    ProviderCapabilities, Role, StreamChunk, StreamEvent, StreamOptions, ToolSpec, ToolsPayload,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::format;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::task::id;

pub struct ReliableModelProviderEntry {
    display_name: String,
    cooldown_key: String,
    provider: Box<dyn ModelProvider>,
}

impl ReliableModelProviderEntry {
    pub fn new(
        display_name: impl Into<String>,
        cooldown_key: impl Into<String>,
        provider: Box<dyn ModelProvider>,
    ) -> Self {
        Self {
            display_name: display_name.into(),
            cooldown_key: cooldown_key.into(),
            provider,
        }
    }
}

pub struct ReliableModelProvider {
    alias: String,
    model_providers: Vec<ReliableModelProviderEntry>,
    max_retries: u32,
    base_backoff_ms: u64,
    api_keys: Vec<String>,
    key_index: AtomicUsize,
    model_fallbacks: HashMap<String, Vec<String>>,
    rate_limit_cooldowns: Mutex<HashMap<String, Instant>>,
}

impl ReliableModelProvider {
    pub fn new(
        alias: &str,
        model_providers: Vec<(String, Box<dyn ModelProvider>)>,
        max_retries: u32,
        base_backoff_ms: u64,
    ) -> Self {
        let model_providers = model_providers
            .into_iter()
            .map(|(display_name, provider)| {
                ReliableModelProviderEntry::new(display_name.clone(), display_name, provider)
            })
            .collect();

        Self::new_with_entries(alias, model_providers, max_retries, base_backoff_ms)
    }

    pub fn new_with_entries(
        alias: &str,
        model_providers: Vec<ReliableModelProviderEntry>,
        max_retries: u32,
        base_backoff_ms: u64,
    ) -> Self {
        Self {
            alias: alias.to_string(),
            model_providers,
            max_retries,
            base_backoff_ms: base_backoff_ms.max(50),
            api_keys: Vec::new(),
            key_index: AtomicUsize::new(0),
            model_fallbacks: HashMap::new(),
            rate_limit_cooldowns: Mutex::new(HashMap::new()),
        }
    }

    fn model_chain<'a>(&'a self, model: &'a str) -> Vec<&'a str> {
        let mut chain = vec![model];
        if let Some(fallbacks) = self.model_fallbacks.get(model) {
            chain.extend(fallbacks.iter().map(|f| f.as_str()));
        }
        chain
    }

    async fn backoff_after_empty_completion(
        &self,
        failures: &mut Vec<String>,
        provider_name: &str,
        model: &str,
        attempt: u32,
        backoff_ms: &mut u64,
    ) {
        push_failure(
            failures,
            provider_name,
            model,
            attempt + 1,
            self.max_retries + 1,
            "empty_response",
            "model_provider returned an empty completion",
            None,
        );

        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_outcome(shadow_log::EventOutcome::Unknown)
                .with_attrs(serde_json::json!({
                    "model_provider": provider_name,
                    "model": model,
                    "attempt": attempt + 1,
                    "backoff_ms": *backoff_ms
                })),
            "Empty completion; retrying"
        );

        tokio::time::sleep(Duration::from_millis(*backoff_ms)).await;
        *backoff_ms = backoff_ms.saturating_mul(2).min(10_000);
    }

    fn compute_backoff(&self, base: u64, err: &anyhow::Error) -> u64 {
        if let Some(retry_after) = parse_retry_after_ms(err) {
            retry_after.min(30_000).max(base)
        } else {
            base
        }
    }

    fn rotate_key(&self) -> Option<&str> {
        if self.api_keys.is_empty() {
            return None;
        }

        let idx = self.key_index.fetch_add(1, Ordering::Relaxed) % self.api_keys.len();
        Some(&self.api_keys[idx])
    }
    const RARE_LIMIT_COOLDOWN: Duration = Duration::from_secs(10);
    fn set_rate_limit_cooldown(&self, cd_key: &str, err: &anyhow::Error) -> Duration {
        let cd = parse_retry_after_ms(err)
            .map(|ms| Duration::from_millis(ms.min(60_000)))
            .unwrap_or(Self::RARE_LIMIT_COOLDOWN);

        let mut cds = self
            .rate_limit_cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cds.insert(cd_key.to_string(), Instant::now() + cd);
        cd
    }

    fn cool_down_rate_limited_provider(
        &self,
        entry: &ReliableModelProviderEntry,
        model: &str,
        err: &anyhow::Error,
    ) -> Duration {
        let cooldown = self.set_rate_limit_cooldown(&entry.cooldown_key, err);
        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note).with_attrs(
                serde_json::json!({
                    "model_provider": entry.display_name,
                    "model": model,
                    "cooldown_ms": cooldown.as_millis()
                })
            ),
            "ModelProvider rate-limited; trying next provider"
        );
        cooldown
    }

    fn provider_should_skip_for_cooldown(&self, entry: &ReliableModelProviderEntry) -> bool {
        self.model_providers.len() > 1 && self.provider_cooldown_active(&entry.cooldown_key)
    }

    fn provider_cooldown_active(&self, cd_key: &str) -> bool {
        let now = Instant::now();
        let mut cds = self
            .rate_limit_cooldowns
            .lock()
            .unwrap_or_else(|posioned| posioned.into_inner());
        match cds.get(cd_key).copied() {
            None => false,
            Some(deadline) if now < deadline => true,
            Some(_) => {
                cds.remove(cd_key);
                false
            }
        }
    }

    fn log_cooldown_skip(&self, provider_name: &str) {
        shadow_log::record!(
            DEBUG,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note).with_attrs(
                serde_json::json!({
                    "model_provider": provider_name
                })
            ),
            "Skipping model_provider during rate-limit cooldown"
        );
    }

    fn record_cooldown_skip_failure(failures: &mut Vec<String>, provider_name: &str, model: &str) {
        failures.push(format!(
            "model_provider={provider_name} model={model}: skipped; reason=rate_limit_cooldown"
        ));
    }
}

impl Attributable for ReliableModelProvider {
    fn role(&self) -> Role {
        Role::System
    }

    fn alias(&self) -> &str {
        &self.alias
    }
}

#[async_trait]
impl ModelProvider for ReliableModelProvider {
    fn supports_native_tools(&self) -> bool {
        self.model_providers
            .first()
            .map(|entry| entry.provider.supports_native_tools())
            .unwrap_or(false)
    }

    fn supports_vision(&self) -> bool {
        self.model_providers
            .first()
            .map(|entry| entry.provider.supports_vision())
            .unwrap_or(false)
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        for entry in &self.model_providers {
            let provider_name = entry.display_name.as_str();
            shadow_log::record!(
                INFO,
                shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                    .with_attrs(serde_json::json!({"model_provider": provider_name})),
                "Warming up model_provider connection pool"
            );

            if ProviderDispatch::from_ref(entry.provider.as_ref())
                .warmup()
                .await
                .is_err()
            {
                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_outcome(shadow_log::EventOutcome::Unknown)
                        .with_attrs(serde_json::json!({
                            "model_provider": provider_name
                        })),
                    "Warmup failed (non-fatal)"
                );
            }
        }

        Ok(())
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        let models = self.model_chain(model);
        let mut failures = Vec::new();

        for curr_model in &models {
            for entry in &self.model_providers {
                let provider_name = entry.display_name.as_str();
                if self.provider_should_skip_for_cooldown(entry) {
                    self.log_cooldown_skip(provider_name);
                    Self::record_cooldown_skip_failure(&mut failures, provider_name, curr_model);
                    continue;
                }

                let mut backoff_ms = self.base_backoff_ms;
                let mut last_error_detail: Option<String> = None;
                let mut last_diagnostic: Option<ProviderErrorDiagnostic> = None;

                for attempt in 0..self.max_retries {
                    match ProviderDispatch::from_ref(entry.provider.as_ref())
                        .chat_with_system(system_prompt, message, curr_model, temperature)
                        .await
                    {
                        Ok(resp) => {
                            if attempt < self.max_retries && resp.trim().is_empty() {
                                self.backoff_after_empty_completion(
                                    &mut failures,
                                    provider_name,
                                    curr_model,
                                    attempt,
                                    &mut backoff_ms,
                                )
                                .await;
                                continue;
                            }

                            if attempt > 0
                                || *curr_model != model
                                || self
                                    .model_providers
                                    .first()
                                    .map(|entry| entry.display_name.as_str())
                                    != Some(provider_name)
                            {
                                shadow_log::record!(
                                    INFO,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_attrs(
                                        ::serde_json::json!({
                                            "model_provider": provider_name,
                                            "model": *curr_model,
                                            "attempt": attempt,
                                            "original_model": model,
                                        })
                                    ),
                                    "ModelProvider recovered (failover/retry)"
                                );
                                let primary = self
                                    .model_providers
                                    .first()
                                    .map(|entry| entry.display_name.as_str())
                                    .unwrap_or("");

                                record_provider_fallback(primary, model, provider_name, curr_model);
                            }

                            return Ok(resp);
                        }
                        Err(e) => {
                            if is_context_window_exceeded(&e) {
                                let error_detail = compact_error_detail(&e);
                                push_failure(
                                    &mut failures,
                                    provider_name,
                                    curr_model,
                                    attempt + 1,
                                    self.max_retries + 1,
                                    "non_retryable",
                                    &error_detail,
                                    None,
                                );

                                anyhow::bail!(
                                    "Request exceeds model context window. Attempts:\n{}",
                                    failures.join("\n")
                                );
                            }

                            let non_retryable_rate_limit = is_non_retryable_rate_limit(&e);
                            let non_retryable = non_retryable_rate_limit || is_non_retryable(&e);
                            let rate_limited = is_rate_limited(&e);
                            let failure_reason = failure_reason(rate_limited, non_retryable);
                            let error_detail = compact_error_detail(&e);
                            let diagnostic = provider_error_diagnostic(&e);
                            last_error_detail = Some(error_detail.clone());
                            last_diagnostic = Some(diagnostic.clone());

                            push_failure(
                                &mut failures,
                                provider_name,
                                curr_model,
                                attempt + 1,
                                self.max_retries + 1,
                                failure_reason,
                                &error_detail,
                                Some(&diagnostic),
                            );

                            if rate_limited
                                && !non_retryable_rate_limit
                                && let Some(new_key) = self.rotate_key()
                            {
                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        serde_json::json!({
                                            "model_provider": provider_name,
                                            "error": error_detail
                                        })
                                    ),
                                    &format!(
                                        "Rate limited; ket rotation selected key ending ... {} \
                                        but cannot apply (ModelProvider trait has no set_api_key.) \
                                        Retry with original key.",
                                        &new_key[new_key.len().saturating_sub(4)..]
                                    )
                                )
                            }

                            if non_retryable {
                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        provider_failure_attrs(
                                            provider_name,
                                            curr_model,
                                            &error_detail,
                                            &diagnostic
                                        )
                                    ),
                                    "Non-retryable error, moving on"
                                );
                                break;
                            }

                            if rate_limited && self.model_providers.len() > 1 {
                                self.cool_down_rate_limited_provider(entry, curr_model, &e);
                                break;
                            }

                            if attempt < self.max_retries {
                                let wait = self.compute_backoff(backoff_ms, &e);

                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        ::shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        provider_retry_attrs(
                                            provider_name,
                                            curr_model,
                                            attempt + 1,
                                            wait,
                                            failure_reason,
                                            &error_detail,
                                            &diagnostic,
                                        )
                                    ),
                                    "ModelProvider call failed, retrying"
                                );

                                tokio::time::sleep(Duration::from_millis(wait)).await;
                                backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                            }
                        }
                    }
                }

                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_outcome(shadow_log::EventOutcome::Unknown)
                        .with_attrs(provider_exhausted_attrs(
                            provider_name,
                            curr_model,
                            last_error_detail.as_deref(),
                            last_diagnostic.as_ref(),
                        )),
                    "Exhausted retries, trying next model_provider/model"
                )
            }

            if *curr_model != model {
                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_outcome(shadow_log::EventOutcome::Unknown)
                        .with_attrs(serde_json::json!({
                            "original_model": model, "fallback_model": *curr_model
                        })),
                    "Model fallback exhuasted all model_provider, trying next fallback model"
                )
            }
        }

        anyhow::bail!(
            "All model_providers/models failed. Attempts:\n{}",
            failures.join("\n")
        )
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        let models = self.model_chain(model);
        let mut failures = Vec::new();
        let mut eff_msgs = messages.to_vec();
        let mut context_truncated = false;

        for curr_model in &models {
            for entry in &self.model_providers {
                let provider_name = entry.display_name.as_str();
                if self.provider_should_skip_for_cooldown(entry) {
                    self.log_cooldown_skip(provider_name);
                    Self::record_cooldown_skip_failure(&mut failures, provider_name, curr_model);
                    continue;
                }

                let mut backoff_ms = self.base_backoff_ms;
                let mut last_error_detail: Option<String> = None;
                let mut last_diagnostic: Option<ProviderErrorDiagnostic> = None;

                for attempt in 0..self.max_retries {
                    match ProviderDispatch::from_ref(entry.provider.as_ref())
                        .chat_with_history(&eff_msgs, curr_model, temperature)
                        .await
                    {
                        Ok(resp) => {
                            if attempt < self.max_retries && resp.trim().is_empty() {
                                self.backoff_after_empty_completion(
                                    &mut failures,
                                    provider_name,
                                    curr_model,
                                    attempt,
                                    &mut backoff_ms,
                                )
                                .await;
                                continue;
                            }

                            if attempt > 0
                                || *curr_model != model
                                || context_truncated
                                || self
                                    .model_providers
                                    .first()
                                    .map(|entry| entry.display_name.as_str())
                                    != Some(provider_name)
                            {
                                shadow_log::record!(
                                    INFO,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_attrs(
                                        ::serde_json::json!({
                                            "model_provider": provider_name,
                                            "model": *curr_model,
                                            "attempt": attempt,
                                            "original_model": model,
                                            "context_truncated": context_truncated
                                        })
                                    ),
                                    "ModelProvider recovered (failover/retry)"
                                );
                                let primary = self
                                    .model_providers
                                    .first()
                                    .map(|entry| entry.display_name.as_str())
                                    .unwrap_or("");

                                record_provider_fallback(primary, model, provider_name, curr_model);
                            }

                            return Ok(resp);
                        }
                        Err(e) => {
                            if is_context_window_exceeded(&e) && !context_truncated {
                                let dropped = truncate_for_context(&mut eff_msgs);
                                if dropped > 0 {
                                    context_truncated = true;
                                    shadow_log::record!(
                                        WARN,
                                        shadow_log::Event::new(
                                            module_path!(),
                                            shadow_log::Action::Note
                                        ).with_attrs(
                                            ::serde_json::json!({"model_provider": provider_name, "model": *curr_model, "dropped": dropped, "remaining": eff_msgs.len()})
                                        ).with_outcome(shadow_log::EventOutcome::Unknown),
                                        "Context window exceeded; truncated history and retrying"
                                    );
                                    continue;
                                }

                                let error_detail = compact_error_detail(&e);
                                push_failure(
                                    &mut failures,
                                    provider_name,
                                    curr_model,
                                    attempt + 1,
                                    self.max_retries + 1,
                                    "non_retryable",
                                    &error_detail,
                                    None,
                                );

                                anyhow::bail!(
                                    "Request exceeds model context window and cannot be reduced further.\
                                    Try using a model with a larger context window, reducing the number \
                                    of tools/skills, or enabling compact_context in config. Attempts:\n{}",
                                    failures.join("\n")
                                );
                            }

                            let non_retryable_rate_limit = is_non_retryable_rate_limit(&e);
                            let non_retryable = non_retryable_rate_limit || is_non_retryable(&e);
                            let rate_limited = is_rate_limited(&e);
                            let failure_reason = failure_reason(rate_limited, non_retryable);
                            let error_detail = compact_error_detail(&e);
                            let diagnostic = provider_error_diagnostic(&e);
                            last_error_detail = Some(error_detail.clone());
                            last_diagnostic = Some(diagnostic.clone());

                            push_failure(
                                &mut failures,
                                provider_name,
                                curr_model,
                                attempt + 1,
                                self.max_retries + 1,
                                failure_reason,
                                &error_detail,
                                Some(&diagnostic),
                            );

                            if rate_limited
                                && !non_retryable_rate_limit
                                && let Some(new_key) = self.rotate_key()
                            {
                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        serde_json::json!({
                                            "model_provider": provider_name,
                                            "error": error_detail
                                        })
                                    ),
                                    &format!(
                                        "Rate limited; ket rotation selected key ending ... {} \
                                        but cannot apply (ModelProvider trait has no set_api_key.) \
                                        Retry with original key.",
                                        &new_key[new_key.len().saturating_sub(4)..]
                                    )
                                )
                            }

                            if non_retryable {
                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        provider_failure_attrs(
                                            provider_name,
                                            curr_model,
                                            &error_detail,
                                            &diagnostic
                                        )
                                    ),
                                    "Non-retryable error, moving on"
                                );
                                break;
                            }

                            if rate_limited && self.model_providers.len() > 1 {
                                self.cool_down_rate_limited_provider(entry, curr_model, &e);
                                break;
                            }

                            if attempt < self.max_retries {
                                let wait = self.compute_backoff(backoff_ms, &e);

                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        ::shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        provider_retry_attrs(
                                            provider_name,
                                            curr_model,
                                            attempt + 1,
                                            wait,
                                            failure_reason,
                                            &error_detail,
                                            &diagnostic,
                                        )
                                    ),
                                    "ModelProvider call failed, retrying"
                                );

                                tokio::time::sleep(Duration::from_millis(wait)).await;
                                backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                            }
                        }
                    }
                }

                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_outcome(shadow_log::EventOutcome::Unknown)
                        .with_attrs(provider_exhausted_attrs(
                            provider_name,
                            curr_model,
                            last_error_detail.as_deref(),
                            last_diagnostic.as_ref(),
                        )),
                    "Exhausted retries, trying next model_provider/model"
                )
            }
        }

        anyhow::bail!(
            "All model_providers/models failed. Attempts:\n{}",
            failures.join("\n")
        )
    }

    async fn chat_with_tools(&self, messages: &[ChatMessage], tools: &[Value], model: &str, temperature: Option<f64>) -> anyhow::Result<ChatResponse> {
        let models = self.model_chain(model);
        let mut failures = Vec::new();
        let mut eff_msgs = messages.to_vec();
        let mut context_truncated = false;

        for curr_model in &models {
            for entry in &self.model_providers {
                let provider_name = entry.display_name.as_str();
                if self.provider_should_skip_for_cooldown(entry) {
                    self.log_cooldown_skip(provider_name);
                    Self::record_cooldown_skip_failure(&mut failures, provider_name, curr_model);
                    continue;
                }

                let mut backoff_ms = self.base_backoff_ms;
                let mut last_error_detail: Option<String> = None;
                let mut last_diagnostic: Option<ProviderErrorDiagnostic> = None;

                for attempt in 0..self.max_retries {
                    match ProviderDispatch::from_ref(entry.provider.as_ref())
                        .chat_with_tools(&eff_msgs, tools, curr_model, temperature)
                        .await
                    {
                        Ok(resp) => {
                            if attempt < self.max_retries && resp.trim().is_empty() {
                                self.backoff_after_empty_completion(
                                    &mut failures,
                                    provider_name,
                                    curr_model,
                                    attempt,
                                    &mut backoff_ms,
                                )
                                    .await;
                                continue;
                            }

                            if attempt > 0
                                || *curr_model != model
                                || context_truncated
                                || self
                                .model_providers
                                .first()
                                .map(|entry| entry.display_name.as_str())
                                != Some(provider_name)
                            {
                                shadow_log::record!(
                                    INFO,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_attrs(
                                        ::serde_json::json!({
                                            "model_provider": provider_name,
                                            "model": *curr_model,
                                            "attempt": attempt,
                                            "original_model": model,
                                            "context_truncated": context_truncated
                                        })
                                    ),
                                    "ModelProvider recovered (failover/retry)"
                                );
                                let primary = self
                                    .model_providers
                                    .first()
                                    .map(|entry| entry.display_name.as_str())
                                    .unwrap_or("");

                                record_provider_fallback(primary, model, provider_name, curr_model);
                            }

                            return Ok(resp);
                        }
                        Err(e) => {
                            if is_context_window_exceeded(&e) && !context_truncated {
                                let dropped = truncate_for_context(&mut eff_msgs);
                                if dropped > 0 {
                                    context_truncated = true;
                                    shadow_log::record!(
                                        WARN,
                                        shadow_log::Event::new(
                                            module_path!(),
                                            shadow_log::Action::Note
                                        ).with_attrs(
                                            ::serde_json::json!({"model_provider": provider_name, "model": *curr_model, "dropped": dropped, "remaining": eff_msgs.len()})
                                        ).with_outcome(shadow_log::EventOutcome::Unknown),
                                        "Context window exceeded; truncated history and retrying"
                                    );
                                    continue;
                                }

                                let error_detail = compact_error_detail(&e);
                                push_failure(
                                    &mut failures,
                                    provider_name,
                                    curr_model,
                                    attempt + 1,
                                    self.max_retries + 1,
                                    "non_retryable",
                                    &error_detail,
                                    None,
                                );

                                anyhow::bail!(
                                    "Request exceeds model context window and cannot be reduced further.\
                                    Try using a model with a larger context window, reducing the number \
                                    of tools/skills, or enabling compact_context in config. Attempts:\n{}",
                                    failures.join("\n")
                                );
                            }

                            let non_retryable_rate_limit = is_non_retryable_rate_limit(&e);
                            let non_retryable = non_retryable_rate_limit || is_non_retryable(&e);
                            let rate_limited = is_rate_limited(&e);
                            let failure_reason = failure_reason(rate_limited, non_retryable);
                            let error_detail = compact_error_detail(&e);
                            let diagnostic = provider_error_diagnostic(&e);
                            last_error_detail = Some(error_detail.clone());
                            last_diagnostic = Some(diagnostic.clone());

                            push_failure(
                                &mut failures,
                                provider_name,
                                curr_model,
                                attempt + 1,
                                self.max_retries + 1,
                                failure_reason,
                                &error_detail,
                                Some(&diagnostic),
                            );

                            if rate_limited
                                && !non_retryable_rate_limit
                                && let Some(new_key) = self.rotate_key()
                            {
                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        serde_json::json!({
                                            "model_provider": provider_name,
                                            "error": error_detail
                                        })
                                    ),
                                    &format!(
                                        "Rate limited; ket rotation selected key ending ... {} \
                                        but cannot apply (ModelProvider trait has no set_api_key.) \
                                        Retry with original key.",
                                        &new_key[new_key.len().saturating_sub(4)..]
                                    )
                                )
                            }

                            if non_retryable {
                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        provider_failure_attrs(
                                            provider_name,
                                            curr_model,
                                            &error_detail,
                                            &diagnostic
                                        )
                                    ),
                                    "Non-retryable error, moving on"
                                );
                                break;
                            }

                            if rate_limited && self.model_providers.len() > 1 {
                                self.cool_down_rate_limited_provider(entry, curr_model, &e);
                                break;
                            }

                            if attempt < self.max_retries {
                                let wait = self.compute_backoff(backoff_ms, &e);

                                shadow_log::record!(
                                    WARN,
                                    shadow_log::Event::new(
                                        module_path!(),
                                        ::shadow_log::Action::Note
                                    )
                                    .with_outcome(shadow_log::EventOutcome::Unknown)
                                    .with_attrs(
                                        provider_retry_attrs(
                                            provider_name,
                                            curr_model,
                                            attempt + 1,
                                            wait,
                                            failure_reason,
                                            &error_detail,
                                            &diagnostic,
                                        )
                                    ),
                                    "ModelProvider call failed, retrying"
                                );

                                tokio::time::sleep(Duration::from_millis(wait)).await;
                                backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                            }
                        }
                    }
                }

                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                        .with_outcome(shadow_log::EventOutcome::Unknown)
                        .with_attrs(provider_exhausted_attrs(
                            provider_name,
                            curr_model,
                            last_error_detail.as_deref(),
                            last_diagnostic.as_ref(),
                        )),
                    "Exhausted retries, trying next model_provider/model"
                )
            }
        }

        anyhow::bail!(
            "All model_providers/models failed. Attempts:\n{}",
            failures.join("\n")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderErrorDiagnostic {
    kind: &'static str,
    phase: &'static str,
    hint: &'static str,
    endpoint: Option<String>,
}

fn parse_retry_after_ms(err: &anyhow::Error) -> Option<u64> {
    let msg = err.to_string();
    let lower = msg.to_lowercase();

    for prefix in &["retry-after:", "retry_after:", "retry-after", "retry_after"] {
        if let Some(pos) = lower.find(prefix) {
            let after = &msg[pos + prefix.len()..];
            let num_str: String = after
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(secs) = num_str.parse::<f64>()
                && secs.is_finite()
                && secs >= 0.0
            {
                let millis = Duration::from_secs_f64(secs).as_millis();
                if let Ok(value) = u64::try_from(millis) {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn compact_error_detail(err: &anyhow::Error) -> String {
    sanitize_api_error(&format!("{err:#}"))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn is_context_window_exceeded(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    let hints = [
        "exceeds the context window",
        "exceeds the available context size",
        "context window of this model",
        "maximum context length",
        "context length exceeded",
        "too many tokens",
        "token limit exceeded",
        "prompt is too long",
        "input is too long",
        "prompt exceeds max length",
    ];

    hints.iter().any(|hint| lower.contains(hint))
}

fn failure_reason(rate_limited: bool, non_retryable: bool) -> &'static str {
    if rate_limited && non_retryable {
        "rate_limited_non_retryable"
    } else if rate_limited {
        "rate_limited"
    } else if non_retryable {
        "non_retryable"
    } else {
        "retryable"
    }
}

fn truncate_for_context(messages: &mut Vec<ChatMessage>) -> usize {
    let non_system: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.is_system())
        .map(|(i, _)| i)
        .collect();

    if non_system.len() <= 1 {
        return 0;
    }

    let drop_count = non_system.len() / 2;
    let indices_to_remove: Vec<usize> = non_system[..drop_count].to_vec();
    for &idx in indices_to_remove.iter().rev() {
        messages.remove(idx);
    }
    drop_count
}

#[derive(Debug, Clone)]
pub struct ProviderFallbackInfo {
    pub requested_provider: String,
    pub requested_model: String,

    pub actual_provider: String,
    pub actual_model: String,
}

tokio::task_local! {
    static PROVIDER_FALLBACK: RefCell<Option<ProviderFallbackInfo>>
}

fn record_provider_fallback(
    requested_provider: &str,
    requested_model: &str,
    actual_provider: &str,
    actual_model: &str,
) {
    let _ = PROVIDER_FALLBACK.try_with(|cell| {
        *cell.borrow_mut() = Some(ProviderFallbackInfo {
            requested_provider: requested_provider.to_string(),
            requested_model: requested_model.to_string(),
            actual_provider: actual_provider.to_string(),
            actual_model: actual_model.to_string(),
        })
    });
}

fn provider_failure_attrs(
    provider_name: &str,
    model: &str,
    error_detail: &str,
    diagnostic: &ProviderErrorDiagnostic,
) -> serde_json::Value {
    serde_json::json!({
        "model_provider": provider_name,
        "model":model,
         "error": error_detail,
        "error_kind": diagnostic.kind,
        "error_phase": diagnostic.phase,
        "endpoint": diagnostic.endpoint.as_deref(),
        "hint": diagnostic.hint,
    })
}
fn provider_exhausted_attrs(
    provider_name: &str,
    model: &str,
    last_error_detail: Option<&str>,
    last_diagnostic: Option<&ProviderErrorDiagnostic>,
) -> serde_json::Value {
    serde_json::json!({
        "model_provider": provider_name,
        "model":model,
         "error": last_error_detail,
        "error_kind": last_diagnostic.map(|d| d.kind),
        "error_phase": last_diagnostic.map(|d| d.phase),
        "endpoint": last_diagnostic.map(|d| d.endpoint.as_deref()),
        "hint": last_diagnostic.map(|d| d.hint),
    })
}

fn provider_retry_attrs(
    provider_name: &str,
    model: &str,
    attempt: u32,
    backoff_ms: u64,
    reason: &str,
    error_detail: &str,
    diagnostic: &ProviderErrorDiagnostic,
) -> serde_json::Value {
    serde_json::json!({
        "model_provider": provider_name,
        "model":model,
        "attempt":attempt,
        "backoff_ms":backoff_ms,
        "reason":reason,
         "error": error_detail,
        "error_kind": diagnostic.kind,
        "error_phase": diagnostic.phase,
        "endpoint": diagnostic.endpoint.as_deref(),
        "hint": diagnostic.hint,
    })
}

fn push_failure(
    failures: &mut Vec<String>,
    provider_name: &str,
    model: &str,
    attempt: u32,
    max_attempts: u32,
    reason: &str,
    error_detail: &str,
    diagnostic: Option<&ProviderErrorDiagnostic>,
) {
    let mut failure = format!(
        "model_provider={provider_name} model={model}, attempt={attempt}/{max_attempts}: {reason}; error={error_detail}"
    );

    if let Some(diagnostic) = diagnostic {
        failure.push_str(&format!(
            "; kind={}; phase={}; hint={}",
            diagnostic.kind, diagnostic.phase, diagnostic.hint
        ));

        if let Some(endpoint) = diagnostic.endpoint.as_deref() {
            failure.push_str(&format!("; endpoint={endpoint}"));
        }
    }

    failures.push(failure)
}

fn is_non_retryable_rate_limit(err: &anyhow::Error) -> bool {
    if !is_rate_limited(err) {
        return false;
    }

    let msg = err.to_string();
    let lower = msg.to_lowercase();

    let business_hints = [
        "plan does not include",
        "doesn't include",
        "not include",
        "insufficient balance",
        "insufficient_balance",
        "insufficient quota",
        "insufficient_quota",
        "quota exhausted",
        "out of credits",
        "no available package",
        "package not active",
        "purchase package",
        "model not available for your plan",
    ];

    if business_hints.iter().any(|b| lower.contains(b)) {
        return true;
    }

    for token in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>()
            && matches!(code, 1113 | 1311)
        {
            return true;
        }
    }

    false
}

fn sanitized_url_endpoint(mut url: reqwest::Url) -> String {
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    sanitize_api_error(url.as_ref())
}

fn endpoint_from_error_text(text: &str) -> Option<String> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let raw = text[start..]
        .split(|c: char| c.is_whitespace() || matches!(c, ')' | ',' | ';' | '"'))
        .next()
        .unwrap_or("");
    let url = reqwest::Url::parse(raw)
        .or_else(|_| reqwest::Url::parse(raw.trim_end_matches([':', '.'])))
        .ok()?;
    Some(sanitized_url_endpoint(url))
}

pub fn is_auth_error(err: &anyhow::Error) -> bool {
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>()
        && let Some(status) = reqwest_err.status()
    {
        let code = status.as_u16();
        return code == 401 || code == 403;
    }

    let msg = err.to_string().to_lowercase();
    let hints = [
        "401 unauthorized",
        "403 forbidden",
        "invalid api key",
        "incorrect api key",
        "authentication failed",
        "auth failed",
        "unauthorized",
        "invalid token",
        "token expired",
        "access_token",
    ];
    hints.iter().any(|hint| msg.contains(hint))
}

fn provider_error_diagnostic(err: &anyhow::Error) -> ProviderErrorDiagnostic {
    let error_detail = compact_error_detail(err);
    let lower = error_detail.to_lowercase();
    let endpoint = err
        .downcast_ref::<reqwest::Error>()
        .and_then(|e| e.url().cloned().map(sanitized_url_endpoint))
        .or_else(|| endpoint_from_error_text(&error_detail));

    if is_context_window_exceeded(err) {
        return ProviderErrorDiagnostic {
            kind: "context_window",
            phase: "request_validation",
            hint: "reduce context or use a larger-context model",
            endpoint,
        };
    }

    if is_auth_error(err) {
        return ProviderErrorDiagnostic {
            kind: "auth",
            phase: "http_response",
            hint: "check provider credentials",
            endpoint,
        };
    }

    if is_rate_limited(err) {
        return ProviderErrorDiagnostic {
            kind: "rate_limited",
            phase: "http_response",
            hint: "wait, change key/quota, or switch provider",
            endpoint,
        };
    }

    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            let code = status.as_u16();
            let (kind, hint) = if status.is_server_error() {
                (
                    "provider_server",
                    "provider returned a server error; retry or switch provider",
                )
            } else if code == 404 {
                (
                    "model_not_found",
                    "check the configured model id for this provider",
                )
            } else if status.is_client_error() {
                (
                    "client_error",
                    "provider rejected the request; check config, model, or request shape",
                )
            } else {
                ("http_error", "inspect provider response or switch provider")
            };

            return ProviderErrorDiagnostic {
                kind,
                phase: "http_response",
                hint,
                endpoint,
            };
        }

        if reqwest_err.is_timeout() && reqwest_err.is_connect() {
            return ProviderErrorDiagnostic {
                kind: "connect_timeout",
                phase: "tls_or_connect",
                hint: "connection reached the host but timed out during connect/TLS; check VPN, firewall, routing, or switch provider",
                endpoint,
            };
        }

        if reqwest_err.is_timeout() {
            return ProviderErrorDiagnostic {
                kind: "timeout",
                phase: "request",
                hint: "provider request timed out; retry or switch provider",
                endpoint,
            };
        }

        if reqwest_err.is_connect() {
            return ProviderErrorDiagnostic {
                kind: "connect",
                phase: "connect",
                hint: "could not open provider connection; check network, VPN or firewall",
                endpoint,
            };
        }
    }

    if (lower.contains("client err (connect)") && lower.contains("timed out"))
        || lower.contains("ssl connection timeout")
        || (lower.contains("tls") && lower.contains("timeout"))
    {
        return ProviderErrorDiagnostic {
            kind: "connect_timeout",
            phase: "tls_or_connect",
            hint: "connection reached the host but timed out during connect/TLS; check VPN, firewall, routing, or switch provider",
            endpoint,
        };
    }

    if lower.contains("timed out") || lower.contains("timeout") {
        return ProviderErrorDiagnostic {
            kind: "timeout",
            phase: "request",
            hint: "provider request timed out; retry or switch provider",
            endpoint,
        };
    }

    if lower.contains("dns") || lower.contains("resolve") {
        return ProviderErrorDiagnostic {
            kind: "dns",
            phase: "dns",
            hint: "DNS resolution failed; check network or provider host",
            endpoint,
        };
    }

    if lower.contains("model")
        && (lower.contains("not found")
            || lower.contains("unknown")
            || lower.contains("unsupported")
            || lower.contains("does not exist")
            || lower.contains("invalid"))
    {
        return ProviderErrorDiagnostic {
            kind: "model_not_found",
            phase: "http_response",
            hint: "check the configured model id for this provider",
            endpoint,
        };
    }

    ProviderErrorDiagnostic {
        kind: "provider_error",
        phase: "unknown",
        hint: "inspect provider error or switch provider",
        endpoint,
    }
}

pub fn is_tool_schema_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    let hints = [
        "tool call validation failed",
        "was not in request",
        "not found in tool list",
        "invalid_tool_call",
    ];

    hints.iter().any(|hint| lower.contains(hint))
}

pub fn is_non_retryable(err: &anyhow::Error) -> bool {
    if is_context_window_exceeded(err) {
        return false;
    }

    if is_tool_schema_error(err) {
        return false;
    }

    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>()
        && let Some(status) = reqwest_err.status()
    {
        let code = status.as_u16();
        return status.is_client_error() && code != 429 && code != 408;
    }

    let msg = err.to_string();

    for word in msg.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = word.parse::<u16>()
            && (400..500).contains(&code)
        {
            return code != 429 && code != 408;
        }
    }

    let msg_lower = msg.to_lowercase();
    let auth_failure_hints = [
        "invalid api key",
        "incorrect api key",
        "missing api key",
        "api key not set",
        "authentication failed",
        "auth failed",
        "unauthorized",
        "forbidden",
        "permission denied",
        "access denied",
        "invalid token",
    ];

    if auth_failure_hints
        .iter()
        .any(|hint| msg_lower.contains(hint))
    {
        return true;
    }

    msg_lower.contains("model")
        && (msg_lower.contains("not found")
            || msg_lower.contains("unknown")
            || msg_lower.contains("unsupported")
            || msg_lower.contains("does not exist")
            || msg_lower.contains("invalid"))
}

fn is_rate_limited(err: &anyhow::Error) -> bool {
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>()
        && let Some(status) = reqwest_err.status()
    {
        return status.as_u16() == 429;
    }

    let msg = err.to_string();
    msg.contains("429")
        && (msg.contains("Too Many") || msg.contains("rate") || msg.contains("limit"))
}
