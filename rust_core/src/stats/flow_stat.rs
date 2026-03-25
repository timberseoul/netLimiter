use std::sync::Arc;
use std::time::Instant;

use rustc_hash::FxHashMap;
use serde::Serialize;

/// Process category for grouping in the UI.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub enum ProcessCategory {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "service")]
    Service,
    #[serde(rename = "unknown")]
    Unknown,
}

/// Process activity status.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum ProcessStatus {
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "inactive")]
    Inactive,
}

/// Per-process state cache entry.
/// This is the global state for one PID — never deleted, only updated.
#[derive(Debug, Clone)]
struct ProcessEntry {
    pid: u32,
    name: Arc<str>,
    category: ProcessCategory,
    /// Bytes accumulated since last speed reset (used to compute speed)
    upload_delta: u64,
    download_delta: u64,
    /// Lifetime cumulative bytes
    total_upload: u64,
    total_download: u64,
    /// When this process last had any traffic
    last_seen: Instant,
}

/// Snapshot sent to UI via IPC — serialised to JSON.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessStats {
    pub pid: u32,
    pub name: String,
    pub category: ProcessCategory,
    pub status: ProcessStatus,
    pub upload_speed: f64,
    pub download_speed: f64,
    pub total_upload: u64,
    pub total_download: u64,
}

/// The global state cache:  packet → update cache → UI reads cache.
pub struct FlowAggregator {
    entries: FxHashMap<u32, ProcessEntry>,
    last_reset: Instant,
}

impl FlowAggregator {
    pub fn new() -> Self {
        Self {
            entries: FxHashMap::default(),
            last_reset: Instant::now(),
        }
    }

    // ──────────────────────────────────────────
    //  Called on every captured packet
    // ──────────────────────────────────────────

    /// Accumulate bytes for a PID.  Never removes entries.
    /// `name` is `Arc<str>` — clone is O(1) ref-count bump.
    pub fn record(
        &mut self,
        pid: u32,
        name: &Arc<str>,
        category: ProcessCategory,
        upload: u64,
        download: u64,
    ) {
        let now = Instant::now();
        let entry = self.entries.entry(pid).or_insert_with(|| ProcessEntry {
            pid,
            name: Arc::clone(name),
            category,
            upload_delta: 0,
            download_delta: 0,
            total_upload: 0,
            total_download: 0,
            last_seen: now,
        });

        // Keep name / category fresh (Arc pointer comparison first, cheap)
        if !Arc::ptr_eq(&entry.name, name) && *entry.name != **name {
            entry.name = Arc::clone(name);
        }
        entry.category = category;

        // Accumulate delta (for speed calc) AND total (lifetime)
        entry.upload_delta += upload;
        entry.download_delta += download;
        entry.total_upload += upload;
        entry.total_download += download;

        // Mark active
        entry.last_seen = now;
    }

    // ──────────────────────────────────────────
    //  Called once per refresh cycle (e.g. 1 s)
    // ──────────────────────────────────────────

    /// Compute speeds from accumulated deltas, then reset deltas to 0.
    /// Returns a snapshot of ALL historically-seen processes.
    pub fn snapshot(&mut self) -> Vec<ProcessStats> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_reset).as_secs_f64();
        let elapsed = if elapsed < 0.001 { 0.001 } else { elapsed };

        let inactive_threshold = std::time::Duration::from_secs(30);

        let stats: Vec<ProcessStats> = self
            .entries
            .values()
            .map(|e| {
                let status = if now.duration_since(e.last_seen) > inactive_threshold {
                    ProcessStatus::Inactive
                } else {
                    ProcessStatus::Active
                };
                ProcessStats {
                    pid: e.pid,
                    name: e.name.to_string(),
                    category: e.category,
                    status,
                    upload_speed: e.upload_delta as f64 / elapsed,
                    download_speed: e.download_delta as f64 / elapsed,
                    total_upload: e.total_upload,
                    total_download: e.total_download,
                }
            })
            .collect();

        // Reset deltas — but NEVER delete entries
        for e in self.entries.values_mut() {
            e.upload_delta = 0;
            e.download_delta = 0;
        }

        self.last_reset = now;
        stats
    }
}
