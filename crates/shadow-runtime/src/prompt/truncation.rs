//! 工具输出截断 -- 头尾保留 + JSON 感知
//!
//! 参考 ZeroClaw 的 history.rs 实现

/// 截断工具输出: 保留头尾, 中间用占位符替代
///
/// 如果 output 长度 <= max_chars, 原样返回
/// 否则保留头部和尾部各约 max_chars/3, 中间用 "[... N characters truncated ...]" 替代
pub fn truncate_tool_result(output: &str, max_chars: usize) -> String {
    if output.len() <= max_chars {
        return output.to_string();
    }

    // 保留头尾各 1/3
    let head_size = max_chars / 3;
    let tail_size = max_chars / 3;
    let truncated_chars = output.len() - head_size - tail_size;

    // 找到不截断 UTF-8 字符的边界
    let head_end = find_char_boundary(output, head_size);
    let tail_start = find_char_boundary_from_end(output, output.len() - tail_size);

    format!(
        "{}\n\n[... {} characters truncated ...]\n\n{}",
        &output[..head_end],
        truncated_chars,
        &output[tail_start..]
    )
}

/// JSON 感知的工具消息截断
///
/// 如果消息是 {"content": "..."} 格式, 截断内部 content 字段
/// 否则直接截断
pub fn truncate_tool_message(msg_content: &str, max_chars: usize) -> String {
    // 尝试解析为 JSON
    if let Ok(mut obj) = serde_json::from_str::<serde_json::Value>(msg_content) {
        if let Some(inner) = obj.get("content").and_then(|c| c.as_str()) {
            let truncated = truncate_tool_result(inner, max_chars);
            if let Some(obj_mut) = obj.as_object_mut() {
                obj_mut.insert("content".to_string(), serde_json::Value::String(truncated));
            }
            return serde_json::to_string(&obj).unwrap_or_else(|_| msg_content.to_string());
        }
    }

    // 非 JSON, 直接截断
    truncate_tool_result(msg_content, max_chars)
}

/// 找到从前往后不超过 idx 的 UTF-8 字符边界
fn find_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 找到从后往前不超过 idx 的 UTF-8 字符边界
fn find_char_boundary_from_end(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

