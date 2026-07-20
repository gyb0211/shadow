mod agent;
pub mod loop_;

pub use loop_::*;
use shadow_core::{Attributable, Role};

pub struct AgentAttribution<'a>(pub &'a str);

impl Attributable for AgentAttribution<'_> {
    fn role(&self) -> Role {
        Role::Agent
    }

    fn alias(&self) -> &str {
        self.0
    }
}
