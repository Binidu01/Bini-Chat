
// message.rs
use serde::{Serialize, Deserialize};
use chrono::Utc;

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    pub username: String,
    pub text: String,
    pub room: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<bool>,
    pub timestamp: String,
}

impl Message {
    pub fn new(username: &str, room: &str, text: &str) -> Self {
        Self {
            username: username.to_string(),
            text: text.to_string(),
            room: room.to_string(),
            admin: None,
            timestamp: Utc::now().to_rfc3339(),
        }
    }
}