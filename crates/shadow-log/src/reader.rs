use crate::LogEvent;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};

use anyhow::Context;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFilter {
    pub since_ts: Option<String>,
    pub cursor_ts: Option<String>,
    pub cursor_id: Option<String>,
    pub cursor_line_offset: Option<u64>,
    pub action: Option<String>,
    pub category: Option<String>,
    pub outcome: Option<String>,
    pub severity_min: Option<u8>,
    pub trace_id: Option<String>,
    pub q: Option<String>,
    pub hide_internal: bool,
    pub field_eq: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPage {
    pub events: Vec<LogEvent>,
    pub next_cursor_line_offset: Option<u64>,
    pub at_end: bool,
}

pub fn load_page(path: &Path, filter: &LogFilter, limit: usize) -> anyhow::Result<LogPage> {
    let limit = limit.clamp(1, 10000);
    if !path.exists() {
        return Ok(LogPage {
            events: Vec::new(),
            next_cursor_line_offset: None,
            at_end: true,
        });
    }

    let file = File::open(path).with_context(|| format!("opening log:{}", path.display()))?;
    let mut reader = BufReader::new(file);

    let mut window = VecDeque::with_capacity(limit + 1);
    let needle = filter.q.as_deref().map(|s| s.to_ascii_lowercase());

    let mut dropped_older = false;
    let mut stopped_early = false;
    let cursor_line_offset = filter.cursor_line_offset;
    let mut next_byte_offset = 0u64;
    let mut buf = String::new();
    loop {
        buf.clear();
        let bytes_read = reader.read_line(&mut buf).context("reading log line")?;
        let line_byte_end = next_byte_offset + bytes_read as u64;
        if bytes_read == 0 {
            break;
        }
        if let Some(cap) = cursor_line_offset
            && line_byte_end >= cap
        {
            stopped_early = true;
            break;
        }
        let trimmed = buf.trim();
        next_byte_offset = line_byte_end;

        if trimmed.is_empty() {
            continue;
        }

        let event: LogEvent = match serde_json::from_str(trimmed) {
            Ok(event) => event,
            Err(err) => {
                tracing::trace!(
                    target: "log",
                    error= ?err,
                    "log: skipping malformed JSONL line"
                );
                continue;
            }
        };

        if !matches_filter(&event, filter, needle.as_deref()) {
            continue;
        }

        window.push_back((event, line_byte_end));
        if window.len() > limit {
            window.pop_front();
            dropped_older = true;
        }
    }

    let oldest_line_offset = window.front().map(|(_, line)| *line);
    let mut events: Vec<LogEvent> = window.into_iter().map(|(e, _)| e).collect();
    events.reverse();

    // let next_cursor = events.last().map(|e| (e.timestamp.clone(), e.id.clone()));
    let at_end = !dropped_older && !stopped_early || events.is_empty();
    Ok(LogPage {
        events,
        next_cursor_line_offset: oldest_line_offset,
        at_end,
    })
}

fn matches_filter(event: &LogEvent, filter: &LogFilter, needle: Option<&str>) -> bool {
    if filter.hide_internal && event.event.category == "internal" {
        return false;
    }
    if let Some(ref since) = filter.since_ts
        && event.timestamp.as_str() < since.as_str()
    {
        return false;
    }
    if let Some(ref cursor) = filter.cursor_ts {
        match event.timestamp.as_str().cmp(cursor.as_str()) {
            Ordering::Less => return false,
            Ordering::Equal => {
                if let Some(ref cursor_id) = filter.cursor_id
                    && event.id.as_str() >= cursor_id.as_str()
                {
                    return false;
                }
            }
            Ordering::Greater => {}
        }
    }

    if let Some(ref action) = filter.action
        && !event.event.action.eq_ignore_ascii_case(action)
    {
        return false;
    }
    if let Some(ref category) = filter.category
        && !event.event.category.eq_ignore_ascii_case(category)
    {
        return false;
    }
    if let Some(ref outcome) = filter.outcome
        && !event.event.outcome.eq_ignore_ascii_case(outcome)
    {
        return false;
    }
    if let Some(min) = filter.severity_min
        && event.severity_number < min
    {
        return false;
    }
    for (key, want) in &filter.field_eq {
        if event.attribution.get(key) != Some(want.as_str()) {
            return false;
        }
    }
    if let Some(ref tid) = filter.trace_id
        && event.trace_id.as_deref() != Some(tid.as_str())
    {
        return false;
    }
    if let Some(n) = needle {
        let hay_msg = event.message.as_deref().unwrap_or("").to_ascii_lowercase();
        let hay_attrs = event.attributes.to_string().to_ascii_lowercase();
        if !hay_msg.contains(n) && !hay_attrs.contains(n) {
            return false;
        }
    }
    true
}

pub fn find_event_by_id(path: &Path, id: &str) -> anyhow::Result<Option<LogEvent>> {
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path).with_context(|| format!("opening log: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut found: Option<LogEvent> = None;
    for line in reader.lines() {
        let line = line.context("reading log line")?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<LogEvent>(trimmed)
            && event.id == id
        {
            found = Some(event); // Don't break — last write wins for duplicate ids.
        }
    }
    Ok(found)
}
#[must_use]
pub fn current_log_path() -> Option<PathBuf> {
    crate::writer::runtime_trace_path()
}
