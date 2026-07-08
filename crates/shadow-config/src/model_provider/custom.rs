use serde::{Deserialize, Serialize};
use crate::model_provider::ModelProviderConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomModelProviderConfig {
    #[serde(flatten)]
    pub base: ModelProviderConfig,
}