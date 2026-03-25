use serde::{Deserialize, Serialize};

use crate::stats::flow_stat::ProcessStats;

/// Message sent from Rust → Go over the named pipe.
#[derive(Debug, Clone, Serialize)]
pub struct IpcResponse {
    /// "stats" | "error" | "ack"
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    /// Present when type == "stats"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<ProcessStats>>,
    /// Present when type == "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Message sent from Go → Rust over the named pipe.
#[derive(Debug, Clone, Deserialize)]
pub struct IpcRequest {
    /// "get_stats" | "set_limit" | "ping"
    pub command: String,
    /// For set_limit: target PID
    #[serde(default)]
    pub pid: Option<u32>,
    /// For set_limit: upload limit in bytes/sec (0 = unlimited)
    #[serde(default)]
    pub upload_limit: Option<f64>,
    /// For set_limit: download limit in bytes/sec (0 = unlimited)
    #[serde(default)]
    pub download_limit: Option<f64>,
}

impl IpcResponse {
    pub fn stats(data: Vec<ProcessStats>) -> Self {
        Self {
            msg_type: "stats",
            data: Some(data),
            error: None,
        }
    }

    /// Build a stats response from an Arc-borrowed slice (avoids clone).
    pub fn stats_ref(data: &[ProcessStats]) -> Self {
        Self {
            msg_type: "stats",
            data: Some(data.to_vec()),
            error: None,
        }
    }

    pub fn error(msg: &str) -> Self {
        Self {
            msg_type: "error",
            data: None,
            error: Some(msg.to_string()),
        }
    }

    pub fn ack() -> Self {
        Self {
            msg_type: "ack",
            data: None,
            error: None,
        }
    }
}
