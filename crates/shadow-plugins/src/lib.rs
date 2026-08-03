//! 影子插件 -- WASM 插件系统
//!
//! 对照 ZeroClaw 对应 crate, 尚未实现。
//! 参见 GAP.md 了解差距分析。

// TODO: 实现模块

#[cfg(feature = "plugins-wasmtime")]
pub mod logging;
#[cfg(feature = "plugins-wasmtime")]
pub mod component;
mod error;
mod host;
mod signature;
pub mod runtime;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,

    pub wasm_path: Option<String>,
    pub capabilities: Vec<PluginCapability>,
    pub permissions: Vec<PluginPermission>,
    pub signature: Option<String>,
    pub publisher_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    Tool,
    Channel,
    Memory,
    Observer,
    Skill,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginPermission {
    HttpClient,
    FileRead,
    FileWrite,
    ConfigRead,
    MemoryRead,
    MemoryWrite,
}


#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub descriptions: Option<String>,
    pub capabilities: Vec<PluginCapability>,
    pub permissions: Vec<PluginPermission>,
    pub wasm_path: Option<PathBuf>,
    pub loaded: bool,
}
