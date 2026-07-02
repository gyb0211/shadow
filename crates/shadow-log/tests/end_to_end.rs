//! 端到端验证: record! 宏 → tracing → LogCaptureLayer → JSONL 文件
//!
//! 这是对 install_subscriber 过滤器配置的关键回归测试 --
//! 之前 fmt_filter 挂在 registry 级别, INFO 级 record! 被 warn 默认值
//! 先拦截, 导致 JSONL 文件永远为空.

use std::path::PathBuf;

/// 计算本次测试专用的 workspace dir, 避免污染真实 ~/.shadow
fn test_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "shadow-log-e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn record_event_reaches_jsonl_file() {
    let workspace = test_workspace();
    shadow_log::init_from_config(&workspace, 10_000);
    shadow_log::install_subscriber(false); // 默认 warn 模式

    // 发射一条 INFO 级 record! 事件
    shadow_log::record!(INFO, shadow_log::Action::Start, "test e2e event");

    // 给 tracing 一点时间刷盘 (事件是同步派发的, 但 BufWriter flush 是即时的)
    std::thread::sleep(std::time::Duration::from_millis(50));

    let log_path = workspace.join("logs").join("runtime-trace.jsonl");
    let content = std::fs::read_to_string(&log_path)
        .unwrap_or_else(|e| panic!("读取日志文件失败 {}: {e}", log_path.display()));

    assert!(
        content.contains("test e2e event"),
        "record! 事件未写入 JSONL, content = {content}"
    );
    assert!(
        content.contains("\"action\":\"start\""),
        "action 字段错误, content = {content}"
    );

    // 清理
    let _ = std::fs::remove_dir_all(&workspace);
}
