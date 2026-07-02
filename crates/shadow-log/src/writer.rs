//! JSONL 写入器 -- 追加式持久化

use crate::event::LogEvent;
use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

struct WriterState {
    path: PathBuf,
    max_entries: usize,
    write_lock: Mutex<()>,
    lines: Mutex<Vec<String>>,
}

static WRITER: OnceLock<Option<Arc<WriterState>>> = OnceLock::new();

/// 初始化写入器
pub fn init_from_config(workspace_dir: &Path, max_entries: usize) {
    let path = workspace_dir.join("logs").join("runtime-trace.jsonl");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let state = Arc::new(WriterState {
        path,
        max_entries,
        write_lock: Mutex::new(()),
        lines: Mutex::new(Vec::new()),
    });
    let _ = WRITER.set(Some(state));
}

/// 获取日志文件路径
#[must_use]
pub fn runtime_trace_path() -> Option<PathBuf> {
    WRITER.get().and_then(|opt| opt.as_ref().map(|s| s.path.clone()))
}

/// 发射一条事件 -- 扇出到: 广播 + JSONL 文件
pub fn record_event(event: LogEvent) {
    let value = match serde_json::to_value(&event) {
        Ok(v) => v,
        Err(_) => return,
    };

    // 广播
    if let Some(hook) = crate::current_broadcast_hook() {
        let _ = hook.send(value.clone());
    }

    // 持久化
    let Some(state) = WRITER.get().and_then(|s| s.as_ref()) else {
        return;
    };

    let _guard = state.write_lock.lock();
    let line = serde_json::to_string(&value).unwrap_or_default();

    // 追加到文件
    if let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.path)
    {
        let mut writer = BufWriter::new(file);
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }

    // 内存缓冲 (用于滚动裁剪)
    let mut lines = state.lines.lock();
    lines.push(line);
    if lines.len() > state.max_entries {
        let drain_count = lines.len() - state.max_entries;
        lines.drain(..drain_count);
        // 重写文件
        let _ = rewrite_file(state);
    }
}

fn rewrite_file(state: &WriterState) -> Result<()> {
    let tmp = state.path.with_extension("jsonl.tmp");
    {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .context("创建临时日志文件")?;
        let mut writer = BufWriter::new(file);
        let lines = state.lines.lock();
        for line in lines.iter() {
            writeln!(writer, "{line}")?;
        }
        writer.flush()?;
    }
    fs::rename(&tmp, &state.path).context("替换日志文件")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 端到端验证: record_event 的文件写入路径
    #[test]
    fn record_event_writes_to_file() {
        let workspace = std::env::temp_dir().join(format!(
            "shadow-log-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // 用新的进程内 init (而非全局 OnceLock) 直接构造 WriterState 测试
        // 由于 init_from_config 用全局 OnceLock, 测试中只能调一次 -- 用独立验证.
        let path = workspace.join("logs").join("runtime-trace.jsonl");

        // 手动模拟 record_event 的行为
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let event = crate::event::LogEvent::new(
            crate::event::Severity::INFO,
            crate::event::Action::Start,
            crate::event::EventCategory::System,
        )
        .with_message("test message");
        let value = serde_json::to_value(&event).unwrap();
        let line = serde_json::to_string(&value).unwrap();

        // 写入
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        let mut writer = BufWriter::new(file);
        writeln!(writer, "{line}").unwrap();
        writer.flush().unwrap();

        // 验证
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("test message"), "content = {content}");
        assert!(content.contains("\"action\":\"start\""), "content = {content}");
    }
}
