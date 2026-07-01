//! CommandPalette widget -- ⌘K 弹层
//!
//! 中央浮层, 显示过滤后的命令列表

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;

pub struct CommandPalette<'a> {
    pub query: &'a str,
    pub items: &'a [&'static str],
    pub selected: usize,
}

impl<'a> CommandPalette<'a> {
    pub fn new(query: &'a str, items: &'a [&'static str], selected: usize) -> Self {
        Self { query, items, selected }
    }
}

impl<'a> Widget for CommandPalette<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 居中浮层: 宽 60%, 高根据 items + 2 (query line)
        let w = area.width.min(60);
        let h = (self.items.len() as u16 + 2).min(area.height.saturating_sub(2));
        let x = area.left() + (area.width - w) / 2;
        let y = area.top() + (area.height - h) / 2;
        let layer = Rect::new(x, y, w, h);

        // 背景填充
        for yy in layer.top()..layer.bottom() {
            for xx in layer.left()..layer.right() {
                if let Some(cell) = buf.cell_mut((xx, yy)) {
                    cell.set_char(' ').set_style(Style::default().bg(theme::bg()));
                }
            }
        }

        // 查询行
        let qstyle = Style::default().fg(theme::accent());
        let _ = buf.set_line(
            layer.left(),
            layer.top(),
            &Line::from(vec![
                Span::styled("> ", qstyle),
                Span::styled(self.query.to_string(), Style::default().fg(theme::text())),
            ]),
            layer.width,
        );

        // 项列表
        for (i, item) in self.items.iter().enumerate() {
            let yy = layer.top() + 1 + i as u16;
            if yy >= layer.bottom() { break; }
            let style = if i == self.selected {
                Style::default().fg(theme::text()).bg(theme::tool_dim())
            } else {
                Style::default().fg(theme::dim())
            };
            let _ = buf.set_line(
                layer.left(),
                yy,
                &Line::from(vec![Span::styled(format!("  {item}"), style)]),
                layer.width,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_query_line_at_top_of_layer() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let items = vec!["chat", "config"];
        CommandPalette::new("c", &items, 0).render(Rect::new(0, 0, 80, 24), &mut buf);
        // 第一行第一字符应该是 '>'
        // 由于浮层居中, 直接扫描整个 buf 找 '>'
        let mut found = false;
        for y in 0..24 {
            for x in 0..80 {
                if buf.cell((x, y)).unwrap().symbol() == ">" { found = true; break; }
            }
        }
        assert!(found);
    }

    #[test]
    fn selected_item_uses_text_color() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let items = vec!["chat"];
        CommandPalette::new("", &items, 0).render(Rect::new(0, 0, 80, 24), &mut buf);
        // 选中项有 TOOL_DIM 背景
        // ratatui 0.28: Cell.bg is Color (not Option<Color>)
        let mut found = false;
        for y in 0..24 {
            for x in 0..80 {
                let cell = buf.cell((x, y)).unwrap();
                if cell.bg == theme::tool_dim() || cell.fg == theme::tool_dim() {
                    found = true; break;
                }
            }
        }
        assert!(found);
    }
}
