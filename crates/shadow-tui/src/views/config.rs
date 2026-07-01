//! ConfigView -- 列出 config.toml 的键值, 行选中后弹 InputBox 编辑

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;
use shadow_config::Config;

pub struct ConfigView<'a> {
    pub config: &'a Config,
    pub selected: usize,
}

impl<'a> ConfigView<'a> {
    pub fn new(config: &'a Config, selected: usize) -> Self {
        Self { config, selected }
    }

    /// 把 config 扁平成 (path, value) 列表
    pub fn flatten(cfg: &Config) -> Vec<(String, String)> {
        let mut out = Vec::new();
        out.push(("agent.alias".to_string(), cfg.agent.alias.clone()));
        out.push(("agent.model_provider".to_string(), cfg.agent.model_provider.clone()));
        out.push(("agent.model".to_string(), cfg.agent.model.clone()));
        if let Some(t) = cfg.agent.temperature {
            out.push(("agent.temperature".to_string(), format!("{t}")));
        }
        out.push(("agent.autonomy".to_string(), cfg.agent.autonomy.clone()));
        out.push(("agent.max_iterations".to_string(), cfg.agent.max_iterations.to_string()));
        out.push(("agent.max_history".to_string(), cfg.agent.max_history.to_string()));
        if let Some(p) = &cfg.agent.system_prompt {
            out.push(("agent.system_prompt".to_string(), p.clone()));
        }
        out.push(("memory.backend".to_string(), cfg.memory.backend.clone()));

        // providers.<family>.<alias>.<field>
        for (family, aliases) in &cfg.providers.families {
            for (alias, entry) in aliases {
                if let Some(k) = &entry.api_key {
                    out.push((format!("providers.{family}.{alias}.api_key"), k.clone()));
                }
                if let Some(m) = &entry.model {
                    out.push((format!("providers.{family}.{alias}.model"), m.clone()));
                }
                if let Some(u) = &entry.base_url {
                    out.push((format!("providers.{family}.{alias}.base_url"), u.clone()));
                }
            }
        }
        out
    }
}

impl<'a> Widget for ConfigView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rows = Self::flatten(self.config);
        for (i, (path, value)) in rows.iter().enumerate() {
            let y = area.top() + i as u16;
            if y >= area.bottom() { break; }
            let style = if i == self.selected {
                Style::default().fg(theme::TEXT).bg(theme::TOOL_DIM)
            } else {
                Style::default().fg(theme::DIM)
            };
            let line = Line::from(vec![
                Span::styled(format!("{path:<40} "), style),
                Span::styled(value.clone(), Style::default().fg(theme::TEXT)),
            ]);
            let _ = buf.set_line(area.left(), y, &line, area.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_includes_agent_alias() {
        let cfg = Config::default();
        let rows = ConfigView::flatten(&cfg);
        let paths: Vec<_> = rows.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"agent.alias"));
        assert!(paths.contains(&"agent.model_provider"));
    }

    #[test]
    fn flatten_includes_providers_when_present() {
        let mut cfg = Config::default();
        cfg.providers.find_or_create("openai", "minimax").api_key = Some("sk-x".into());
        let rows = ConfigView::flatten(&cfg);
        let paths: Vec<_> = rows.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"providers.openai.minimax.api_key"));
    }
}
