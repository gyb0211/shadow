use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub enum ObservabilityBackend {
    #[default]
    None,
    Prometheus,
}