//! Prompt 注入防护 -- 检测上下文文件中的注入攻击
//!
//! 参考 Hermes Agent 的 prompt_builder.py 实现

use regex::Regex;
use std::sync::OnceLock;

/// 扫描结果
pub struct ScanResult {
    pub safe: bool,
    pub findings: Vec<String>,
    pub sanitized: String,
}

/// 威胁模式
struct ThreatPattern {
    regex: Regex,
    id: &'static str,
}

static THREAT_PATTERNS: OnceLock<Vec<ThreatPattern>> = OnceLock::new();

fn threat_patterns() -> &'static Vec<ThreatPattern> {
    THREAT_PATTERNS.get_or_init(|| vec![
        ThreatPattern { regex: Regex::new(r"(?i)ignore\s+(previous|all|above|prior)\s+instructions").unwrap(), id: "prompt_injection" },
        ThreatPattern { regex: Regex::new(r"(?i)do\s+not\s+tell\s+the\s+user").unwrap(), id: "deception_hide" },
        ThreatPattern { regex: Regex::new(r"(?i)system\s+prompt\s+override").unwrap(), id: "sys_prompt_override" },
        ThreatPattern { regex: Regex::new(r"(?i)disregard\s+(your|all|any)\s+(instructions|rules|guidelines)").unwrap(), id: "disregard_rules" },
        ThreatPattern { regex: Regex::new(r"(?i)act\s+as\s+(if|though)\s+you\s+(have\s+no|don't\s+have)\s+(restrictions|limits|rules)").unwrap(), id: "bypass_restrictions" },
        ThreatPattern { regex: Regex::new(r"(?i)<!--[^>]*(?:ignore|override|system|secret|hidden)[^>]*-->").unwrap(), id: "html_comment_injection" },
        ThreatPattern { regex: Regex::new(concat!(r"(?i)<\s*div\s+style\s*=\s*['", r#"""#, r"'][\s\S]*?display\s*:\s*none")).unwrap(), id: "hidden_div" },
        ThreatPattern { regex: Regex::new(r"(?i)translate\s+.*\s+into\s+.*\s+and\s+(execute|run|eval)").unwrap(), id: "translate_execute" },
        ThreatPattern { regex: Regex::new(r"(?i)curl\s+[^\n]*\$\{?\w*(KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)").unwrap(), id: "exfil_curl" },
        ThreatPattern { regex: Regex::new(r"(?i)cat\s+[^\n]*(\.env|credentials|\.netrc|\.pgpass)").unwrap(), id: "read_secrets" },
    ])
}

/// 不可见 Unicode 字符
const INVISIBLE_CHARS: &[char] = &[
    '\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{feff}', '\u{202a}', '\u{202b}', '\u{202c}',
    '\u{202d}', '\u{202e}',
];

/// 扫描上下文文件内容, 检测注入攻击
pub fn scan_context_content(content: &str, filename: &str) -> ScanResult {
    let mut findings = Vec::new();

    // 检测不可见 Unicode 字符
    for ch in INVISIBLE_CHARS {
        if content.contains(*ch) {
            findings.push(format!("invisible_unicode_U+{:04X}", *ch as u32));
        }
    }

    // 检测威胁模式
    for pattern in threat_patterns() {
        if pattern.regex.is_match(content) {
            findings.push(pattern.id.to_string());
        }
    }

    if findings.is_empty() {
        ScanResult {
            safe: true,
            findings,
            sanitized: content.to_string(),
        }
    } else {
        let finding_str = findings.join(", ");
        let sanitized =
            format!("[BLOCKED: {filename} 包含潜在 prompt 注入 ({finding_str}). 内容未加载.]");
        ScanResult {
            safe: false,
            findings,
            sanitized,
        }
    }
}

