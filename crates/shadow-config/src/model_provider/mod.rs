pub mod custom;

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub use custom::*;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelProviderConfig{
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub api_key: Option<String>,

    #[serde(default, skip_serializing_if="Option::is_none")]
    pub kind: Option<String>,

    #[serde(default, skip_serializing_if="Option::is_none")]
    pub uri: Option<String>,

    #[serde(default, skip_serializing_if="Option::is_none")]
    pub model: Option<String>,

    #[serde(default, skip_serializing_if="Option::is_none")]
    pub temperature: Option<f64>,

    #[serde(default, skip_serializing_if="Option::is_none")]
    pub timeout_secs: Option<u64>,

    #[serde(default, skip_serializing_if="HashMap::is_empty")]
    pub extra_headers: HashMap<String, String>,

    #[serde(default, skip_serializing_if="Option::is_none")]
    pub response_max_tokens: Option<u32>,

    #[serde(default, skip_serializing_if="Option::is_none")]
    pub native_tools: Option<bool>,

    #[serde(default, skip_serializing_if="Option::is_none")]
    pub think: Option<bool>,

    /// 上下文窗口 token最大值
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub context_window: Option<usize>,
}