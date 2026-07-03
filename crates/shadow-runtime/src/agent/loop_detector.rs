//! 循环检测 -- 检测工具调用循环, 防止 Agent 卡死
//!
//! 借鉴 ZeroClaw 的 loop_detector, 精简版:
//! - 滑动窗口记录最近 N 次工具调用 (名称 + 参数哈希 + 结果哈希)
//! - 检测三种循环模式: 精确重复 / 乒乓交替 / 无进展
//! - 按严重度升序返回: Ok < Warning < Block < Break

use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

/// 循环检测结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopDetectionResult {
    /// 正常, 继续
    Ok,
    /// 注入提示消息到对话 (警告 LLM 换策略)
    Warning(String),
    /// 拒绝工具调用 (结果替换为错误)
    Block(String),
    /// 终止整个工具循环
    Break(String),
}

/// 单次工具调用记录
#[derive(Debug, Clone)]
struct ToolCallRecord {
    name: String,
    args_hash: u64,
    result_hash: u64,
}

/// 循环检测器 -- 滑动窗口检测三种循环模式
pub struct LoopDetector {
    window: VecDeque<ToolCallRecord>,
    window_size: usize, // 默认 20
    max_repeats: usize, // 默认 3
}

impl Default for LoopDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopDetector {
    /// 创建默认检测器 (窗口 20, 最大重复 3)
    pub fn new() -> Self {
        Self {
            window: VecDeque::with_capacity(20),
            window_size: 20,
            max_repeats: 3,
        }
    }

    /// 记录一次工具调用, 返回检测结果
    ///
    /// 按严重度升序检查:
    /// 1. 精确重复 (同工具 + 同参数连续出现)
    /// 2. 乒乓交替 (两工具交替出现)
    /// 3. 无进展 (同工具不同参数但相同结果)
    pub fn record(&mut self, name: &str, args: &Value, result: &str) -> LoopDetectionResult {
        let record = ToolCallRecord {
            name: name.to_string(),
            args_hash: hash_value(args),
            result_hash: hash_str(result),
        };

        if self.window.len() >= self.window_size {
            self.window.pop_front();
        }
        self.window.push_back(record);

        // 按严重度升序检查
        if let Some(r) = self.detect_exact_repeat() {
            return r;
        }
        if let Some(r) = self.detect_ping_pong() {
            return r;
        }
        if let Some(r) = self.detect_no_progress() {
            return r;
        }

        LoopDetectionResult::Ok
    }

    /// 精确重复检测: 同工具 + 同参数 + 同结果连续出现 max_repeats 次
    /// - max_repeats     → Warning
    /// - max_repeats + 1 → Block
    /// - max_repeats + 2 → Break
    ///
    /// 注: 结果哈希也参与比较 -- 同参数不同结果说明工具可能有进展 (如轮询),
    /// 不视为循环.
    fn detect_exact_repeat(&self) -> Option<LoopDetectionResult> {
        let last = self.window.back()?;
        let mut count = 0usize;

        // 从窗口末尾向前数连续相同的调用 (名称 + 参数 + 结果)
        for rec in self.window.iter().rev() {
            if rec.name == last.name
                && rec.args_hash == last.args_hash
                && rec.result_hash == last.result_hash
            {
                count += 1;
            } else {
                break;
            }
        }

        let warn = self.max_repeats;
        let block = self.max_repeats + 1;
        let brk = self.max_repeats + 2;

        if count >= brk {
            return Some(LoopDetectionResult::Break(format!(
                "工具 [{}] 连续重复 {} 次, 终止循环",
                last.name, count
            )));
        }
        if count >= block {
            return Some(LoopDetectionResult::Block(format!(
                "工具 [{}] 连续重复 {} 次, 阻止调用",
                last.name, count
            )));
        }
        if count >= warn {
            return Some(LoopDetectionResult::Warning(format!(
                "工具 [{}] 连续重复 {} 次, 请尝试不同方法",
                last.name, count
            )));
        }

        None
    }

    /// 乒乓检测: 两个工具交替出现 4+ 周期 (8 次调用)
    /// - 4 周期 (8 次) → Warning
    /// - 5 周期 (10 次) → Block
    /// - 6+ 周期 (12 次) → Break
    fn detect_ping_pong(&self) -> Option<LoopDetectionResult> {
        let n = self.window.len();
        if n < 8 {
            return None;
        }

        // 取最近最多 12 条 (6 周期)
        let take = n.min(12);
        let tail: Vec<&ToolCallRecord> = self.window.iter().rev().take(take).collect();
        // tail[0] 是最新, tail[take-1] 是最旧

        // 乒乓模式: 两个工具名交替 (A B A B A B ...)
        // 即 tail[i].name == tail[i+2].name (奇偶各自一致)
        let name_a = tail[0].name.clone();
        let name_b = tail[1].name.clone();
        if name_a == name_b {
            // 两名相同不是乒乓
            return None;
        }

        // 校验整个 tail 区间是否严格交替
        for i in 0..take {
            let expected = if i % 2 == 0 { &name_a } else { &name_b };
            if tail[i].name != *expected {
                return None;
            }
        }

        // 计算完整周期数 (每周期 = 2 次调用)
        let cycles = take / 2;

        if cycles >= 6 {
            return Some(LoopDetectionResult::Break(format!(
                "工具 [{}] 与 [{}] 乒乓交替 {} 周期, 终止循环",
                name_a, name_b, cycles
            )));
        }
        if cycles >= 5 {
            return Some(LoopDetectionResult::Block(format!(
                "工具 [{}] 与 [{}] 乒乓交替 {} 周期, 阻止调用",
                name_a, name_b, cycles
            )));
        }
        if cycles >= 4 {
            return Some(LoopDetectionResult::Warning(format!(
                "工具 [{}] 与 [{}] 乒乓交替 {} 周期, 请尝试不同方法",
                name_a, name_b, cycles
            )));
        }

        None
    }

    /// 无进展检测: 同工具不同参数但相同结果连续 5+ 次 (跨整个窗口)
    /// - 5 次 → Warning
    /// - 6 次 → Block
    /// - 7+ 次 → Break
    fn detect_no_progress(&self) -> Option<LoopDetectionResult> {
        let last = self.window.back()?;
        let target_name = &last.name;
        let target_hash = last.result_hash;

        // 统计窗口中同名 + 同结果哈希的调用数
        // 注: exact_repeat 已先行返回, 这里只需统计同名同结果
        let count = self
            .window
            .iter()
            .filter(|r| r.name == *target_name && r.result_hash == target_hash)
            .count();

        let warn = 5;
        let block = 6;
        let brk = 7;

        if count >= brk {
            return Some(LoopDetectionResult::Break(format!(
                "工具 [{}] 无进展 (相同结果 {} 次), 终止循环",
                target_name, count
            )));
        }
        if count >= block {
            return Some(LoopDetectionResult::Block(format!(
                "工具 [{}] 无进展 (相同结果 {} 次), 阻止调用",
                target_name, count
            )));
        }
        if count >= warn {
            return Some(LoopDetectionResult::Warning(format!(
                "工具 [{}] 无进展 (相同结果 {} 次), 请尝试不同方法",
                target_name, count
            )));
        }

        None
    }
}

/// 规范化 JSON 后哈希 -- 确保键顺序不影响哈希
fn hash_value(value: &Value) -> u64 {
    // 对 Object 用排序键序列化; 其他类型直接 to_string
    let canonical = match value {
        Value::Object(map) => {
            // 用 BTreeMap 排序键
            let sorted: std::collections::BTreeMap<&String, &Value> = map.iter().collect();
            serde_json::to_string(&sorted).unwrap_or_else(|_| value.to_string())
        }
        _ => value.to_string(),
    };
    hash_str(&canonical)
}

/// 字符串哈希
fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Value {
        serde_json::json!({ "input": s })
    }

    // ---- 精确重复 ----

    #[test]
    fn exact_repeat_warning_at_3() {
        // max_repeats = 3 → 3 次 Warning
        let mut det = LoopDetector::new();
        let a = args("hello");
        assert_eq!(det.record("search", &a, "r1"), LoopDetectionResult::Ok);
        assert_eq!(det.record("search", &a, "r1"), LoopDetectionResult::Ok);
        let r3 = det.record("search", &a, "r1");
        assert!(matches!(r3, LoopDetectionResult::Warning(_)));
    }

    #[test]
    fn exact_repeat_block_at_4() {
        let mut det = LoopDetector::new();
        let a = args("hello");
        for _ in 0..3 {
            det.record("search", &a, "r1");
        }
        let r4 = det.record("search", &a, "r1");
        assert!(matches!(r4, LoopDetectionResult::Block(_)));
    }

    #[test]
    fn exact_repeat_break_at_5() {
        let mut det = LoopDetector::new();
        let a = args("hello");
        for _ in 0..4 {
            det.record("search", &a, "r1");
        }
        let r5 = det.record("search", &a, "r1");
        assert!(matches!(r5, LoopDetectionResult::Break(_)));
    }

    // ---- 乒乓 ----

    #[test]
    fn ping_pong_warning_at_4_cycles() {
        // 4 周期 = 8 次 → Warning
        let mut det = LoopDetector::new();
        let a = args("a");
        let b = args("b");
        let mut got_warning = false;
        for _ in 0..4 {
            let r1 = det.record("tool_a", &a, "ra");
            if matches!(r1, LoopDetectionResult::Warning(_)) {
                got_warning = true;
            }
            let r2 = det.record("tool_b", &b, "rb");
            if matches!(r2, LoopDetectionResult::Warning(_)) {
                got_warning = true;
            }
        }
        assert!(got_warning, "4 周期应触发 Warning");
    }

    #[test]
    fn ping_pong_block_at_5_cycles() {
        // 5 周期 = 10 次 → Block
        let mut det = LoopDetector::new();
        let a = args("a");
        let b = args("b");
        let mut got_block = false;
        for _ in 0..5 {
            let r1 = det.record("tool_a", &a, "ra");
            if matches!(r1, LoopDetectionResult::Block(_)) {
                got_block = true;
            }
            let r2 = det.record("tool_b", &b, "rb");
            if matches!(r2, LoopDetectionResult::Block(_)) {
                got_block = true;
            }
        }
        assert!(got_block, "5 周期应触发 Block");
    }

    #[test]
    fn ping_pong_break_at_6_cycles() {
        // 6 周期 = 12 次 → Break
        let mut det = LoopDetector::new();
        let a = args("a");
        let b = args("b");
        for _ in 0..6 {
            let r = det.record("tool_a", &a, "ra");
            if matches!(r, LoopDetectionResult::Break(_)) {
                return;
            }
            let r = det.record("tool_b", &b, "rb");
            if matches!(r, LoopDetectionResult::Break(_)) {
                return;
            }
        }
        panic!("应在 6 周期触发 Break");
    }

    // ---- 无进展 ----

    #[test]
    fn no_progress_warning_at_5() {
        // 同工具不同参数相同结果 5 次 → Warning
        let mut det = LoopDetector::new();
        let mut got_warning = false;
        for i in 0..5 {
            let r = det.record("search", &args(&format!("q{i}")), "empty");
            if matches!(r, LoopDetectionResult::Warning(_)) {
                got_warning = true;
            }
        }
        assert!(got_warning, "5 次应触发 Warning");
    }

    #[test]
    fn no_progress_block_at_6() {
        let mut det = LoopDetector::new();
        for i in 0..6 {
            let r = det.record("search", &args(&format!("q{i}")), "empty");
            if matches!(r, LoopDetectionResult::Block(_)) {
                return;
            }
        }
        panic!("应在 6 次触发 Block");
    }

    #[test]
    fn no_progress_break_at_7() {
        let mut det = LoopDetector::new();
        for i in 0..7 {
            let r = det.record("search", &args(&format!("q{i}")), "empty");
            if matches!(r, LoopDetectionResult::Break(_)) {
                return;
            }
        }
        panic!("应在 7 次触发 Break");
    }

    // ---- 正常 ----

    #[test]
    fn normal_calls_no_trigger() {
        let mut det = LoopDetector::new();
        assert_eq!(
            det.record("search", &args("a"), "r1"),
            LoopDetectionResult::Ok
        );
        assert_eq!(
            det.record("read", &args("b"), "r2"),
            LoopDetectionResult::Ok
        );
        assert_eq!(
            det.record("write", &args("c"), "r3"),
            LoopDetectionResult::Ok
        );
    }

    #[test]
    fn different_results_no_progress_ok() {
        // 同工具同参数但结果不同 → 不是 exact_repeat 也不是 no_progress
        let mut det = LoopDetector::new();
        let a = args("q");
        for i in 0..6 {
            let r = det.record("search", &a, &format!("r{i}"));
            assert_eq!(r, LoopDetectionResult::Ok, "第 {} 次应正常", i + 1);
        }
    }
}
