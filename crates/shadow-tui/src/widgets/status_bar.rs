//! StatusBar widget -- 两行插件化状态栏
//!
//! 第 1 行: 左侧 segments + 右侧 segments (用 · 分隔)
//! 第 2 行: 提示文本 或 错误 (红色)
//!
//! 插件化: 调用方提供 Vec<StatusSegment>, 每段独立着色.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;

/// 单个状态段 (一个插件单元)
#[derive(Clone, Debug)]
pub struct StatusSegment {
    pub text: String,
    pub color: Color,
}

impl StatusSegment {
    pub fn new(text: impl Into<String>, color: Color) -> Self {
        Self { text: text.into(), color }
    }
}

/// 两行状态栏数据
pub struct StatusBarData {
    pub left: Vec<StatusSegment>,
    pub right: Vec<StatusSegment>,
    /// 第 2 行: 提示文本 (error 为 None 时显示)
    pub hint: String,
    /// 错误 (覆盖 hint, 红色显示)
    pub error: Option<String>,
}

impl StatusBarData {
    pub fn new() -> Self {
        Self {
            left: vec![],
            right: vec![],
            hint: String::new(),
            error: None,
        }
    }

    pub fn push_left(&mut self, seg: StatusSegment) -> &mut Self {
        self.left.push(seg);
        self
    }

    pub fn push_right(&mut self, seg: StatusSegment) -> &mut Self {
        self.right.push(seg);
        self
    }
}

impl Default for StatusBarData {
    fn default() -> Self { Self::new() }
}

pub struct StatusBar<'a> {
    pub data: &'a StatusBarData,
}

impl<'a> StatusBar<'a> {
    pub fn new(data: &'a StatusBarData) -> Self {
        Self { data }
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let sep = Span::styled(" · ", Style::default().fg(theme::DIM));

        // ── 第 1 行: left + right ──
        if area.height >= 1 {
            let mut spans: Vec<Span<'_>> = Vec::new();
            for (i, seg) in self.data.left.iter().enumerate() {
                if i > 0 { spans.push(sep.clone()); }
                spans.push(Span::styled(
                    seg.text.clone(),
                    Style::default().fg(seg.color).bg(theme::BG),
                ));
            }
            let left_line = Line::from(spans);
            let _ = buf.set_line(area.left(), area.top(), &left_line, area.width);

            // 右侧 segments 右对齐
            if !self.data.right.is_empty() {
                let mut right_spans: Vec<Span<'_>> = Vec::new();
                for (i, seg) in self.data.right.iter().enumerate() {
                    if i > 0 { right_spans.insert(0, sep.clone()); }
                    right_spans.push(Span::styled(
                        seg.text.clone(),
                        Style::default().fg(seg.color).bg(theme::BG),
                    ));
                }
                let right_line = Line::from(right_spans);
                let right_len = right_line.width() as u16;
                if right_len < area.width {
                    let x = area.right().saturating_sub(right_len);
                    let _ = buf.set_line(x, area.top(), &right_line, right_len);
                }
            }
        }

        // ── 第 2 行: hint / error ──
        if area.height >= 2 {
            let y = area.top() + 1;
            if let Some(err) = &self.data.error {
                let line = Line::from(vec![
                    Span::styled("⚠ ", Style::default().fg(theme::ERROR).bg(theme::BG)),
                    Span::styled(err.clone(), Style::default().fg(theme::ERROR).bg(theme::BG)),
                ]);
                let _ = buf.set_line(area.left(), y, &line, area.width);
            } else if !self.data.hint.is_empty() {
                let line = Line::from(vec![
                    Span::styled(self.data.hint.clone(), Style::default().fg(theme::DIM).bg(theme::BG)),
                ]);
                let _ = buf.set_line(area.left(), y, &line, area.width);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(data: &StatusBarData, w: u16, h: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        StatusBar::new(data).render(Rect::new(0, 0, w, h), &mut buf);
        buf
    }

    #[test]
    fn left_segment_rendered() {
        let mut data = StatusBarData::new();
        data.push_left(StatusSegment::new("shadow", theme::TEXT));
        let buf = render(&data, 40, 2);
        let s: String = (0..6).map(|x| buf.cell((x, 0)).unwrap().symbol().chars().next().unwrap()).collect();
        assert_eq!(s, "shadow");
    }

    #[test]
    fn multiple_left_segments_separated_by_dot() {
        let mut data = StatusBarData::new();
        data.push_left(StatusSegment::new("a", theme::TEXT));
        data.push_left(StatusSegment::new("b", theme::TEXT));
        let buf = render(&data, 40, 2);
        // "a · b"
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "a");
        assert_eq!(buf.cell((4, 0)).unwrap().symbol(), "b");
    }

    #[test]
    fn right_segment_at_right_edge() {
        let mut data = StatusBarData::new();
        data.push_right(StatusSegment::new("OK", theme::ACCENT));
        let buf = render(&data, 40, 2);
        assert_eq!(buf.cell((38, 0)).unwrap().symbol(), "O");
        assert_eq!(buf.cell((39, 0)).unwrap().symbol(), "K");
    }

    #[test]
    fn hint_on_second_line() {
        let mut data = StatusBarData::new();
        data.hint = "press Enter".into();
        let buf = render(&data, 40, 2);
        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "p");
    }

    #[test]
    fn error_overrides_hint_on_second_line() {
        let mut data = StatusBarData::new();
        data.hint = "normal hint".into();
        data.error = Some("boom!".into());
        let buf = render(&data, 40, 2);
        // 第 2 行应显示 ⚠ boom!
        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "⚠");
        let cell = buf.cell((2, 1)).unwrap();
        assert_eq!(cell.fg, theme::ERROR);
    }
}
