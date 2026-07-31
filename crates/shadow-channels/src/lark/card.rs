pub(super) const LARK_CARD_MARKDOWN_MAX_BYTES: usize = 28_000;

/// Build the full message body for sending an interactive card message.
pub(super) fn build_interactive_card_body(recipient: &str, markdown: &str) -> serde_json::Value {
    serde_json::json!({
        "receive_id": recipient,
        "msg_type": "interactive",
        "content": build_card_content(markdown),
    })
}

fn build_card_content(markdown: &str) -> String {
    serde_json::json!({
        "schema": "2.0",
        "body": {
            "elements": [{
                "tag": "markdown",
                "content": markdown
            }]
        }
    })
    .to_string()
}

/// Split markdown content into chunks that fit within the card size limit.
/// Splits on line boundaries to avoid breaking markdown syntax.
pub(super) fn split_markdown_chunks(text: &str, max_bytes: usize) -> Vec<&str> {
    if text.len() <= max_bytes {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        if start + max_bytes >= text.len() {
            chunks.push(&text[start..]);
            break;
        }

        let end = start + max_bytes;
        let search_region = &text[start..end];
        let split_at = search_region
            .rfind('\n')
            .map(|pos| start + pos + 1)
            .unwrap_or(end);

        let split_at = if text.is_char_boundary(split_at) {
            split_at
        } else {
            (start..split_at)
                .rev()
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(start)
        };

        if split_at <= start {
            let forced = (end..=text.len())
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(text.len());
            chunks.push(&text[start..forced]);
            start = forced;
        } else {
            chunks.push(&text[start..split_at]);
            start = split_at;
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_short_text_returns_single_chunk() {
        let chunks = split_markdown_chunks("hello world", 100);
        assert_eq!(chunks, vec!["hello world"]);
    }

    #[test]
    fn split_empty_text() {
        let chunks = split_markdown_chunks("", 100);
        assert_eq!(chunks, vec![""]);
    }

    #[test]
    fn split_exact_max_bytes() {
        let chunks = split_markdown_chunks("abcd", 4);
        assert_eq!(chunks, vec!["abcd"]);
    }

    #[test]
    fn split_on_newline_boundary() {
        // 5 bytes per line, max 10 → splits at newlines
        let text = "aaaaa\nbbbbb\nccccc";
        let chunks = split_markdown_chunks(text, 10);
        assert_eq!(chunks, vec!["aaaaa\n", "bbbbb\n", "ccccc"]);
    }

    #[test]
    fn split_no_newlines_forces_split() {
        let text = "abcdefghij"; // 10 bytes
        let chunks = split_markdown_chunks(text, 4);
        assert_eq!(chunks.concat(), text);
        assert_eq!(chunks, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn split_multibyte_utf8_aligned() {
        // Each Chinese char = 3 bytes; max_bytes = 6 = 2 chars
        let text = "你好世界测试 Rust";
        let chunks = split_markdown_chunks(text, 6);
        assert_eq!(chunks.concat(), text);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn build_card_body_structure() {
        let body = build_interactive_card_body("chat_123", "# Title");
        assert_eq!(body["receive_id"], "chat_123");
        assert_eq!(body["msg_type"], "interactive");

        let content: serde_json::Value =
            serde_json::from_str(body["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["schema"], "2.0");
        assert_eq!(content["body"]["elements"][0]["tag"], "markdown");
        assert_eq!(content["body"]["elements"][0]["content"], "# Title");
    }
}
