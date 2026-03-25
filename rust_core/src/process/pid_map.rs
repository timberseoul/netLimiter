use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

use log::{error, debug};
use rustc_hash::FxHashMap;
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, MIB_UDPROW_OWNER_PID, MIB_UDPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::AF_INET;
use windows::Win32::System::ProcessStatus::K32GetModuleFileNameExA;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

use crate::stats::flow_stat::ProcessCategory;

/// Connection key: (local_ip, local_port, remote_ip, remote_port, protocol)
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ConnectionKey {
    pub local_ip: IpAddr,
    pub local_port: u16,
    pub remote_ip: IpAddr,
    pub remote_port: u16,
    pub protocol: u8,
}

/// Caches PID → process name, path, and category mappings.
/// TCP/UDP connection tables are cached and refreshed periodically
/// instead of querying the OS for every single packet.
pub struct PidMapper {
    /// PID → process name (Arc<str> for zero-cost cloning on hot path)
    process_names: HashMap<u32, Arc<str>>,
    /// PID → full executable path
    process_paths: HashMap<u32, String>,
    /// PID → classification (User/System/Service/Unknown)
    process_categories: HashMap<u32, ProcessCategory>,
    /// When the PID → name/path/category caches were last cleared.
    /// Periodic clearing handles OS PID reuse.
    pid_cache_last_clear: std::time::Instant,
    /// Cached set of PIDs that are Windows services (from SCM).
    service_pids: HashSet<u32>,
    /// When we last refreshed the service PID set.
    service_pids_last_refresh: std::time::Instant,
    /// Cached TCP index: (local_port, remote_port) → PID.
    /// FxHashMap for fast integer hashing (no DoS protection needed).
    tcp_index: FxHashMap<u32, u32>,
    /// Cached UDP index: local_port → PID.
    udp_index: FxHashMap<u16, u32>,
    /// When we last refreshed the TCP/UDP table indexes.
    table_cache_last_refresh: std::time::Instant,
    /// Reusable buffer for OS table queries (avoids per-refresh allocation).
    table_query_buf: Vec<u8>,
}

impl PidMapper {
    /// How often to refresh the cached TCP/UDP connection table indexes.
    const TABLE_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
    /// How often to clear the PID → name/path/category caches
    /// to handle OS PID reuse (a PID assigned to a new process).
    const PID_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

    pub fn new() -> Self {
        let service_pids = query_service_pids();
        let mut mapper = Self {
            process_names: HashMap::new(),
            process_paths: HashMap::new(),
            process_categories: HashMap::new(),
            pid_cache_last_clear: std::time::Instant::now(),
            service_pids,
            service_pids_last_refresh: std::time::Instant::now(),
            tcp_index: FxHashMap::default(),
            udp_index: FxHashMap::default(),
            table_cache_last_refresh: std::time::Instant::now(),
            table_query_buf: Vec::with_capacity(64 * 1024),
        };
        mapper.rebuild_table_indexes();
        mapper
    }

    /// Refresh cached TCP/UDP table indexes if the interval has elapsed.
    /// Also periodically clears process name/path/category caches to
    /// handle PID reuse by the OS.
    /// Call once per batch cycle, NOT per-packet.
    pub fn maybe_refresh_tables(&mut self) {
        if self.table_cache_last_refresh.elapsed() >= Self::TABLE_REFRESH_INTERVAL {
            self.rebuild_table_indexes();
            self.table_cache_last_refresh = std::time::Instant::now();
        }
        // Periodically invalidate PID → name/path/category caches
        // so that reused PIDs get fresh lookups.
        if self.pid_cache_last_clear.elapsed() >= Self::PID_CACHE_TTL {
            self.process_names.clear();
            self.process_paths.clear();
            self.process_categories.clear();
            self.pid_cache_last_clear = std::time::Instant::now();
            debug!("PID cache cleared (TTL={}s)", Self::PID_CACHE_TTL.as_secs());
        }
    }

    /// Look up PID by matching (src_ip, src_port, dst_ip, dst_port, protocol)
    /// against the OS TCP/UDP connection tables.
    pub fn lookup_pid(
        &mut self,
        src_ip: IpAddr,
        src_port: u16,
        dst_ip: IpAddr,
        dst_port: u16,
        protocol: u8,
        outbound: bool,
    ) -> Option<u32> {
        let (local_port, _remote_ip, remote_port) = if outbound {
            (src_port, dst_ip, dst_port)
        } else {
            (dst_port, src_ip, src_port)
        };

        match protocol {
            6 => self.lookup_tcp_pid(local_port, remote_port),
            17 => self.lookup_udp_pid(local_port),
            _ => None,
        }
    }

    /// Get the process name for a PID (cached).
    /// Returns `Arc<str>` — clone is O(1) ref-count bump, no heap alloc.
    pub fn get_process_name(&mut self, pid: u32) -> Arc<str> {
        if let Some(name) = self.process_names.get(&pid) {
            return Arc::clone(name);
        }

        let (name, path) = get_process_name_and_path(pid);
        let arc_name: Arc<str> = Arc::from(name.as_str());
        self.process_names.insert(pid, Arc::clone(&arc_name));
        self.process_paths.insert(pid, path);
        arc_name
    }

    /// Classify a process into a category.
    /// Priority: SCM service check → path-based system check → user.
    pub fn get_process_category(&mut self, pid: u32) -> ProcessCategory {
        // Return cached category if available
        if let Some(&cat) = self.process_categories.get(&pid) {
            return cat;
        }

        let category = self.classify_pid(pid);
        self.process_categories.insert(pid, category);
        category
    }

    /// Core classification logic.
    fn classify_pid(&mut self, pid: u32) -> ProcessCategory {
        // PID 0 and 4 are always system
        if pid == 0 || pid == 4 {
            return ProcessCategory::System;
        }

        // Refresh service PID cache every 30 seconds
        if self.service_pids_last_refresh.elapsed() > std::time::Duration::from_secs(30) {
            self.service_pids = query_service_pids();
            self.service_pids_last_refresh = std::time::Instant::now();
        }

        // 1) Check SCM: is this PID a Windows service?
        if self.service_pids.contains(&pid) {
            return ProcessCategory::Service;
        }

        // Ensure path is cached
        if !self.process_paths.contains_key(&pid) {
            let _ = self.get_process_name(pid);
        }

        let path = match self.process_paths.get(&pid) {
            Some(p) => p.to_lowercase(),
            None => return ProcessCategory::Unknown,
        };

        // 2) Check path: under Windows\ directories → system
        classify_by_path(&path)
    }

    /// O(1) TCP PID lookup from cached FxHashMap index.
    /// Key packs (local_port, remote_port) into a single u32.
    #[inline]
    fn lookup_tcp_pid(&self, local_port: u16, remote_port: u16) -> Option<u32> {
        let key = (local_port as u32) << 16 | remote_port as u32;
        self.tcp_index.get(&key).copied()
    }

    /// O(1) UDP PID lookup from cached FxHashMap index.
    #[inline]
    fn lookup_udp_pid(&self, local_port: u16) -> Option<u32> {
        self.udp_index.get(&local_port).copied()
    }

    /// Rebuild TCP/UDP index hashmaps from OS tables.
    /// Reuses existing allocations via clear() + re-insert.
    fn rebuild_table_indexes(&mut self) {
        // ── TCP ──
        self.tcp_index.clear();
        if let Some(rows) = get_tcp_table(&mut self.table_query_buf) {
            self.tcp_index.reserve(rows.len());
            for row in rows {
                let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                let remote_port = u16::from_be((row.dwRemotePort & 0xFFFF) as u16);
                let key = (local_port as u32) << 16 | remote_port as u32;
                self.tcp_index.insert(key, row.dwOwningPid);
            }
        }

        // ── UDP ──
        self.udp_index.clear();
        if let Some(rows) = get_udp_table(&mut self.table_query_buf) {
            self.udp_index.reserve(rows.len());
            for row in rows {
                let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                self.udp_index.insert(local_port, row.dwOwningPid);
            }
        }
    }
}

/// Retrieve the TCP connection table with owning PIDs.
/// Reuses the provided buffer to avoid per-call allocation.
fn get_tcp_table(buf: &mut Vec<u8>) -> Option<&[MIB_TCPROW_OWNER_PID]> {
    unsafe {
        let mut size: u32 = 0;
        // First call to get required buffer size
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        buf.resize(size as usize, 0);
        let ret = GetExtendedTcpTable(
            Some(buf.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        if ret != NO_ERROR.0 {
            error!("GetExtendedTcpTable failed with error: {}", ret);
            return None;
        }

        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let count = table.dwNumEntries as usize;
        let rows_ptr = table.table.as_ptr();
        Some(std::slice::from_raw_parts(rows_ptr, count))
    }
}

/// Retrieve the UDP endpoint table with owning PIDs.
/// Reuses the provided buffer to avoid per-call allocation.
fn get_udp_table(buf: &mut Vec<u8>) -> Option<&[MIB_UDPROW_OWNER_PID]> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedUdpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );

        buf.resize(size as usize, 0);
        let ret = GetExtendedUdpTable(
            Some(buf.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );

        if ret != NO_ERROR.0 {
            error!("GetExtendedUdpTable failed with error: {}", ret);
            return None;
        }

        let table = &*(buf.as_ptr() as *const MIB_UDPTABLE_OWNER_PID);
        let count = table.dwNumEntries as usize;
        let rows_ptr = table.table.as_ptr();
        Some(std::slice::from_raw_parts(rows_ptr, count))
    }
}

/// Get the process executable name and full path by PID.
fn get_process_name_and_path(pid: u32) -> (String, String) {
    if pid == 0 {
        return ("System Idle".to_string(), "".to_string());
    }
    if pid == 4 {
        return ("System".to_string(), "C:\\Windows\\System32\\ntoskrnl.exe".to_string());
    }

    unsafe {
        let process = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid);
        match process {
            Ok(handle) => {
                let mut buf = [0u8; 260];
                let len = K32GetModuleFileNameExA(handle, None, &mut buf);
                let _ = windows::Win32::Foundation::CloseHandle(handle);

                if len > 0 {
                    let full_path = String::from_utf8_lossy(&buf[..len as usize]).to_string();
                    let name = full_path
                        .rsplit('\\')
                        .next()
                        .unwrap_or(&full_path)
                        .to_string();
                    (name, full_path)
                } else {
                    (format!("PID:{}", pid), String::new())
                }
            }
            Err(_) => (format!("PID:{}", pid), String::new()),
        }
    }
}

// ─────────────────────────────────────────────────────
//  SCM: Query Service Control Manager for service PIDs
// ─────────────────────────────────────────────────────

/// Query the Windows Service Control Manager and return a set of PIDs
/// that belong to running services.
fn query_service_pids() -> HashSet<u32> {
    use windows::Win32::System::Services::{
        OpenSCManagerW, EnumServicesStatusExW, CloseServiceHandle,
        SC_MANAGER_ENUMERATE_SERVICE, SC_ENUM_PROCESS_INFO,
        SERVICE_WIN32, SERVICE_ACTIVE,
        ENUM_SERVICE_STATUS_PROCESSW,
    };

    let mut pids = HashSet::new();

    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_ENUMERATE_SERVICE);
        let scm = match scm {
            Ok(h) => h,
            Err(e) => {
                debug!("Failed to open SCM: {}", e);
                return pids;
            }
        };

        // First call: get required buffer size
        let mut bytes_needed: u32 = 0;
        let mut services_returned: u32 = 0;
        let mut resume_handle: u32 = 0;

        let _ = EnumServicesStatusExW(
            scm,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_ACTIVE,
            None,
            &mut bytes_needed,
            &mut services_returned,
            Some(&mut resume_handle),
            None,
        );

        if bytes_needed == 0 {
            let _ = CloseServiceHandle(scm);
            return pids;
        }

        // Allocate buffer and call again
        let mut buffer = vec![0u8; bytes_needed as usize];
        resume_handle = 0;

        let ok = EnumServicesStatusExW(
            scm,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_ACTIVE,
            Some(&mut buffer),
            &mut bytes_needed,
            &mut services_returned,
            Some(&mut resume_handle),
            None,
        );

        if ok.is_ok() && services_returned > 0 {
            let entries = std::slice::from_raw_parts(
                buffer.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW,
                services_returned as usize,
            );
            for entry in entries {
                let pid = entry.ServiceStatusProcess.dwProcessId;
                if pid != 0 {
                    pids.insert(pid);
                }
            }
        }

        let _ = CloseServiceHandle(scm);
    }

    debug!("SCM: found {} service PIDs", pids.len());
    pids
}

// ─────────────────────────────────────────────────────
//  Path-based classification
// ─────────────────────────────────────────────────────

/// Classify process by executable path (lowercase).
/// Called only AFTER SCM check, so anything reaching here is not a known service.
fn classify_by_path(path_lower: &str) -> ProcessCategory {
    if path_lower.is_empty() {
        return ProcessCategory::Unknown;
    }

    // Anything under \Windows\ is a system process
    if path_lower.contains("\\windows\\system32\\")
        || path_lower.contains("\\windows\\syswow64\\")
        || path_lower.contains("\\windows\\systemapps\\")
        || path_lower.contains("\\windows\\explorer.exe")
    {
        return ProcessCategory::System;
    }

    // General \Windows\ catch-all (e.g. \Windows\ImmersiveControlPanel\, etc.)
    if path_lower.contains("\\windows\\") {
        return ProcessCategory::System;
    }

    // Everything else is a user process
    ProcessCategory::User
}
