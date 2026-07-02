//! MessageList widget -- 渲染消息流
//!
//! 手动预折行 + 切片渲染 (不依赖 Paragraph::scroll/Wrap, 后者组合不可靠).
//!
//! 角色配色:
//!   user      ❯ 蓝 (USER)
//!   assistant ❯ 绿 (ASSISTANT)
//!   tool      ❯ 灰 (TOOL_TEXT)

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::theme;
use shadow_core::ChatMessage;

pub struct MessageList<'a> {
    pub messages: &'a [ChatMessage],
    /// 滚动偏移 (从底部向上算). 0 = 跟随最新消息.
    pub scroll_from_bottom: usize,
    /// 是否显示思考内容 (<think> 标签 / reasoning_content)
    pub show_thinking: bool,
}

impl<'a> MessageList<'a> {
    pub fn new(messages: &'a [ChatMessage]) -> Self {
        Self { messages, scroll_from_bottom: 0, show_thinking: false }
    }

    pub fn scroll(mut self, scroll_from_bottom: usize) -> Self {
        self.scroll_from_bottom = scroll_from_bottom;
        self
    }

    pub fn show_thinking(mut self, show: bool) -> Self {
        self.show_thinking = show;
        self
    }
}

/// 将一条 Line 按终端宽度拆成多条 (保留样式)
fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let line_width = line.width();
    if width == 0 || line_width <= width {
        return vec![line.clone()];
    }

    // 展开为 (char, style) 序列
    let mut chars: Vec<(char, Style)> = Vec::new();
    for span in line.spans.iter() {
        for c in span.content.chars() {
            chars.push((c, span.style));
        }
    }

    let mut result = Vec::new();
    for chunk in chars.chunks(width) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut buf = String::new();
        let mut cur_style = chunk[0].1;
        for &(c, style) in chunk {
            if style != cur_style {
                if !buf.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buf), cur_style));
                }
                cur_style = style;
            }
            buf.push(c);
        }
        if !buf.is_empty() {
            spans.push(Span::styled(buf, cur_style));
        }
        result.push(Line::from(spans));
    }
    result
}

impl<'a> Widget for MessageList<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line<'static>> = Vec::new();

        for (idx, msg) in self.messages.iter().enumerate() {
            if idx > 0 {
                lines.push(Line::from(""));
            }

            let (label, color) = match msg.role.as_str() {
                "user" => ("user ❯ ".to_string(), theme::user()),
                "assistant" => ("assistant ❯ ".to_string(), theme::assistant()),
                "tool" => ("tool ❯ ".to_string(), theme::tool_text()),
                _ => (msg.role.clone(), theme::dim()),
            };

            let label_style = Style::default().fg(color).bg(theme::bg());
            let content_style = Style::default().fg(theme::text()).bg(theme::bg());
            let dim_style = Style::default().fg(theme::dim()).bg(theme::bg());
            let think_style = Style::default().fg(theme::dim()).bg(theme::bg());

            // 处理思考内容: show_thinking=false 时过滤 <think> 标签; true 时用 dim 样式显示
            let display_content = if self.show_thinking {
                // 显示思考: 保留原始内容, <think> 块用 dim 样式
                // 同时如果有 reasoning_content, 在内容前显示
                let mut full = String::new();
                if let Some(rc) = &msg.reasoning_content {
                    if !rc.is_empty() {
                        full.push_str(rc);
                        full.push_str("\n");
                    }
                }
                full.push_str(&msg.content);
                full
            } else {
                // 不显示思考: 去除 <think>...</think> 块
                strip_think_blocks(&msg.content)
            };

            // 判断是否在 think 块内 (用于 show_thinking=true 时的行级着色)
            let mut in_think = false;
            let mut content_lines = display_content.lines();
            match content_lines.next() {
                Some(first) => {
                    // 检查第一行是否以 <think> 开头
                    let line_style = if self.show_thinking && first.contains("<think>") {
                        in_think = true;
                        think_style
                    } else if self.show_thinking && in_think {
                        think_style
                    } else {
                        content_style
                    };
                    lines.push(Line::from(vec![
                        Span::styled(label, label_style),
                        Span::styled(first.to_string(), line_style),
                    ]));
                }
                None => {
                    lines.push(Line::from(vec![Span::styled(label, label_style)]));
                }
            }
            for text in content_lines {
                let line_style = if self.show_thinking {
                    if text.contains("</think>") {
                        in_think = false;
                        think_style
                    } else if text.contains("<think>") {
                        in_think = true;
                        think_style
                    } else if in_think {
                        think_style
                    } else {
                        content_style
                    }
                } else {
                    content_style
                };
                lines.push(Line::from(vec![Span::styled(text.to_string(), line_style)]));
            }

            for tc in &msg.tool_calls {
                let args_preview = serde_json::to_string(&tc.arguments).unwrap_or_default();
                let preview = if args_preview.len() > 80 {
                    format!("{}...", &args_preview[..80])
                } else {
                    args_preview
                };
                lines.push(Line::from(vec![
                    Span::styled("  ┌─ ", dim_style),
                    Span::styled(tc.name.clone(), Style::default().fg(theme::tool_dim()).bg(theme::bg())),
                    Span::styled(format!("  {}", preview), dim_style),
                ]));
            }
        }

        // ── 预折行: 按终端宽度拆分, 得到精确的显示行列表 ──
        let width = area.width as usize;
        let mut display_lines: Vec<Line<'static>> = Vec::new();
        for line in &lines {
            for wrapped in wrap_line(line, width) {
                display_lines.push(wrapped);
            }
        }

        // ── 切片: 只渲染可见窗口 ──
        let visible = area.height as usize;
        let total = display_lines.len();
        let bottom_offset = total.saturating_sub(visible);
        let start = bottom_offset.saturating_sub(self.scroll_from_bottom);
        let end = (start + visible).min(total);
        let visible_lines: Vec<Line<'_>> = display_lines[start..end].to_vec();

        Paragraph::new(visible_lines)
            .style(Style::default().bg(theme::bg()))
            .render(area, buf);

        // ── 滚动指示条 ──
        if total > visible && area.width > 0 {
            let scroll_pos = start;
            let bar_height = ((visible * visible) as f64 / total as f64).ceil() as u16;
            let bar_height = bar_height.max(1).min(visible as u16);
            let bar_top = if bottom_offset == 0 {
                0
            } else {
                (scroll_pos as f64 / bottom_offset as f64 * (visible as f64 - bar_height as f64)) as u16
            };
            for i in 0..bar_height {
                let y = area.top() + bar_top + i;
                if y < area.bottom() {
                    if let Some(cell) = buf.cell_mut((area.right() - 1, y)) {
                        cell.set_char('┃');
                        cell.set_fg(theme::dim());
                        cell.set_bg(theme::bg());
                    }
                }
            }
        }
    }
}

/// 去除 `<think>...</think>` 思考块 (用于 show_thinking=false 时的显示过滤)
fn strip_think_blocks(content: &str) -> String {
    let mut result = content.to_string();
    loop {
        match result.find("<think>") {
            Some(start) => match result[start..].find("</think>") {
                Some(end_rel) => {
                    let end_abs = start + end_rel + "</think>".len();
                    result.replace_range(start..end_abs, "");
                }
                None => {
                    // 未闭合 <think>: 删除到末尾
                    result.truncate(start);
                    break;
                }
            },
            None => break,
        }
    }
    result.trim().to_string()
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
        assert_eq!(cell.fg, theme::user());
    }

    #[test]
    fn assistant_label_is_green() {
        let messages = vec![msg("assistant", "hello")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.fg, theme::assistant());
    }

    #[test]
    fn tool_label_is_dim() {
        let messages = vec![msg("tool", "/tmp/foo")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.fg, theme::tool_text());
    }

    #[test]
    fn multiline_content_spans_multiple_rows() {
        let messages = vec![msg("assistant", "第一行\n第二行\n第三行")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 5);
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme::assistant());
        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "第");
        assert_eq!(buf.cell((0, 2)).unwrap().symbol(), "第");
    }

    #[test]
    fn empty_content_still_renders_label() {
        let messages = vec![msg("user", "")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 1);
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme::user());
    }

    #[test]
    fn long_line_wraps_not_truncated() {
        let long = "a".repeat(100);
        let messages = vec![msg("user", &long)];
        let buf = render_to_buffer(MessageList::new(&messages), 60, 3);
        let row1_content: String = (0..60)
            .map(|x| buf.cell((x, 1)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect();
        assert!(row1_content.contains('a'), "第 2 行应包含折行内容");
    }

    #[test]
    fn auto_scroll_shows_latest_message() {
        let messages = vec![
            msg("user", "1"),
            msg("assistant", "2"),
            msg("user", "3"),
            msg("assistant", "4"),
            msg("user", "latest"),
        ];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 3);
        let bottom_row: String = (0..40)
            .map(|x| buf.cell((x, 2)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect();
        assert!(bottom_row.contains("latest"), "底部行应显示最新消息");
    }

    #[test]
    fn gap_between_messages() {
        let messages = vec![msg("user", "a"), msg("assistant", "b")];
        let buf = render_to_buffer(MessageList::new(&messages), 40, 5);
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme::user());
        assert_eq!(buf.cell((0, 2)).unwrap().fg, theme::assistant());
    }

    #[test]
    fn scroll_up_shows_older_messages() {
        // 10 条消息, 3 行可见, scroll_from_bottom=5 → 不应看到最新消息
        let messages: Vec<ChatMessage> = (0..10)
            .map(|i| msg("user", &format!("msg{i}")))
            .collect();
        let buf = render_to_buffer(MessageList::new(&messages).scroll(5), 40, 3);
        // 可见区域至少应有一条消息
        let mut all = String::new();
        for y in 0..3 {
            for x in 0..40 {
                if let Some(c) = buf.cell((x, y)) {
                    all.push_str(c.symbol());
                }
            }
        }
        assert!(all.contains("msg"), "滚动后可见区域应有消息, got: {all:?}");
        // 不应包含 msg9 (最新的, 在底部)
        assert!(!all.contains("msg9"), "滚动向上后不应看到最新消息 msg9");
    }

    #[test]
    fn wrapped_long_message_auto_scrolls_correctly() {
        let long = "a".repeat(100);
        let messages = vec![msg("assistant", &long)];
        let buf = render_to_buffer(MessageList::new(&messages), 20, 3);
        let bottom_char = buf.cell((0, 2)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(bottom_char, "a", "长消息末尾应通过自动滚动可见");
    }
}
