use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeliveryConfig{
    pub mode: String,
    pub channel: Option<String>,
    pub to: Option<String>,
}