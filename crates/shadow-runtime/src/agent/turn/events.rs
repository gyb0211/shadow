

#[derive(Debug, Clone)]
pub enum StreamDelta {
    Text(String),
    Status(String),
}
