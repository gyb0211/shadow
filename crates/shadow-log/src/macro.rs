
#[macro_export]
macro_rules! record {
    // 基本形式: record!(level, action, msg)
    ($level:ident, $event:expr, $msg:expr $(,)?) => {{
        let __event: $crate::Event = $event;
        $crate::__private::tracing::event!(
            target: "log_event",
            $crate::__private::tracing::Level::$level,
            sd_name = %__event.name,
            sd_action = %__event.action.as_str(),
            sd_outcome = %__event.outcome_str(),
            sd_category = %__event.category_str(),
            sd_attrs = %__event.attrs_str(),
            sd_has_duration = %__event.has_duration(),
            sd_duration_ms = %__event.duration_ms(),
            sd_file = %file!(),
            sd_line = %line!(),
            message = %$msg,
        );
    }};
}


#[macro_export]
macro_rules! scope {
    ($($key:ident : $value:expr),+ $(,)? => $body:expr) => {{
        use $crate::__private::tracing::Instrument;
        ($body).instrument($crate::__private::tracing::info_span!(
            target: "log_internal_scope",
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
            target: "log_attribution",
            "shadow_attribution",
            role = %role.family_str(),
            field = %role.attribution_field().unwrap_or(""),
            alias = %alias,
        )
    }};
}
