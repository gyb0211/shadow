use serde::{Deserialize, Serialize};
use crate::alias_agent::AgentAlias;
use crate::define_provider_ref;
use crate::providers::ChannelRef;


define_provider_ref!(PeerGroupName, "peer_groups");
define_provider_ref!(PeerUsername, "channels.peers");

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum OutputModality{
    #[default]
    Mirror,
    Voice,
    Text,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeerGroupConfig {
    pub channel: ChannelRef,
    pub agents: Vec<AgentAlias>,
    pub external_peers: Vec<PeerUsername>,
    pub ignore: Vec<PeerUsername>,
    pub output_modality: OutputModality,
}