//! InputBox widget -- 输入框 (单行渲染, 多行存储)
//!
//! 行为由 AppState.chat.input 持有, 这里只渲染当前行 + 光标

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;

pub struct InputBox<'a> {
    pub text: &'a str,
    pub cursor: usize,
}

impl<'a> InputBox<'a> {
    pub fn new(text: &'a str, cursor: usize) -> Self {
        Self { text, cursor }
    }
}

impl<'a> Widget for InputBox<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let prompt = Span::styled("❯ ", Style::default().fg(theme::USER));
        let cursor_span = Span::styled("_", Style::default().fg(theme::ACCENT));

        let mut spans = vec![prompt];
        let chars: Vec<char> = self.text.chars().collect();
        let before: String = chars[..self.cursor.min(chars.len())].iter().collect();
        let after: String = chars[self.cursor.min(chars.len())..].iter().collect();

        if !before.is_empty() {
            spans.push(Span::styled(before, Style::default().fg(theme::TEXT)));
        }
        spans.push(cursor_span);
        if !after.is_empty() {
            spans.push(Span::styled(after, Style::default().fg(theme::TEXT)));
        }

        let line = Line::from(spans);
        let _ = buf.set_line(area.left(), area.top(), &line, area.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    #[test]
    fn empty_input_renders_prompt_and_cursor() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        InputBox::new("", 0).render(Rect::new(0, 0, 20, 1), &mut buf);
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.symbol(), "❯");
    }

    #[test]
    fn cursor_position_rendered_as_underscore() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        InputBox::new("abc", 2).render(Rect::new(0, 0, 20, 1), &mut buf);
        // ❯ + space + a + b + _ + c
        let cursor_char = buf.cell((4, 0)).unwrap().symbol();
        assert_eq!(cursor_char, "_");
    }
}
