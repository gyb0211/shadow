use std::sync::{Arc, Mutex};

// return => (model_provider, model)
pub type ModelSwitchCallback = Arc<Mutex<Option<(String,String)>>>;