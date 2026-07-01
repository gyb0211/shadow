//! InputBox widget -- 输入框 (多行渲染 + 光标)
//!
//! 按 \n 拆行, 每行渲染; 第一行带 ❯ 提示符.
//! 光标以 _ (下划线) 标记, 定位在 cursor 所在行.

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
        let prompt_style = Style::default().fg(theme::user());
        let text_style = Style::default().fg(theme::text());
        let cursor_style = Style::default().fg(theme::accent());

        // 按 \n 拆行; 计算光标在哪一行哪一列
        let chars: Vec<char> = self.text.chars().collect();
        let cursor_pos = self.cursor.min(chars.len());

        // 找到光标所在行
        let mut line_start = 0usize;
        let mut line_idx = 0usize;
        for (i, &c) in chars.iter().enumerate() {
            if i == cursor_pos {
                break;
            }
            if c == '\n' {
                line_start = i + 1;
                line_idx += 1;
            }
        }
        // 如果光标在末尾且最后字符是 \n
        if cursor_pos == chars.len() {
            let newlines = chars[..cursor_pos].iter().filter(|&&c| c == '\n').count();
            line_idx = newlines;
            line_start = chars[..cursor_pos].iter().rposition(|&c| c == '\n').map(|p| p + 1).unwrap_or(0);
        }
        let col_in_line = cursor_pos - line_start;

        // 拆行
        let all_lines: Vec<&str> = self.text.split('\n').collect();

        for (row, text_line) in all_lines.iter().enumerate() {
            if row as u16 + area.top() >= area.bottom() {
                break; // 超出区域
            }

            let y = area.top() + row as u16;
            let mut spans = Vec::new();

            // 第一行带提示符
            if row == 0 {
                spans.push(Span::styled("❯ ", prompt_style));
            } else {
                spans.push(Span::styled("  ", prompt_style));
            }

            let line_chars: Vec<char> = text_line.chars().collect();

            if row == line_idx {
                // 光标所在行: 在 col_in_line 处插入 _
                let before: String = line_chars[..col_in_line.min(line_chars.len())].iter().collect();
                let after: String = line_chars[col_in_line.min(line_chars.len())..].iter().collect();
                if !before.is_empty() {
                    spans.push(Span::styled(before, text_style));
                }
                spans.push(Span::styled("_", cursor_style));
                if !after.is_empty() {
                    spans.push(Span::styled(after, text_style));
                }
            } else {
                if !text_line.is_empty() {
                    spans.push(Span::styled(*text_line, text_style));
                }
            }

            let line = Line::from(spans);
            let _ = buf.set_line(area.left(), y, &line, area.width);
        }

        // 空输入也要画提示符 + 光标
        if all_lines.is_empty() {
            let line = Line::from(vec![
                Span::styled("❯ ", prompt_style),
                Span::styled("_", cursor_style),
            ]);
            let _ = buf.set_line(area.left(), area.top(), &line, area.width);
        }
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

    #[test]
    fn multiline_input_renders_on_multiple_rows() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 5));
        InputBox::new("hello\nworld", 11).render(Rect::new(0, 0, 20, 5), &mut buf);
        // 第一行: ❯ hello
        assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "h");
        // 第二行:   world
        assert_eq!(buf.cell((2, 1)).unwrap().symbol(), "w");
    }

    #[test]
    fn cursor_on_second_line() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 5));
        // "hello\nwor|ld" -- cursor 在第 2 行第 3 列
        InputBox::new("hello\nworld", 9).render(Rect::new(0, 0, 20, 5), &mut buf);
        // 第 2 行: "  wor_ld"
        let cell = buf.cell((5, 1)).unwrap();
        assert_eq!(cell.symbol(), "_");
    }
}
