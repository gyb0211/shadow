//! GitHub Dark 主题配色
//!
//! - user: 蓝 (#58a6ff)
//! - assistant: 绿 (#7ee787)
//! - tool: 灰 (#8b949e / #6e7681)
//! - error: 红
//! - 背景: #0d1117

use ratatui::style::Color;

pub const USER:      Color = Color::Rgb(0x58, 0xa6, 0xff);
pub const ASSISTANT: Color = Color::Rgb(0x7e, 0xe7, 0x87);
pub const TOOL_DIM:  Color = Color::Rgb(0x6e, 0x76, 0x81);
pub const TOOL_TEXT: Color = Color::Rgb(0x8b, 0x94, 0x9e);
pub const ERROR:     Color = Color::Rgb(0xf8, 0x53, 0x73);
pub const TEXT:      Color = Color::Rgb(0xe6, 0xed, 0xf3);
pub const DIM:       Color = Color::Rgb(0x6e, 0x76, 0x81);
pub const BG:        Color = Color::Rgb(0x0d, 0x11, 0x17);
pub const ACCENT:    Color = Color::Rgb(0x58, 0xa6, 0xff);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_color_is_blue_rgb() {
        assert_eq!(USER, Color::Rgb(0x58, 0xa6, 0xff));
    }

    #[test]
    fn assistant_color_is_green_rgb() {
        assert_eq!(ASSISTANT, Color::Rgb(0x7e, 0xe7, 0x87));
    }

    #[test]
    fn tool_colors_differ_from_user_assistant() {
        assert_ne!(TOOL_TEXT, USER);
        assert_ne!(TOOL_TEXT, ASSISTANT);
    }
}
