

#[derive(Debug, Clone)]
pub enum TurnEvent {
    Chunk{delta: String},
    Think{delta: String},
    ToolCall{
        id:String,
        name:String,
        args:serde_json::Value,
    }
}