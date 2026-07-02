//! 影子配置层 -- TOML schema + 加载 + 多 provider 支持
//!
//! 借鉴 ZeroClaw 的 `providers.models.<family>.<alias>` 设计,
//! 精简为 `providers.<family>.<alias>` 的 flatten HashMap:
//!
//! ```toml
//! [agent]
//! alias = "default"
//! model_provider = "openai.default"   # 引用 providers 表
//! model = "gpt-4o-mini"
//! temperature = 0.7
//! autonomy = "supervised"
//!
//! [providers.openai.default]
//! api_key = "sk-xxx"
//! model = "gpt-4o-mini"
//! base_url = "https://api.openai.com/v1"
//!
//! [providers.anthropic.claude]
//! api_key = "sk-ant-xxx"
//! model = "claude-sonnet-4-20250514"
//!
//! [providers.custom.minimax1]
//! api_key = "xxx"
//! model = "abab6.5s-chat"
//! base_url = "https://api.minimax.chat/v1"
//!
//! [providers.custom.glm2]
//! api_key = "xxx"
//! model = "glm-4-flash"
//! base_url = "https://open.bigmodel.cn/api/paas/v4"
//!
//! [memory]
//! backend = "none"
//! ```

pub mod migration;
pub mod provider;
pub mod schema;
pub mod secrets;

pub use migration::{migrate_str, CURRENT_SCHEMA_VERSION};
pub use provider::{default_base_url, list_providers, resolve_provider, ProviderEntry, ProviderRef, ResolvedProvider};
pub use schema::{AgentSection, Config, MemorySection, ProvidersConfig};
pub use schema::{config_dir, config_path, load_from, load_or_init, save, save_to};
pub use secrets::{is_encrypted, SecretStore};
