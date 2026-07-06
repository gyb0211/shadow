//! 后台技能审查 -- 对话结束后触发, 分析对话决定是否改进技能
//!
//! 参考 Hermes _spawn_background_review + ZeroClaw skills/review.rs
//! 简化版: 对话后异步调用 LLM 分析, 记录建议 (不自动执行)

use shadow_core::{ChatMessage, ChatRequest, ModelProvider};
use std::path::PathBuf;

/// review Agent 的系统提示
const REVIEW_PROMPT: &str = r#"你是 Shadow 的后台技能审查 Agent。分析刚才的对话, 决定是否应该改变技能库。

# 信号 (任一即可行动)
- 用户纠正了你的风格/格式/步骤 → 更新技能
- 发现了新技巧/修复/绕过 → 捕获到技能
- 技能被调用但失败/过时 → 修复技能

# 不要保存
- 环境相关错误 (缺包/路径不对)
- 对工具的负面断言 ("X 不能用")
- 一次性任务

# 优先级
1. PATCH 当前调用的技能
2. CREATE 新的类级技能 (最后手段, 名字要泛化)

如果没什么值得保存的, 回复 "无需改进"。"#;

/// 决定是否触发 review, 并运行
///
/// 触发条件:
/// 1. 对话积累了足够的工具调用 (>= nudge_threshold)
/// 2. provider 可用
///
/// 简化版: 只记录 LLM 的分析建议, 不自动执行修改
/// (完整版需要 review fork 有自己的工具循环, 后续实现)
pub async fn maybe_run_skill_review(
    _workspace_dir: PathBuf,
    history: &[ChatMessage],
    nudge_threshold: usize,
    provider: &dyn ModelProvider,
    model: &str,
) -> anyhow::Result<()> {
    // 1. 检查工具调用次数是否达到阈值
    let tool_call_count = count_tool_results(history);
    if tool_call_count < nudge_threshold {
        return Ok(());
    }

    shadow_log::record!(
        INFO,
        shadow_log::Action::Note,
        format!("技能审查触发: {tool_call_count} 次工具调用 >= 阈值 {nudge_threshold}")
    );

    // 2. 构建 review 请求
    let recent = format_recent_history(history, 20);
    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: REVIEW_PROMPT.to_string(),
            ..Default::default()
        },
        ChatMessage {
            role: "user".into(),
            content: format!("分析以下对话, 决定是否需要改进技能:\n\n{recent}"),
            ..Default::default()
        },
    ];

    let request = ChatRequest {
        messages,
        model: model.to_string(),
        temperature: Some(0.3),
        max_tokens: Some(2000),
        tools: vec![],
    };

    // 3. 调用 LLM 分析
    match provider.chat(request).await {
        Ok(response) => {
            let preview: String = response.content.chars().take(500).collect();
            shadow_log::record!(
                INFO,
                shadow_log::Action::Note,
                format!("技能审查结果: {preview}")
            );
        }
        Err(e) => {
            shadow_log::record!(WARN, shadow_log::Action::Fail, format!("技能审查失败: {e}"));
        }
    }

    Ok(())
}

/// 统计历史中的 tool 消息数
fn count_tool_results(history: &[ChatMessage]) -> usize {
    history.iter().filter(|m| m.role == "tool").count()
}

/// 格式化最近的历史 (截断每条消息到 500 字符)
fn format_recent_history(history: &[ChatMessage], max: usize) -> String {
    let start = history.len().saturating_sub(max);
    let recent = &history[start..];
    let mut out = String::new();
    for msg in recent {
        let content: String = msg.content.chars().take(500).collect();
        out.push_str(&format!("[{}] {}\n\n", msg.role, content));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_tool_results_empty() {
        assert_eq!(count_tool_results(&[]), 0);
    }

    #[test]
    fn count_tool_results_filters() {
        let history = vec![
            ChatMessage {
                role: "user".into(),
                content: "hi".into(),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".into(),
                content: "result".into(),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".into(),
                content: "ok".into(),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".into(),
                content: "result2".into(),
                ..Default::default()
            },
        ];
        assert_eq!(count_tool_results(&history), 2);
    }

    #[test]
    fn format_recent_history_truncates() {
        let long = "x".repeat(1000);
        let history = vec![ChatMessage {
            role: "user".into(),
            content: long,
            ..Default::default()
        }];
        let formatted = format_recent_history(&history, 10);
        assert!(formatted.len() < 600); // 500 + 一些格式字符
    }

    #[test]
    fn format_recent_history_limits_count() {
        let history: Vec<_> = (0..30)
            .map(|i| ChatMessage {
                role: "user".into(),
                content: format!("msg{i}"),
                ..Default::default()
            })
            .collect();
        let formatted = format_recent_history(&history, 5);
        assert!(formatted.contains("msg25"));
        assert!(formatted.contains("msg29"));
        assert!(!formatted.contains("msg24")); // 被截断
    }
}
