//! 循环检测 -- 检测工具调用循环, 防止 Agent 卡死
//!
//! 借鉴 ZeroClaw 的 loop_detector, 精简版:
//! - 滑动窗口记录最近 N 次工具调用 (名称 + 参数哈希 + 结果哈希)
//! - 检测三种循环模式: 精确重复 / 乒乓交替 / 无进展
//! - 按严重度升序返回: Ok < Warning < Block < Break

use serde_json::Value;
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
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
        for (i, item) in tail.iter().enumerate().take(take) {
            let expected = if i % 2 == 0 { &name_a } else { &name_b };
            if item.name != *expected {
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

