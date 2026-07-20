use shadow_core::kennel::observer::ObserverMetric;
use shadow_core::{Observer, ObserverEvent};
use std::any::Any;

pub struct NoopObserver;

impl Observer for NoopObserver {
    fn record_event(&self, _event: &ObserverEvent) {}

    fn record_metric(&self, _metric: &ObserverMetric) {}

    fn name(&self) -> &str {
        "noop"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
