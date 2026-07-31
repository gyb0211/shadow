use crate::agent::turn::events::StreamDelta;
use shadow_core::Observer;
use shadow_core::agent::TurnEvent;
use std::sync::mpsc::Sender;

pub(crate) struct TurnCtx<'a> {
    // pub observer: &'a dyn Observer,
    pub on_delta: Option<&'a Sender<StreamDelta>>,
    pub event_tx: Option<&'a Sender<TurnEvent>>,
    pub temperature: Option<f64>,
    pub turn_id: &'a str,
    pub agent_alias: Option<&'a str>,
}
