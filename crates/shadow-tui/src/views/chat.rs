//! ChatView -- 组合 MessageList + InputBox, 渲染单屏 chat

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::Widget;

use crate::app::ChatState;
use crate::widgets::{InputBox, MessageList};

pub struct ChatView<'a> {
    pub state: &'a ChatState,
}

impl<'a> ChatView<'a> {
    pub fn new(state: &'a ChatState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for ChatView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 70% 消息流, 30% 输入框 (最少 3 行)
        let constraints = [
            Constraint::Percentage(70),
            Constraint::Min(3),
        ];
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        MessageList::new(&self.state.messages)
            .scroll(self.state.scroll_offset)
            .render(chunks[0], buf);
        InputBox::new(&self.state.input, self.state.input.chars().count()).render(chunks[1], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::ChatMessage;

    #[test]
    fn renders_messages_then_input() {
        let mut state = ChatState::default();
        state.messages.push(ChatMessage {
            role: "user".into(), content: "hi".into(),
            tool_call_id: None, tool_calls: vec![], reasoning_content: None,
        });
        state.input = "draft".to_string();

        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        ChatView::new(&state).render(Rect::new(0, 0, 40, 10), &mut buf);

        // 第一行应该是 user 消息
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "u");
    }
}
