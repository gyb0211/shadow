//! ChatView -- 渲染消息流 (不含输入框, 输入框由顶层布局统一渲染)

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::app::ChatState;
use crate::widgets::MessageList;

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
        MessageList::new(&self.state.messages)
            .scroll(self.state.scroll_offset)
            .show_thinking(self.state.show_thinking)
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::ChatMessage;

    #[test]
    fn renders_messages() {
        let mut state = ChatState::default();
        state.messages.push(ChatMessage {
            role: "user".into(), content: "hi".into(),
            tool_call_id: None, tool_calls: vec![], reasoning_content: None,
        });

        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        ChatView::new(&state).render(Rect::new(0, 0, 40, 10), &mut buf);

        // 第一行应该是 user 消息
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "u");
    }
}
