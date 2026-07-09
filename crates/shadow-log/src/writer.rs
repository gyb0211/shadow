//! JSONL 写入器 -- 追加式持久化 + 流式裁剪
//!
//! 参考 ZeroClaw writer.rs:
//! - 不在内存缓冲所有行 (避免 OOM)
//! - 超限时流式重写: BufReader 逐行读 → 保留最新 N 行 → 写临时文件 → rename
//! - RAM 使用量恒定: 一行读缓冲 + 一行写缓冲

use crate::event::LogEvent;
use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

struct WriterState {
    path: PathBuf,
    max_entries: usize,
    write_lock: Mutex<()>,
    /// 当前文件行数估算 (追加时 +1, 裁剪后重置)
    /// 仅用于触发裁剪, 不作为精确计数
    line_count: Mutex<usize>,
}

static WRITER: OnceLock<parking_lot::RwLock<Option<Arc<WriterState>>>> = OnceLock::new();

fn slot() -> &'static parking_lot::RwLock<Option<Arc<WriterState>>> {
    WRITER.get_or_init(|| parking_lot::RwLock::new(None))
}

fn current_state() -> Option<Arc<WriterState>> {
    slot().read().clone()
}

/// 初始化写入器
pub fn init_from_config(workspace_dir: &Path, max_entries: usize) {
    let path = workspace_dir.join("logs").join("runtime-trace.jsonl");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // 统计现有文件行数 (用于初始 line_count)
    let initial_count = count_lines(&path).unwrap_or(0);

    let state = Arc::new(WriterState {
        path,
        max_entries,
        write_lock: Mutex::new(()),
        line_count: Mutex::new(initial_count),
    });
    *slot().write() = Some(state);
}

/// 获取日志文件路径
#[must_use]
pub fn runtime_trace_path() -> Option<PathBuf> {
    current_state().map(|s| s.path.clone())
}

/// 发射一条事件 -- 扇出到: 广播 + JSONL 文件 + observer 桥接
pub fn record_event(event: LogEvent) {
    let value = match serde_json::to_value(&event) {
        Ok(v) => v,
        Err(_) => return,
    };

    // 广播 (SSE / 实时订阅)
    if let Some(hook) = crate::current_broadcast_hook() {
        let _ = hook.send(value.clone());
    }

    // observer 桥接 (投影到 ObserverEvent)
    crate::observer_bridge::forward(&event);

    // 持久化到 JSONL 文件
    let Some(state) = current_state() else {
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

    // 行数 +1, 超限时流式裁剪
    let mut count = state.line_count.lock();
    *count += 1;
    if *count > state.max_entries {
        let _ = trim_file_streaming(&state.path, state.max_entries);
        *count = state.max_entries;
    }
}

/// 流式裁剪: 保留文件最后 N 行, 删除其余
///
/// 参考 ZeroClaw writer.rs 的 rolling trim:
/// 1. BufReader 逐行读原文件, 保留最后 N 行到 VecDeque (RAM 有界)
/// 2. 写临时文件
/// 3. rename 替换原文件
///
/// 与旧版 rewrite_file 的区别: 不在内存缓冲所有行, 只保留 N 行
fn trim_file_streaming(path: &Path, keep: usize) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let file = File::open(path).context("打开日志文件进行裁剪")?;
    let reader = BufReader::new(file);

    // 环形缓冲: 只保留最后 keep 行 (RAM 有界)
    use std::collections::VecDeque;
    let mut tail: VecDeque<String> = VecDeque::with_capacity(keep);
    for line in reader.lines() {
        let line = line.context("读取日志行")?;
        if tail.len() == keep {
            tail.pop_front();
        }
        tail.push_back(line);
    }

    // 写临时文件
    let tmp = path.with_extension("jsonl.tmp");
    {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .context("创建临时日志文件")?;
        let mut writer = BufWriter::new(file);
        for line in &tail {
            writeln!(writer, "{line}")?;
        }
        writer.flush()?;
    }

    fs::rename(&tmp, path).context("替换日志文件")?;
    Ok(())
}

/// 统计文件行数 (用于初始化 line_count)
fn count_lines(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    Ok(reader.lines().count())
}

// ── 日志读取器 -- 分页查询 ──────────────────────────────

/// 日志过滤参数
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// 时间下界 (RFC 3339, 包含)
    pub since_ts: Option<String>,
    /// 时间上界 (RFC 3339, 不包含)
    pub until_ts: Option<String>,
    /// 匹配 action
    pub action: Option<String>,
    /// 匹配 category
    pub category: Option<String>,
    /// 匹配 outcome
    pub outcome: Option<String>,
    /// 最低 severity_number
    pub severity_min: Option<u8>,
    /// 归因字段精确匹配 (key → value)
    pub field_eq: std::collections::BTreeMap<String, String>,
    /// 全文搜索 (message + attribution)
    pub q: Option<String>,
}

/// 一页查询结果
#[derive(Debug, Clone)]
pub struct LogPage {
    pub events: Vec<LogEvent>,
    /// 是否还有更旧的日志
    pub has_more: bool,
}

/// 从日志文件加载一页事件 (从尾部向前扫描)
///
/// 参考 ZeroClaw reader.rs:
/// - 从 EOF 向后扫描, 逐行解码
/// - 应用过滤条件, 收集 limit 条匹配
/// - RAM 有界: 最多 limit 条事件 + 一行读缓冲
pub fn load_page(filter: &LogFilter, limit: usize) -> Result<LogPage> {
    let Some(path) = runtime_trace_path() else {
        return Ok(LogPage { events: vec![], has_more: false });
    };

    if !path.exists() {
        return Ok(LogPage { events: vec![], has_more: false });
    }

    // 读取所有行 (简化版: 全量读取后过滤)
    // TODO: 后续可优化为从尾部逐行扫描
    let file = File::open(&path)?;
    let reader = BufReader::new(file);
    let all_lines: Vec<String> = reader.lines().collect::<std::io::Result<_>>()?;

    // 过滤 + 收集匹配的事件
    let mut matched: Vec<LogEvent> = Vec::new();

    for line in &all_lines {
        let Ok(event) = serde_json::from_str::<LogEvent>(line) else {
            continue;
        };

        if !matches_filter(&event, filter) {
            continue;
        }

        matched.push(event);
    }

    // 从尾部取 limit 条
    let total = matched.len();
    let start = total.saturating_sub(limit);
    let events = matched[start..].to_vec();
    let has_more = start > 0;

    Ok(LogPage { events, has_more })
}

/// 检查事件是否匹配过滤条件
fn matches_filter(event: &LogEvent, filter: &LogFilter) -> bool {
    // action
    if let Some(ref action) = filter.action
        && !event.action.eq_ignore_ascii_case(action) {
            return false;
        }
    // category
    if let Some(ref category) = filter.category
        && !event.category.eq_ignore_ascii_case(category) {
            return false;
        }
    // outcome
    if let Some(ref outcome) = filter.outcome
        && event.outcome.as_deref().unwrap_or("unknown") != outcome.as_str() {
            return false;
        }
    // severity_min
    if let Some(min) = filter.severity_min
        && event.severity_number < min {
            return false;
        }
    // 时间范围
    if let Some(ref since) = filter.since_ts
        && event.timestamp.as_str() < since.as_str() {
            return false;
        }
    if let Some(ref until) = filter.until_ts
        && event.timestamp.as_str() >= until.as_str() {
            return false;
        }
    // 归因字段
    for (key, value) in &filter.field_eq {
        if event.attribution.get(key).map(|s| s.as_str()) != Some(value.as_str()) {
            return false;
        }
    }
    // 全文搜索
    if let Some(ref q) = filter.q {
        let q_lower = q.to_lowercase();
        let msg_match = event
            .message
            .as_ref()
            .map(|m| m.to_lowercase().contains(&q_lower))
            .unwrap_or(false);
        let attr_match = event
            .attribution
            .values()
            .any(|v| v.to_lowercase().contains(&q_lower));
        if !msg_match && !attr_match {
            return false;
        }
    }

    true
}

