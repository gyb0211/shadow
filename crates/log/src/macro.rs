//! record! 宏 -- 唯一日志发射点
//!
//! 用法:
//!   record!(INFO, Action::Start, "starting agent");
//!   record!(WARN, Action::Fail.with_outcome(EventOutcome::Failure), "tool failed");

/// 发射一条结构化日志事件
#[macro_export]
macro_rules! record {
    ($level:ident, $action:expr, $msg:expr $(,)?) => {{
        let action = $action;
        $crate::__private::tracing::event!(
            target: "shadow_log_event",
            $crate::__private::tracing::Level::$level,
            shadow_action = %action.as_str(),
            message = %$msg,
        );
    }};
}

/// 打开归因 span -- 从 Attributable 对象自动填充归因字段
#[macro_export]
macro_rules! attribution_span {
    ($thing:expr) => {{
        let thing = $thing;
        let role = agent_core::Attributable::role(thing);
        let alias = agent_core::Attributable::alias(thing);
        $crate::__private::tracing::info_span!(
            target: "shadow_log_attribution",
            "shadow_attribution",
            role = %role.family_str(),
            field = %role.attribution_field(),
            alias = %alias,
        )
    }};
}
