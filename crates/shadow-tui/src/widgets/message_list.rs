//! MessageList widget -- 渲染消息流
//!
//! 用 ratatui Paragraph + Wrap 做软换行 (长行自动折到下一行, 不截断).
//! 自动滚到底部 (最新消息可见).
//!
//! 角色配色:
//!   user      ❯ 蓝 (USER)
//!   assistant ❯ 绿 (ASSISTANT)
//!   tool      ❯ 灰 (TOOL_TEXT)

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::theme;
use shadow_core::ChatMessage;

pub struct MessageList<'a> {
    pub messages: &'a [ChatMessage],
    /// 滚动偏移 (从底部向上算). 0 = 跟随最新消息.
    pub scroll_from_bottom: usize,
}

impl<'a> MessageList<'a> {
    pub fn new(messages: &'a [ChatMessage]) -> Self {
        Self { messages, scroll_from_bottom: 0 }
    }

    pub fn scroll(mut self, scroll_from_bottom: usize) -> Self {
        self.scroll_from_bottom = scroll_from_bottom;
        self
    }
}

impl<'a> Widget for MessageList<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line<'static>> = Vec::new();

        for (idx, msg) in self.messages.iter().enumerate() {
            // 消息间距 (除第一条)
            if idx > 0 {
                lines.push(Line::from(""));
            }

            let (label, color) = match msg.role.as_str() {
                "user" => ("user ❯ ".to_string(), theme::USER),
                "assistant" => ("assistant ❯ ".to_string(), theme::ASSISTANT),
                "tool" => ("tool ❯ ".to_string(), theme::TOOL_TEXT),
                _ => (msg.role.clone(), theme::DIM),
            };

            let label_style = Style::default().fg(color).bg(theme::BG);
            let content_style = Style::default().fg(theme::TEXT).bg(theme::BG);
            let dim_style = Style::default().fg(theme::DIM).bg(theme::BG);

            // 按 \n 拆行; 第一行带标签
            let mut content_lines = msg.content.lines();
            match content_lines.next() {
                Some(first) => {
                    lines.push(Line::from(vec![
                        Span::styled(label, label_style),
                        Span::styled(first.to_string(), content_style),
                    ]));
                }
                None => {
                    lines.push(Line::from(vec![Span::styled(label, label_style)]));
                }
            }
            for text in content_lines {
                lines.push(Line::from(vec![Span::styled(text.to_string(), content_style)]));
            }

            // tool_calls: 名称 + 参数预览
            for tc in &msg.tool_calls {
                let args_preview = serde_json::to_string(&tc.arguments)
                    .unwrap_or_default();
                let preview = if args_preview.len() > 80 {
                    format!("{}...", &args_preview[..80])
                } else {
                    args_preview
                };
                lines.push(Line::from(vec![
                    Span::styled("  ┌─ ", dim_style),
                    Span::styled(tc.name.clone(), Style::default().fg(theme::TOOL_DIM).bg(theme::BG)),
                    Span::styled(format!("  {}", preview), dim_style),
                ]));
            }
        }

        // 计算实际显示行数 (考虑 Wrap 软换行)
        let width = area.width as usize;
        let total_display: usize = lines
            .iter()
            .map(|l| {
                let w = l.width();
                if w == 0 || width == 0 {
                    1
                } else {
                    ((w + width - 1) / width).max(1)
                }
            })
            .sum();

        // 自动滚到底部 (最新消息可见); scroll_from_bottom > 0 时向上翻
        let visible = area.height as usize;
        let bottom_offset = total_display.saturating_sub(visible);
        let scroll = bottom_offset.saturating_sub(self.scroll_from_bottom);

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0))
            .style(Style::default().bg(theme::BG))
            .render(area, buf);

        // 滚动指示条 (右侧, scroll_from_bottom > 0 时显示)
        if self.scroll_from_bottom > 0 && total_display > visible && area.width > 0 {
            let ratio = scroll as f64 / bottom_offset as f64;
            let thumb_y = area.top() + (ratio * visible as f64) as u16;
            let thumb_y = thumb_y.min(area.bottom() - 1);
            if let Some(cell) = buf.cell_mut((area.right() - 1, thumb_y)) {
                cell.set_char('┃');
                cell.set_fg(theme::DIM);
                cell.set_bg(theme::BG);
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
            ..Default::default()
        }
    }

    #[test]
    fn user_label_is_blue() {
        let messages = vec![msg("user", "hi")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
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
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme::ASSISTANT);
        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "第");
        assert_eq!(buf.cell((0, 2)).unwrap().symbol(), "第");
    }

    #[test]
    fn empty_content_still_renders_label() {
        let messages = vec![msg("user", "")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme::USER);
    }

    #[test]
    fn long_line_soft_wraps_not_truncated() {
        // 60 字符宽区域, 100 字符内容 -- 软换行后第 2 行应有内容 (非空格)
        let long = "a".repeat(100);
        let messages = vec![msg("user", &long)];
        let buf = render_to_buffer(MessageList::new(&messages), 60, 3);
        // 第 2 行不应是空白 (被 Wrap 到了下一行)
        let row1_content: String = (0..60)
            .map(|x| buf.cell((x, 1)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect();
        assert!(row1_content.contains('a'), "第 2 行应包含软换行内容");
    }

    #[test]
    fn auto_scroll_shows_latest_message() {
        // 3 行高度, 5 条消息 -- 最后一条应在可见区域
        let messages = vec![
            msg("user", "1"),
            msg("assistant", "2"),
            msg("user", "3"),
            msg("assistant", "4"),
            msg("user", "latest"),
        ];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 3);
        // 底部应能看到 "latest" 的某个字符
        let bottom_row: String = (0..40)
            .map(|x| buf.cell((x, 2)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect();
        assert!(bottom_row.contains("latest"), "底部行应显示最新消息");
    }

    #[test]
    fn gap_between_messages() {
        let messages = vec![msg("user", "a"), msg("assistant", "b")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 5);
        // 第 0 行: user, 第 1 行: 空行间距, 第 2 行: assistant
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme::USER);
        assert_eq!(buf.cell((0, 2)).unwrap().fg, theme::ASSISTANT);
    }

    #[test]
    fn scroll_indicator_shown_when_scrolled_up() {
        // 10 条消息, 3 行可见, scroll_from_bottom=3 → 右侧应出现 ┃
        let messages: Vec<ChatMessage> = (0..10)
            .map(|i| msg("user", &format!("msg {i}")))
            .collect();
        let widget = MessageList::new(&messages).scroll(3);
        let buf = render_to_buffer(widget, 40, 3);
        // 右侧某行应有 ┃
        let right_col: String = (0..3)
            .map(|y| buf.cell((39, y)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect();
        assert!(right_col.contains('┃'), "滚动时应显示指示条, got: {right_col:?}");
    }

    #[test]
    fn scroll_indicator_absent_when_at_bottom() {
        let messages: Vec<ChatMessage> = (0..10)
            .map(|i| msg("user", &format!("msg {i}")))
            .collect();
        let widget = MessageList::new(&messages); // scroll_from_bottom=0
        let buf = render_to_buffer(widget, 40, 3);
        let right_col: String = (0..3)
            .map(|y| buf.cell((39, y)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect();
        assert!(!right_col.contains('┃'), "在底部时不应显示指示条");
    }

    #[test]
    fn wrapped_long_message_auto_scrolls_correctly() {
        // 100 字符内容在 20 宽区域 = 5 显示行; 3 行可见 → 需要滚动 2 行才能看到末尾
        let long = "a".repeat(100);
        let messages = vec![msg("assistant", &long)];
        let buf = render_to_buffer(MessageList::new(&messages), 20, 3);
        // 底部行应能看到 'a' (说明滚动到了内容末尾)
        let bottom_char = buf.cell((0, 2)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(bottom_char, "a", "长消息末尾应通过自动滚动可见");
    }
}
