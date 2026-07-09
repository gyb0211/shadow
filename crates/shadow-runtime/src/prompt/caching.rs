//! Anthropic prompt 缓存 -- system_and_3 策略
//!
//! 4 个 cache_control 断点 (Anthropic 最大值):
//! 1. System prompt (所有轮次稳定)
//! 2-4. 最后 3 条非 system 消息 (滚动窗口)
//! 效果: 多轮对话输入 token 成本降低 ~75%

use serde_json::{Value, json};

/// 应用 system_and_3 缓存策略
///
/// 在 system prompt 和最后 3 条非 system 消息上添加 cache_control 断点
pub fn apply_cache_control(messages: &mut Vec<Value>, ttl: &str) {
    if messages.is_empty() {
        return;
    }

    let marker = if ttl == "1h" {
        json!({"type": "ephemeral", "ttl": "1h"})
    } else {
        json!({"type": "ephemeral"})
    };

    // 1. System prompt 断点
    if let Some(first) = messages.first_mut() {
        if first.get("role").and_then(|r| r.as_str()) == Some("system") {
            add_cache_marker(first, &marker);
        }
    }

    // 2-4. 最后 3 条非 system 消息
    let non_system_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) != Some("system"))
        .map(|(i, _)| i)
        .collect();

    let last_3: Vec<usize> = non_system_indices.iter().rev().take(3).copied().collect();
    for idx in last_3 {
        add_cache_marker(&mut messages[idx], &marker);
    }
}

/// 给单条消息添加 cache_control
fn add_cache_marker(msg: &mut Value, marker: &Value) {
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

    // tool 消息: 直接加在顶层
    if role == "tool" {
        if let Some(obj) = msg.as_object_mut() {
            obj.insert("cache_control".to_string(), marker.clone());
        }
        return;
    }

    let content = msg.get("content");
    if content.is_none() || content == Some(&Value::Null) {
        if let Some(obj) = msg.as_object_mut() {
            obj.insert("cache_control".to_string(), marker.clone());
        }
        return;
    }

    // string content -> 转为 array + cache_control
    if let Some(text) = content.and_then(|c| c.as_str()) {
        msg["content"] = json!([{
            "type": "text",
            "text": text,
            "cache_control": marker
        }]);
        return;
    }

    // array content: 在最后一个元素加 cache_control
    if let Some(arr) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
        if let Some(last) = arr.last_mut() {
            if let Some(obj) = last.as_object_mut() {
                obj.insert("cache_control".to_string(), marker.clone());
            }
        }
    }
}

