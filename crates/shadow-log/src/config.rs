use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct LogConfig {
    pub persistence: String,
    pub persistence_path: String,
    pub persistence_max_entries: usize,
    pub persistence_max_bytes: u64,
    pub persistence_rotate_daily: bool,
    pub persistence_retention_max_files: usize,
    pub persistence_retention_max_age_days: u64,
    pub tool_io: String,
    pub tool_io_truncate_bytes: usize,
    pub tool_io_denylist: Vec<String>,
    pub llm_request_payload: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self{
            persistence: "rolling".to_string(),
            persistence_path: String::new(),
            persistence_max_entries: 10_000,
            persistence_max_bytes: 0,
            persistence_rotate_daily: true,
            persistence_retention_max_files: 7,
            persistence_retention_max_age_days: 0,
            tool_io: "redacted".to_string(),
            tool_io_truncate_bytes: 40960,
            tool_io_denylist: Vec::new(),
            llm_request_payload: "off".to_string(),
        }

    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoragePolicy {
    None,
    Rolling,
    Full,
    Rotating,
}

impl StoragePolicy {
    pub fn from_raw(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "rolling" => Self::Rolling,
            "full" => Self::Full,
            "rotate" => Self::Rotating,
            _ => Self::None,
        }
    }
    
    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolIoPolicy {
    Off,
    Redacted,
    Full,
}

impl ToolIoPolicy {
    pub fn from_raw(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Self::Off,
            "full" => Self::Full,
            _ => Self::Redacted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmRequestPayloadPolicy {
    Off,
    Redacted,
    Full,
}

impl LlmRequestPayloadPolicy {
    pub fn from_raw(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "redacted" => Self::Redacted,
            "full" => Self::Full,
            _ => Self::Off,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedPolicy {
    pub storage: StoragePolicy,
    pub path: PathBuf,
    pub max_entries: usize,
    pub max_bytes: u64,
    pub rotate_daily: bool,
    pub retention_max_files: usize,
    pub retention_max_age_days: u64,
    pub tool_io: ToolIoPolicy,
    pub tool_io_truncate_bytes: usize,
    pub tool_id_denylist: Vec<String>,
    pub llm_request_payload: LlmRequestPayloadPolicy,
}

impl ResolvedPolicy {
    pub fn from_config(config: &LogConfig, workspace_dir: &Path) -> Self {
        Self {
            storage: StoragePolicy::from_raw(&config.persistence),
            path: resolve_path(&config.persistence_path, workspace_dir),
            max_entries: config.persistence_max_entries.max(1),
            max_bytes: config.persistence_max_bytes,
            rotate_daily: config.persistence_rotate_daily,
            retention_max_files: config.persistence_retention_max_files,
            retention_max_age_days: config.persistence_retention_max_age_days,
            tool_io: ToolIoPolicy::from_raw(&config.tool_io),
            tool_io_truncate_bytes: config.tool_io_truncate_bytes,
            tool_id_denylist: config.tool_io_denylist.clone(),
            llm_request_payload: LlmRequestPayloadPolicy::from_raw(&config.llm_request_payload),
        }
    }
}

const DEFAULT_LOG_REL_PATH: &str = "state/runtime-trace.jsonl";

fn resolve_path(path: &str, workspace: &Path) -> PathBuf {
    let path = path.trim();
    if path.is_empty() {
        return workspace.join(DEFAULT_LOG_REL_PATH);
    }

    let pf = PathBuf::from(path);
    if pf.is_absolute() {
        pf
    } else {
        workspace.join(pf)
    }
}
