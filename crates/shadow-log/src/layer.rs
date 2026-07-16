//! LogCaptureLayer -- 捕获 record! 事件, 组装 LogEvent, 写入
//!
//! 参考 ZeroClaw layer.rs:
//! - 捕获 target=shadow_log_event 的事件
//! - 从 visit 中提取 action, message, shadow_attrs (归因字段)
//! - 组装 LogEvent 后调 record_event

use crate::event::{Action, Attribution, EventCategory, LogEvent, Severity};
use crate::writer::record_event;
use std::fmt::Debug;
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::{LookupSpan, SpanRef};
use serde_json::map::Map as JsonMap;

pub struct LogCaptureLayer;

const SHADOW_ROLE: &str = "sd_role";

const SHADOW_ATTRIBUTION_SPAN: &str = "shadow_log_attribution";
const SHADOW_SCOPE_SPAN: &str = "shadow_log_scope";

impl<S> Layer<S> for LogCaptureLayer
where
    S: Subscriber + for<'l> LookupSpan<'l>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let target = attrs.metadata().target();
        let Some(span) = ctx.span(id) else {
            return;
        };

        // 只捕获 record! 宏的事件
        if target == SHADOW_ATTRIBUTION_SPAN {
            let mut v = AttributionSpanCollector::default();
            attrs.record(&mut v);
            let mut attribution = Attribution::default();
            let default_category = v.default_category.as_deref().and_then(EventCategory::parse);
            v.apply_info(&mut attribution);

            let mut exts = span.extensions_mut();
            exts.insert(attribution);

            if let Some(cat) = default_category {
                exts.insert(SpanCategory(cat))
            }
            return;
        }

        if target == SHADOW_SCOPE_SPAN {
            let mut v = ScopeSpanCollector::default();
            attrs.record(&mut v);
            v.install(span);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return; };
        let target = span.metadata().target();
        if target == SHADOW_ATTRIBUTION_SPAN {
            let mut v = AttributionSpanCollector::default();
            values.record(&mut v);
            let mut attribution = Attribution::default();
            v.apply_info(&mut attribution);
            let mut exts = span.extensions_mut();
            if let Some(exist) = exts.get_mut::<Attribution>() {
                exist.merge_from(&attribution)
            }else{
                exts.insert(attribution);
            }
            return;
        }

        if target == SHADOW_SCOPE_SPAN {
            let mut v = ScopeSpanCollector::default();
            values.record(&mut v);
            v.install(span);
        }
    }

}

#[derive(Default)]
struct AttributionSpanCollector {
    role_family: Option<String>,
    role_type: Option<String>,
    attribution_field: Option<String>,
    composite_prefix: Option<String>,
    default_category: Option<String>,
    alias: Option<String>,
}

impl AttributionSpanCollector {
    fn apply_info(&self, attr: &mut Attribution) {
        let Some(alias) = self.alias.as_deref().filter(|s| !s.is_empty()) else {
            return;
        };
        if let Some(prefix) = self.composite_prefix.as_deref().filter(|s| !s.is_empty()) {
            let ty = self.role_type.as_deref().unwrap_or("");
            if !ty.is_empty() {
                attr.set_composite(prefix, &format!("{ty}.{alias}"));
            } else {
                attr.set_composite(prefix, alias);
            }
        } else if let Some(field) = self.attribution_field.as_deref().filter(|s| !s.is_empty()) {
            attr.set(field, alias);
        }

        if let Some(family) = self.role_family.as_deref().filter(|s| !s.is_empty()) {
            attr.set(SHADOW_ROLE, family);
        }
    }
}

impl Visit for AttributionSpanCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        todo!()
    }
}

#[derive(Clone, Copy)]
struct SpanCategory(EventCategory);

#[derive(Default)]
struct ScopeSpanCollector {
    category: Option<String>,
    attribution: Attribution,
    extra: JsonMap<String, Value>,

}
#[derive(Default)]
struct ScopeExtra {
    extra: JsonMap<String, Value>,
}


impl ScopeSpanCollector {
    fn install<'a>(&self, span: SpanRef<'a, impl Subscriber + for<'l> LookupSpan<'l>>, ) {
       if !self.attribution.fields.is_empty() || self.attribution.duration_ms.is_some() {
           let mut exts = span.extensions_mut();
           if let Some(exist) = exts.get_mut::<Attribution>(){
               exist.merge_from(&self.attribution)
           }else{
               exts.insert(self.attribution);
           }
       }

        if !self.extra.is_empty() {
            let mut exts = span.extensions_mut();
            if let Some(exist) = exts.get_mut::<ScopeExtra>(){
                for (k,v) in self.extra {
                    exist.extra.entry(k).or_insert(v);
                }
            }else{
                exts.insert(ScopeExtra{extra: self.extra});
            }

        }

        if let Some(cat) = self.category.as_deref().and_then(EventCategory::parse){
            span.extensions_mut().insert(SpanCategory(cat))
        }



    }
}

#[derive(Default)]
struct EventVisitor {
    action: Option<String>,
    message: Option<String>,
    /// record! 宏的归因字段 (JSON 字符串)
    attrs: Option<String>,
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "shadow_action" => self.action = Some(value.to_string()),
            "message" => self.message = Some(value.to_string()),
            "shadow_attrs" => self.attrs = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{value:?}");
        let s = s.trim_matches('"').to_string();
        match field.name() {
            "shadow_action" => self.action = Some(s),
            "message" => self.message = Some(s),
            "shadow_attrs" => self.attrs = Some(s),
            _ => {}
        }
    }
}
