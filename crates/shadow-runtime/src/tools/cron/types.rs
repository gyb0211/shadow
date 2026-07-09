
use serde::{Deserialize, Serialize};




#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionTarget{
    #[default]
    Isolated,
    Main
}


impl SessionTarget {
    pub fn as_str(&self) -> &'static str{
        match self {
            SessionTarget::Isolated => "isolated",
            SessionTarget::Main => "main",
        }
    }

    pub fn parse(raw: &str) -> Self{
        if raw.eq_ignore_ascii_case("main") {
            SessionTarget::Main
        }else{
            SessionTarget::Isolated
        }
    }
}