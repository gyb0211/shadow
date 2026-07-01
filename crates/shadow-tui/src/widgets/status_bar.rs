//! StatusBar widget -- 顶/底状态行

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;

pub struct StatusBar<'a> {
    pub left: &'a str,
    pub right: &'a str,
}

impl<'a> StatusBar<'a> {
    pub fn new(left: &'a str, right: &'a str) -> Self {
        Self { left, right }
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let style = Style::default().fg(theme::DIM);
        let left = Line::from(vec![Span::styled(self.left.to_string(), style)]);
        let _ = buf.set_line(area.left(), area.top(), &left, area.width);

        if !self.right.is_empty() {
            let right_str = self.right.to_string();
            let right_len = right_str.chars().count() as u16;
            if right_len < area.width {
                let x = area.right().saturating_sub(right_len);
                let right = Line::from(vec![Span::styled(right_str, style)]);
                let _ = buf.set_line(x, area.top(), &right, right_len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_left_text() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        StatusBar::new("hello", "").render(Rect::new(0, 0, 40, 1), &mut buf);
        // 取前 5 个字符
        let s: String = (0..5)
            .map(|x| {
                buf.cell((x, 0))
                    .unwrap()
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        assert_eq!(s, "hello");
    }

    #[test]
    fn renders_right_text_at_right_edge() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        StatusBar::new("", "OK").render(Rect::new(0, 0, 40, 1), &mut buf);
        let at_38 = buf.cell((38, 0)).unwrap().symbol();
        let at_39 = buf.cell((39, 0)).unwrap().symbol();
        assert_eq!(format!("{at_38}{at_39}"), "OK");
    }
}
