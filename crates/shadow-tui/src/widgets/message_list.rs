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

            let label_style = Style::default().fg(color).bg(theme::BG);
            let content_style = Style::default().fg(theme::TEXT).bg(theme::BG);

            // 按 content 内的实际换行拆行: 第一行带标签, 后续行无标签
            for (i, text) in msg.content.lines().enumerate() {
                if y >= area.bottom() {
                    break;
                }
                let line = if i == 0 {
                    Line::from(vec![
                        Span::styled(label, label_style),
                        Span::styled(text, content_style),
                    ])
                } else {
                    Line::from(vec![Span::styled(text, content_style)])
                };
                let _ = buf.set_line(area.left(), y, &line, area.width);
                y += 1;
            }

            // content 为空时至少渲染标签行
            if msg.content.is_empty() && y < area.bottom() {
                let line = Line::from(vec![Span::styled(label, label_style)]);
                let _ = buf.set_line(area.left(), y, &line, area.width);
                y += 1;
            }

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
            tool_calls: vec![], reasoning_content: None,
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

    #[test]
    fn multiline_content_spans_multiple_rows() {
        let messages = vec![msg("assistant", "第一行\n第二行\n第三行")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 5);
        // 第一行: 标签 "assistant ❯ " + "第一行"
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme::ASSISTANT);
        // 第二行: 无标签, 纯 content "第二行"
        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "第");
        // 第三行: "第三行"
        assert_eq!(buf.cell((0, 2)).unwrap().symbol(), "第");
    }

    #[test]
    fn empty_content_still_renders_label() {
        let messages = vec![msg("user", "")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
        // 标签行应存在
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme::USER);
    }
}
