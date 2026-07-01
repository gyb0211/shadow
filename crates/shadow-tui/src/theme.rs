//! 主题配色 -- 自动跟随终端背景色 (Dark / Light)
//!
//! 启动时通过 COLORFGBG 环境变量或终端查询检测背景色:
//! - Dark: GitHub Dark 配色 (深色背景 + 亮色文字)
//! - Light: GitHub Light 配色 (浅色背景 + 深色文字)
//!
//! 检测方式:
//! 1. COLORFGBG 环境变量 (格式 "fg;bg", bg < 7 为暗色)
//! 2. 默认 Dark

use ratatui::style::Color;
use std::sync::OnceLock;

/// 主题数据 -- 持有所有颜色
struct ThemeData {
    pub user: Color,
    pub assistant: Color,
    pub tool_dim: Color,
    pub tool_text: Color,
    pub error: Color,
    pub text: Color,
    pub dim: Color,
    pub bg: Color,
    pub accent: Color,
}

impl ThemeData {
    /// GitHub Dark 配色
    fn dark() -> Self {
        Self {
            user:      Color::Rgb(0x58, 0xa6, 0xff),
            assistant: Color::Rgb(0x7e, 0xe7, 0x87),
            tool_dim:  Color::Rgb(0x6e, 0x76, 0x81),
            tool_text: Color::Rgb(0x8b, 0x94, 0x9e),
            error:     Color::Rgb(0xf8, 0x53, 0x73),
            text:      Color::Rgb(0xe6, 0xed, 0xf3),
            dim:       Color::Rgb(0x6e, 0x76, 0x81),
            bg:        Color::Rgb(0x0d, 0x11, 0x17),
            accent:    Color::Rgb(0x58, 0xa6, 0xff),
        }
    }

    /// GitHub Light 配色
    fn light() -> Self {
        Self {
            user:      Color::Rgb(0x09, 0x69, 0xe3),
            assistant: Color::Rgb(0x1a, 0x7f, 0x37),
            tool_dim:  Color::Rgb(0xae, 0xae, 0xae),
            tool_text: Color::Rgb(0x65, 0x6d, 0x76),
            error:     Color::Rgb(0xcf, 0x22, 0x2e),
            text:      Color::Rgb(0x24, 0x2f, 0x3f),
            dim:       Color::Rgb(0x82, 0x88, 0x93),
            bg:        Color::Rgb(0xff, 0xff, 0xff),
            accent:    Color::Rgb(0x09, 0x69, 0xe3),
        }
    }
}

static THEME: OnceLock<ThemeData> = OnceLock::new();

/// 初始化主题 -- 检测终端背景色
/// 在 TUI 启动前调用一次
pub fn init() {
    let dark = detect_dark_terminal();
    let data = if dark { ThemeData::dark() } else { ThemeData::light() };
    let _ = THEME.set(data);
}

/// 检测终端是否为暗色背景
///
/// 优先级:
/// 1. COLORFGBG 环境变量 (格式 "fg;bg", bg < 7 为暗色)
/// 2. 默认 true (大多数终端默认暗色)
fn detect_dark_terminal() -> bool {
    if let Ok(colorfgbg) = std::env::var("COLORFGBG") {
        // 格式: "0;7" 或 "15;0" -- 第二个数字是背景色
        if let Some(bg_part) = colorfgbg.split(';').nth(1) {
            if let Ok(bg) = bg_part.trim().parse::<u8>() {
                // 0-6 为暗色, 7-15 为亮色
                return bg < 7;
            }
        }
    }
    // 默认暗色
    true
}

/// 获取当前主题数据
fn current() -> &'static ThemeData {
    THEME.get_or_init(ThemeData::dark)
}

// ── 公共访问函数 (替代原来的 const) ──

pub fn user() -> Color { current().user }
pub fn assistant() -> Color { current().assistant }
pub fn tool_dim() -> Color { current().tool_dim }
pub fn tool_text() -> Color { current().tool_text }
pub fn error() -> Color { current().error }
pub fn text() -> Color { current().text }
pub fn dim() -> Color { current().dim }
pub fn bg() -> Color { current().bg }
pub fn accent() -> Color { current().accent }

/// 当前是否为暗色主题
pub fn is_dark() -> bool {
    current().bg == ThemeData::dark().bg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_colors_correct() {
        let t = ThemeData::dark();
        assert_eq!(t.bg, Color::Rgb(0x0d, 0x11, 0x17));
        assert_eq!(t.text, Color::Rgb(0xe6, 0xed, 0xf3));
    }

    #[test]
    fn light_theme_colors_correct() {
        let t = ThemeData::light();
        assert_eq!(t.bg, Color::Rgb(0xff, 0xff, 0xff));
        assert_eq!(t.text, Color::Rgb(0x24, 0x2f, 0x3f));
    }

    #[test]
    fn dark_and_light_differ() {
        let d = ThemeData::dark();
        let l = ThemeData::light();
        assert_ne!(d.bg, l.bg);
        assert_ne!(d.text, l.text);
        assert_ne!(d.user, l.user);
    }
}
