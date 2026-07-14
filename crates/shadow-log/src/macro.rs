//! record! 宏 -- 唯一日志发射点
//!
//! 用法:
//!   record!(INFO, Action::Start, "starting agent");
//!   record!(WARN, Action::Fail.with_outcome(EventOutcome::Failure), "tool failed");
//!   record!(INFO, Action::Invoke, "calling shell", "tool", "shell");
//!   record!(INFO, Action::Send, "LLM 请求", "model", "gpt-4o", "agent", "shadow");

/// 发射一条结构化日志事件
///
/// 可选的归因字段以 key-value 对形式追加:
///   record!(INFO, Action::Send, "LLM 请求", "model", "MiniMax-M2.7");
///   record!(INFO, Action::Invoke, "tool call", "tool", "shell", "agent", "shadow");
#[macro_export]
macro_rules! record {
    // 基本形式: record!(level, action, msg)
    ($level:ident, $action:expr, $msg:expr $(,)?) => {{
        let action = $action;
        $crate::__private::tracing::event!(
            target: "shadow_log_event",
            $crate::__private::tracing::Level::$level,
            shadow_action = %action.as_str(),
            message = %$msg,
        );
    }};
    // 带归因字段: record!(level, action, msg, "key1", val1, "key2", val2, ...)
    ($level:ident, $action:expr, $msg:expr, $($key:expr, $val:expr),+ $(,)?) => {{
        let action = $action;
        // 构建归因 JSON 字符串
        let mut attrs = ::std::collections::BTreeMap::new();
        $(
            attrs.insert($key.to_string(), $val.to_string());
        )+
        let attrs_json = $crate::__private::serde_json::to_string(&attrs).unwrap_or_default();
        $crate::__private::tracing::event!(
            target: "shadow_log_event",
            $crate::__private::tracing::Level::$level,
            shadow_action = %action.as_str(),
            message = %$msg,
            shadow_attrs = %attrs_json,
        );
    }};
}


#[macro_export]
macro_rules! scope {
    ($($key:ident : $value:expr),+ $(,)? => $body:expr) => {{
        use $crate::__private::tracing::Instrument;
        ($body).instrument($crate::__private::tracing::info_span!(
            target: "shadow_log_internal_scope",
            "shadow_scope",
            $($key = %($value)),+
        ))
    }};
}

/// 打开归因 span -- 从 Attributable 对象自动填充归因字段
#[macro_export]
macro_rules! attribution_span {
    ($thing:expr) => {{
        let thing = $thing;
        let role = shadow_core::Attributable::role(thing);
        let alias = shadow_core::Attributable::alias(thing);
        $crate::__private::tracing::info_span!(
            target: "shadow_log_attribution",
            "shadow_attribution",
            role = %role.family_str(),
            field = %role.attribution_field().unwrap_or(""),
            alias = %alias,
        )
    }};
}
