//! MessageList widget -- 渲染消息流
//!
//! 角色配色:
//!   user      ❯ 蓝 (USER)
//!   assistant ❯ 绿 (ASSISTANT)
//!   tool      ❯ 灰框线 (TOOL_DIM / TOOL_TEXT)

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;
use shadow_core::ChatMessage;

pub struct MessageList<'a> {
    pub messages: &'a [ChatMessage],
}

impl<'a> MessageList<'a> {
    pub fn new(messages: &'a [ChatMessage]) -> Self {
        Self { messages }
    }
}

impl<'a> Widget for MessageList<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 清空区域
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ').set_style(Style::default().bg(theme::BG));
                }
            }
        }

        let mut y = area.top();
        for msg in self.messages {
            if y >= area.bottom() {
                break;
            }
            let (label, color) = match msg.role.as_str() {
                "user" => ("user ❯ ", theme::USER),
                "assistant" => ("assistant ❯ ", theme::ASSISTANT),
                "tool" => ("tool ❯ ", theme::TOOL_TEXT),
                _ => (msg.role.as_str(), theme::DIM),
            };

            // 标签 + 内容
            let label_style = Style::default().fg(color).bg(theme::BG);
            let content_style = Style::default().fg(theme::TEXT).bg(theme::BG);
            let line = Line::from(vec![
                Span::styled(label.to_string(), label_style),
                Span::styled(msg.content.clone(), content_style),
            ]);
            let _ = buf.set_line(area.left(), y, &line, area.width);
            y += 1;

            // tool_calls 框线 (assistant 消息附带)
            for tc in &msg.tool_calls {
                if y >= area.bottom() {
                    break;
                }
                let tc_line = Line::from(vec![Span::styled(
                    format!("  ┌─ {}", tc.name),
                    Style::default().fg(theme::TOOL_DIM),
                )]);
                let _ = buf.set_line(area.left(), y, &tc_line, area.width);
                y += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn render_to_buffer(widget: MessageList<'_>, w: u16, h: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        widget.render(Rect::new(0, 0, w, h), &mut buf);
        buf
    }

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    #[test]
    fn user_label_is_blue() {
        let messages = vec![msg("user", "hi")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
        // 标签第一个字符 'u' 应该是 USER 蓝色
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.fg, theme::USER);
    }

    #[test]
    fn assistant_label_is_green() {
        let messages = vec![msg("assistant", "hello")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.fg, theme::ASSISTANT);
    }

    #[test]
    fn tool_label_is_dim() {
        let messages = vec![msg("tool", "/tmp/foo")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.fg, theme::TOOL_TEXT);
    }
}
