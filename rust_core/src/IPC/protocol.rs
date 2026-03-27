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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::flow_stat::{ProcessCategory, ProcessStats, ProcessStatus};

    #[test]
    fn serializes_stats_response() {
        let stats = vec![ProcessStats {
            pid: 1234,
            name: "chrome.exe".to_string(),
            category: ProcessCategory::User,
            status: ProcessStatus::Active,
            upload_speed: 12.5,
            download_speed: 34.5,
            total_upload: 100,
            total_download: 200,
        }];

        let response = IpcResponse::stats(stats);
        let json = serde_json::to_string(&response).expect("serialize stats response");

        assert!(json.contains("\"type\":\"stats\""));
        assert!(json.contains("\"name\":\"chrome.exe\""));
        assert!(json.contains("\"category\":\"user\""));
    }

    #[test]
    fn serializes_ack_without_extra_fields() {
        let json = serde_json::to_string(&IpcResponse::ack()).expect("serialize ack");
        assert_eq!(json, "{\"type\":\"ack\"}");
    }

    #[test]
    fn deserializes_request_with_optional_fields() {
        let json = r#"{"command":"set_limit","pid":42,"upload_limit":1024.5,"download_limit":2048.0}"#;
        let request: IpcRequest = serde_json::from_str(json).expect("deserialize request");

        assert_eq!(request.command, "set_limit");
        assert_eq!(request.pid, Some(42));
        assert_eq!(request.upload_limit, Some(1024.5));
        assert_eq!(request.download_limit, Some(2048.0));
    }
}
