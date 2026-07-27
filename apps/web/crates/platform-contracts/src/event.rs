use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Generic event envelope for platform-generated events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformEvent<T> {
    pub id: Uuid,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub payload: T,
    pub trace_id: Option<String>,
}

impl<T: Serialize> PlatformEvent<T> {
    pub fn new(event_type: impl Into<String>, payload: T) -> Self {
        Self {
            id: Uuid::now_v7(),
            event_type: event_type.into(),
            timestamp: Utc::now(),
            payload,
            trace_id: None,
        }
    }
}
