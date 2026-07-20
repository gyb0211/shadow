pub mod noop;
pub mod prometheus;

use std::any::Any;
use std::sync::{Arc, OnceLock};
use parking_lot::RwLock;
use shadow_config::observability::ObservabilityBackend;
use shadow_config::ObservabilityConfig;
use shadow_core::{Observer, ObserverEvent};
use shadow_core::kennel::observer::ObserverMetric;
use shadow_log::record;
use crate::observability::noop::NoopObserver;
#[cfg(feature = "obs-prometheus")]
use crate::observability::prometheus::PrometheusObserver;

static BROADCAST_HOOK: OnceLock<RwLock<BroadcastHookState>> = OnceLock::new();


struct BroadcastHookEntry{
    scoped_id: Option<u64>,
    observer: Arc<dyn Observer>
}

#[derive(Default)]
struct BroadcastHookState {
    next_scoped_id: u64,
    entries: Vec<BroadcastHookEntry>,
}

impl BroadcastHookState {
    fn current(&self) -> Option<Arc<dyn  Observer>> {
        self.entries.last().map(|entry| entry.observer.clone())
    }
}

fn broadcast_hook_slot() -> &'static RwLock<BroadcastHookState>{
    BROADCAST_HOOK.get_or_init(|| RwLock::new(BroadcastHookState::default()))
}

fn current_broadcast_hook() -> Option<Arc<dyn Observer>>{
    broadcast_hook_slot().read().current()
}

pub fn create_observer(config: &ObservabilityConfig) -> Box<dyn  Observer>{
    Box::new(TeeObserver{
        primary: create_primary_observer(config),
    })
}

pub fn create_primary_observer(config: &ObservabilityConfig) -> Box<dyn  Observer> {
    match config.backend {
        ObservabilityBackend::None => Box::new(NoopObserver),
        ObservabilityBackend::Prometheus => {
            #[cfg(feature = "obs-prometheus")]
            {
                Box::new(PrometheusObserver::new())
            }
            #[cfg(not(feature = "obs-prometheus"))]
            {
                record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Note).with_outcome(shadow_log::EventOutcome::Unknown),
                    "Prometheus backend requested but this build was compiled without `obs-prometheus`; fallback to Noop "
                );
                Box::new(NoopObserver)
            }
        }
    }
}

struct TeeObserver{
    primary: Box<dyn  Observer>,
}

impl Observer for TeeObserver {
    fn record_event(&self, event: &ObserverEvent) {
        self.primary.record_event(event);
        if let Some(hook) = current_broadcast_hook() {
            hook.record_event(event);
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        todo!()
    }

    fn name(&self) -> &str {
        todo!()
    }

    fn as_any(&self) -> &dyn Any {
        todo!()
    }
}