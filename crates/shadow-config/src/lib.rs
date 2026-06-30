//! 影子配置层 -- TOML schema + 加载 + 密钥管理
//!
//! 借鉴 ZeroClaw 的 Config 设计, 但大幅精简:
//! - ZeroClaw: 61,918 行, 36 文件, ChaCha20-Poly1305 加密
//! - Shadow: 目标 ~200 行, 单文件, 明文 TOML

pub mod schema;

pub use schema::{Config, AgentSection, ProviderSection, MemorySection};
pub use schema::{config_dir, config_path, load_or_init, save};
