//! 上下文压缩 -- 工具输出预清理 + token 估算
//!
//! 不包含 LLM 摘要 (那需要 Provider 注入)
//! 参考 Hermes context_compressor.py 的工具输出预清理部分

use shadow_core::ChatMessage;

const PRUNED_PLACEHOLDER: &str = "[Old tool output cleared to save context space]";
const CHARS_PER_TOKEN: usize = 4;

/// 估算消息列表的 token 数 (粗略: 1 token ≈ 4 字符)
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| {
            let content_len = m.content.chars().count();
            content_len / CHARS_PER_TOKEN
        })
        .sum()
}

/// 判断是否需要压缩
pub fn should_compress(messages: &[ChatMessage], max_tokens: usize) -> bool {
    estimate_tokens(messages) > max_tokens
}

/// 清理旧的工具输出, 保留最近 N 条
///
/// 将 role="tool" 的消息中, 超过 keep_recent 条的旧消息内容替换为占位符
pub fn prune_old_tool_outputs(messages: &mut Vec<ChatMessage>, keep_recent: usize) {
    // 收集 tool 消息的索引 (从后往前)
    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "tool")
        .map(|(i, _)| i)
        .collect();

    // 需要清理的: 除了最后 keep_recent 个之外的所有 tool 消息
    let to_prune: Vec<usize> = if tool_indices.len() > keep_recent {
        tool_indices[..tool_indices.len() - keep_recent].to_vec()
    } else {
        return; // 不需要清理
    };

    for idx in to_prune {
        // 仅当内容不是占位符时才替换 (避免重复清理)
        if messages[idx].content != PRUNED_PLACEHOLDER {
            messages[idx].content = PRUNED_PLACEHOLDER.to_string();
        }
    }
}

/// 清理旧的工具输出 (按 token 预算)
///
/// 不断清理最旧的工具输出, 直到 token 估算低于 max_tokens
pub fn prune_to_fit(messages: &mut Vec<ChatMessage>, max_tokens: usize) -> usize {
    let mut pruned = 0;
    loop {
        if estimate_tokens(messages) <= max_tokens {
            break;
        }

        // 找到最旧的未被清理的 tool 消息
        let oldest = messages
            .iter()
            .enumerate()
            .find(|(_, m)| m.role == "tool" && m.content != PRUNED_PLACEHOLDER)
            .map(|(i, _)| i);

        match oldest {
            Some(idx) => {
                messages[idx].content = PRUNED_PLACEHOLDER.to_string();
                pruned += 1;
            }
            None => break, // 没有更多可清理的
        }
    }
    pruned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn test_estimate_tokens() {
        let msgs = vec![
            make_msg("user", "hello world"),   // 11 chars / 4 ≈ 2
            make_msg("assistant", "hi there"), // 8 chars / 4 ≈ 2
        ];
        let tokens = estimate_tokens(&msgs);
        assert!(tokens > 0 && tokens < 10);
    }

    #[test]
    fn test_should_compress() {
        let msgs = vec![make_msg("user", &"A".repeat(1000))];
        assert!(should_compress(&msgs, 100));
        assert!(!should_compress(&msgs, 10000));
    }

    #[test]
    fn test_prune_old_tool_outputs() {
        let mut msgs = vec![
            make_msg("tool", "old result 1"),
            make_msg("tool", "old result 2"),
            make_msg("tool", "recent result 1"),
            make_msg("tool", "recent result 2"),
        ];

        prune_old_tool_outputs(&mut msgs, 2);

        assert_eq!(msgs[0].content, PRUNED_PLACEHOLDER);
        assert_eq!(msgs[1].content, PRUNED_PLACEHOLDER);
        assert_eq!(msgs[2].content, "recent result 1");
        assert_eq!(msgs[3].content, "recent result 2");
    }

    #[test]
    fn test_prune_keeps_all_if_fewer_than_limit() {
        let mut msgs = vec![make_msg("tool", "result 1"), make_msg("tool", "result 2")];

        prune_old_tool_outputs(&mut msgs, 5);

        assert_eq!(msgs[0].content, "result 1");
        assert_eq!(msgs[1].content, "result 2");
    }

    #[test]
    fn test_prune_to_fit() {
        let mut msgs = vec![
            make_msg("user", &"X".repeat(200)),
            make_msg("tool", &"Y".repeat(200)),
            make_msg("tool", &"Z".repeat(200)),
        ];

        let pruned = prune_to_fit(&mut msgs, 50);
        assert!(pruned > 0);
        assert!(
            estimate_tokens(&msgs) <= 50
                || msgs
                    .iter()
                    .all(|m| m.content == PRUNED_PLACEHOLDER || m.role != "tool")
        );
    }

    #[test]
    fn test_no_prune_when_no_tool_messages() {
        let mut msgs = vec![make_msg("user", "hello"), make_msg("assistant", "hi")];
        let pruned = prune_to_fit(&mut msgs, 1);
        assert_eq!(pruned, 0);
    }
}
