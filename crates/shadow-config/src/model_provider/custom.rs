use crate::model_provider::ModelProviderConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomModelProviderConfig {
    #[serde(flatten)]
    pub base: ModelProviderConfig,
}
