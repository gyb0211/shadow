use crate::config::{LlmRequestPayloadPolicy, ResolvedPolicy, ToolIoPolicy};

#[derive(Debug, Clone)]
pub struct ToolIoCapture {
    pub original_bytes: usize,
    pub truncated: bool,
    pub text: String,
}

impl ToolIoCapture {
    fn empty() -> Self {
        Self {
            text: "".to_string(),
            original_bytes: 0,
            truncated: false,
        }
    }
}

pub fn capture_tool_input(policy: &ResolvedPolicy, tool: &str, redacted: &str) -> Option<ToolIoCapture> {
    capture_with_policy(policy, tool, redacted)
}

pub fn capture_tool_output(policy: &ResolvedPolicy, tool: &str, redacted: &str) -> Option<ToolIoCapture> {
    capture_with_policy(policy, tool, redacted)
}

fn capture_with_policy(policy: &ResolvedPolicy, tool: &str, redacted: &str) -> Option<ToolIoCapture> {
    if !policy.tool_io.captures_io() {
        return None;
    }
    if policy.is_tool_denylisted(tool) {
        return None;
    }
    let original_bytes = redacted.len();
    match policy.tool_io {
        ToolIoPolicy::Off => None,
        ToolIoPolicy::Redacted => Some(truncated_to_cap(redacted, policy.tool_io_truncate_bytes)),
        ToolIoPolicy::Full => Some(ToolIoCapture {
            text: redacted.to_string(),
            original_bytes,
            truncated: false,
        })
    }
}
pub fn capture_llm_request(
    policy: LlmRequestPayloadPolicy,
    truncate_bytes: usize,
    redacted: &str,
) -> Option<ToolIoCapture> {
    match policy {
        LlmRequestPayloadPolicy::Off => None,
        LlmRequestPayloadPolicy::Redacted => Some(truncated_to_cap(redacted, truncate_bytes)),
        LlmRequestPayloadPolicy::Full => Some(ToolIoCapture {
            text: redacted.to_string(),
            original_bytes: redacted.len(),
            truncated: false,
        })
    }
}
fn truncated_to_cap(redacted: &str, cap: usize) -> ToolIoCapture {
    let original_bytes = redacted.len();
    if original_bytes <= cap {
        return ToolIoCapture {
            original_bytes,
            truncated: false,
            text: redacted.to_string(),
        };
    }

    let mut acc = String::with_capacity(cap);
    for ch in redacted.chars() {
        if acc.len() + ch.len_utf8() > cap {
            break;
        }
        acc.push(ch);
    }
    ToolIoCapture {
        text: acc,
        original_bytes,
        truncated: true,
    }
}

// fn empty_unused_marker() {
//     let _ = ToolIoCapture::empty();
// }