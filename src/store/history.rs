use serde::{Deserialize, Serialize};

/// History event record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub id: Option<i64>,
    pub timestamp: i64,
    pub action: String, // "install", "switch", "remove", "rollback"
    pub package: String,
    pub version_from: Option<String>,
    pub version_to: Option<String>,
    pub success: bool,
}
