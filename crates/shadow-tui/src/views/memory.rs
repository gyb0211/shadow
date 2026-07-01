//! MemoryView -- memory 条目列表 + 顶部搜索框

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::app::MemoryViewState;
use crate::theme;

pub struct MemoryView<'a> {
    pub state: &'a MemoryViewState,
}

impl<'a> MemoryView<'a> {
    pub fn new(state: &'a MemoryViewState) -> Self {
        Self { state }
    }

    /// 应用 query 过滤的条目
    pub fn filtered(&self) -> Vec<&shadow_core::MemoryEntry> {
        if self.state.query.is_empty() {
            self.state.entries.iter().collect()
        } else {
            self.state.entries.iter()
                .filter(|e| e.key.contains(&self.state.query) || e.content.contains(&self.state.query))
                .collect()
        }
    }
}

impl<'a> Widget for MemoryView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 第 0 行: 搜索框
        let qstyle = Style::default().fg(theme::accent());
        let _ = buf.set_line(
            area.left(), area.top(),
            &Line::from(vec![
                Span::styled("/ ", qstyle),
                Span::styled(self.state.query.clone(), Style::default().fg(theme::text())),
            ]),
            area.width,
        );

        // 后续行: 条目
        let items = self.filtered();
        if items.is_empty() && !self.state.entries.is_empty() {
            let _ = buf.set_line(
                area.left(), area.top() + 1,
                &Line::from(Span::styled("(无匹配)", Style::default().fg(theme::dim()))),
                area.width,
            );
            return;
        }
        for (i, entry) in items.iter().enumerate() {
            let y = area.top() + 1 + i as u16;
            if y >= area.bottom() { break; }
            let preview: String = entry.content.chars().take(50).collect();
            let _ = buf.set_line(
                area.left(), y,
                &Line::from(vec![
                    Span::styled(format!("{} ", entry.key), Style::default().fg(theme::user())),
                    Span::styled(preview, Style::default().fg(theme::dim())),
                ]),
                area.width,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::MemoryEntry;

    fn entry(key: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: key.to_string(),
            key: key.to_string(),
            content: content.to_string(),
            category: "note".to_string(),
            timestamp: chrono::Utc::now(),
            session_id: None,
            agent_alias: None,
        }
    }

    #[test]
    fn empty_query_shows_all_entries() {
        let mut state = MemoryViewState::default();
        state.entries = vec![entry("a", "alpha"), entry("b", "beta")];
        let v = MemoryView::new(&state);
        assert_eq!(v.filtered().len(), 2);
    }

    #[test]
    fn query_filters_by_key_or_content() {
        let mut state = MemoryViewState::default();
        state.entries = vec![entry("alpha", "AAA"), entry("beta", "BBB")];
        state.query = "alpha".to_string();
        let v = MemoryView::new(&state);
        assert_eq!(v.filtered().len(), 1);
        assert_eq!(v.filtered()[0].key, "alpha");

        state.query = "BBB".to_string();
        let v = MemoryView::new(&state);
        assert_eq!(v.filtered().len(), 1);
        assert_eq!(v.filtered()[0].key, "beta");
    }
}
