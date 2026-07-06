//! Anthropic prompt 缓存 -- system_and_3 策略
//!
//! 4 个 cache_control 断点 (Anthropic 最大值):
//! 1. System prompt (所有轮次稳定)
//! 2-4. 最后 3 条非 system 消息 (滚动窗口)
//! 效果: 多轮对话输入 token 成本降低 ~75%

use serde_json::{json, Value};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_gets_marker() {
        let mut msgs = vec![
            json!({"role": "system", "content": "You are helpful"}),
            json!({"role": "user", "content": "Hello"}),
        ];
        apply_cache_control(&mut msgs, "5m");
        assert!(msgs[0].get("content").unwrap().as_array().is_some());
    }

    #[test]
    fn test_last_3_messages_get_markers() {
        let mut msgs = vec![
            json!({"role": "system", "content": "system"}),
            json!({"role": "user", "content": "msg1"}),
            json!({"role": "assistant", "content": "resp1"}),
            json!({"role": "user", "content": "msg2"}),
            json!({"role": "assistant", "content": "resp2"}),
            json!({"role": "user", "content": "msg3"}),
        ];
        apply_cache_control(&mut msgs, "5m");

        // 最后 3 条 (assistant resp2, user msg3, 加上 user msg2) 应有 cache_control
        let last = msgs.last().unwrap();
        let content = last.get("content").unwrap();
        assert!(content.as_array().is_some());
        let arr = content.as_array().unwrap();
        assert!(arr.last().unwrap().get("cache_control").is_some());
    }

    #[test]
    fn test_empty_messages() {
        let mut msgs: Vec<Value> = vec![];
        apply_cache_control(&mut msgs, "5m");
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_1h_ttl() {
        let mut msgs = vec![json!({"role": "system", "content": "sys"})];
        apply_cache_control(&mut msgs, "1h");
        let content = msgs[0].get("content").unwrap().as_array().unwrap();
        let marker = content.last().unwrap().get("cache_control").unwrap();
        assert_eq!(marker.get("ttl").and_then(|t| t.as_str()), Some("1h"));
    }

    #[test]
    fn test_tool_message() {
        let mut msgs = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "q"}),
            json!({"role": "tool", "content": "result"}),
        ];
        apply_cache_control(&mut msgs, "5m");
        // tool 消息应直接在顶层有 cache_control
        assert!(msgs[2].get("cache_control").is_some());
    }
}
