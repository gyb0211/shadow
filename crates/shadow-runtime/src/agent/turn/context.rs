use std::sync::mpsc::Sender;
use shadow_core::agent::TurnEvent;
use shadow_core::Observer;
use crate::agent::turn::events::StreamDelta;

pub(crate) struct TurnCtx<'a>{
    pub observer: &'a dyn Observer,
    pub on_delta:Option<&'a Sender<StreamDelta>>,
    pub event_tx:Option<&'a Sender<TurnEvent>>,
    pub temperature: Option<f64>,
    pub turn_id: &'a str,
    pub agent_alias: &'a str,
}
