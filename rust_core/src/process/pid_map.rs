use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use log::{error, debug};
use rustc_hash::FxHashMap;
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID,
    MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    MIB_UDP6ROW_OWNER_PID, MIB_UDP6TABLE_OWNER_PID, MIB_UDPROW_OWNER_PID,
    MIB_UDPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows::Win32::System::ProcessStatus::K32GetModuleFileNameExA;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::core::PWSTR;

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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct TcpV4Key {
    local_ip: u32,
    local_port: u16,
    remote_ip: u32,
    remote_port: u16,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct UdpV4Key {
    local_ip: u32,
    local_port: u16,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct TcpV6Key {
    local_ip: [u8; 16],
    local_port: u16,
    remote_ip: [u8; 16],
    remote_port: u16,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct UdpV6Key {
    local_ip: [u8; 16],
    local_port: u16,
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
    /// Cached map of service PIDs → display name (from SCM).
    /// Multiple services sharing a PID are joined with ", ".
    service_pids: HashMap<u32, String>,
    /// When we last refreshed the service PID set.
    service_pids_last_refresh: std::time::Instant,
    /// Cached IPv4 TCP index: (local_ip, local_port, remote_ip, remote_port) → PID.
    tcp_v4_index: FxHashMap<TcpV4Key, u32>,
    /// Cached IPv6 TCP index: (local_ip, local_port, remote_ip, remote_port) → PID.
    tcp_v6_index: FxHashMap<TcpV6Key, u32>,
    /// Cached IPv4 UDP index: (local_ip, local_port) → PID.
    udp_v4_index: FxHashMap<UdpV4Key, u32>,
    /// Cached IPv6 UDP index: (local_ip, local_port) → PID.
    udp_v6_index: FxHashMap<UdpV6Key, u32>,
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
            tcp_v4_index: FxHashMap::default(),
            tcp_v6_index: FxHashMap::default(),
            udp_v4_index: FxHashMap::default(),
            udp_v6_index: FxHashMap::default(),
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
        let (local_ip, local_port, remote_ip, remote_port) = if outbound {
            (src_ip, src_port, dst_ip, dst_port)
        } else {
            (dst_ip, dst_port, src_ip, src_port)
        };

        match (local_ip, remote_ip, protocol) {
            (IpAddr::V4(local_ip), IpAddr::V4(remote_ip), 6) => {
                self.lookup_tcp_v4_pid(local_ip, local_port, remote_ip, remote_port)
            }
            (IpAddr::V6(local_ip), IpAddr::V6(remote_ip), 6) => {
                self.lookup_tcp_v6_pid(local_ip, local_port, remote_ip, remote_port)
            }
            (IpAddr::V4(local_ip), IpAddr::V4(_), 17) => {
                self.lookup_udp_v4_pid(local_ip, local_port)
            }
            (IpAddr::V6(local_ip), IpAddr::V6(_), 17) => {
                self.lookup_udp_v6_pid(local_ip, local_port)
            }
            _ => None,
        }
    }

    /// Get the process name for a PID (cached).
    /// For service PIDs, returns the SCM display name instead of the exe name.
    /// Returns `Arc<str>` — clone is O(1) ref-count bump, no heap alloc.
    pub fn get_process_name(&mut self, pid: u32) -> Arc<str> {
        if let Some(name) = self.process_names.get(&pid) {
            return Arc::clone(name);
        }

        // For Windows services, prefer the SCM display name
        // (e.g. "DNS Client" instead of "svchost.exe")
        let (name, path) = get_process_name_and_path(pid);
        let display_name = if let Some(svc_name) = self.service_pids.get(&pid) {
            svc_name.clone()
        } else {
            name
        };

        let arc_name: Arc<str> = Arc::from(display_name.as_str());
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
        if self.service_pids.contains_key(&pid) {
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

    /// O(1) IPv4 TCP PID lookup from cached FxHashMap index.
    #[inline]
    fn lookup_tcp_v4_pid(
        &self,
        local_ip: Ipv4Addr,
        local_port: u16,
        remote_ip: Ipv4Addr,
        remote_port: u16,
    ) -> Option<u32> {
        let key = TcpV4Key {
            local_ip: u32::from(local_ip),
            local_port,
            remote_ip: u32::from(remote_ip),
            remote_port,
        };
        self.tcp_v4_index.get(&key).copied()
    }

    /// O(1) IPv6 TCP PID lookup from cached FxHashMap index.
    #[inline]
    fn lookup_tcp_v6_pid(
        &self,
        local_ip: Ipv6Addr,
        local_port: u16,
        remote_ip: Ipv6Addr,
        remote_port: u16,
    ) -> Option<u32> {
        let key = TcpV6Key {
            local_ip: local_ip.octets(),
            local_port,
            remote_ip: remote_ip.octets(),
            remote_port,
        };
        self.tcp_v6_index.get(&key).copied()
    }

    /// O(1) IPv4 UDP PID lookup from cached FxHashMap index.
    #[inline]
    fn lookup_udp_v4_pid(&self, local_ip: Ipv4Addr, local_port: u16) -> Option<u32> {
        let key = UdpV4Key {
            local_ip: u32::from(local_ip),
            local_port,
        };
        self.udp_v4_index.get(&key).copied()
    }

    /// O(1) IPv6 UDP PID lookup from cached FxHashMap index.
    #[inline]
    fn lookup_udp_v6_pid(&self, local_ip: Ipv6Addr, local_port: u16) -> Option<u32> {
        let key = UdpV6Key {
            local_ip: local_ip.octets(),
            local_port,
        };
        self.udp_v6_index.get(&key).copied()
    }

    /// Rebuild TCP/UDP index hashmaps from OS tables.
    /// Reuses existing allocations via clear() + re-insert.
    fn rebuild_table_indexes(&mut self) {
        // ── IPv4 TCP ──
        self.tcp_v4_index.clear();
        if let Some(rows) = get_tcp_table_v4(&mut self.table_query_buf) {
            self.tcp_v4_index.reserve(rows.len());
            for row in rows {
                let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                let remote_port = u16::from_be((row.dwRemotePort & 0xFFFF) as u16);
                let key = TcpV4Key {
                    local_ip: u32::from(ipv4_addr_from_row(row.dwLocalAddr)),
                    local_port,
                    remote_ip: u32::from(ipv4_addr_from_row(row.dwRemoteAddr)),
                    remote_port,
                };
                self.tcp_v4_index.insert(key, row.dwOwningPid);
            }
        }

        // ── IPv6 TCP ──
        self.tcp_v6_index.clear();
        if let Some(rows) = get_tcp_table_v6(&mut self.table_query_buf) {
            self.tcp_v6_index.reserve(rows.len());
            for row in rows {
                let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                let remote_port = u16::from_be((row.dwRemotePort & 0xFFFF) as u16);
                let key = TcpV6Key {
                    local_ip: row.ucLocalAddr,
                    local_port,
                    remote_ip: row.ucRemoteAddr,
                    remote_port,
                };
                self.tcp_v6_index.insert(key, row.dwOwningPid);
            }
        }

        // ── IPv4 UDP ──
        self.udp_v4_index.clear();
        if let Some(rows) = get_udp_table_v4(&mut self.table_query_buf) {
            self.udp_v4_index.reserve(rows.len());
            for row in rows {
                let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                let key = UdpV4Key {
                    local_ip: u32::from(ipv4_addr_from_row(row.dwLocalAddr)),
                    local_port,
                };
                self.udp_v4_index.insert(key, row.dwOwningPid);
            }
        }

        // ── IPv6 UDP ──
        self.udp_v6_index.clear();
        if let Some(rows) = get_udp_table_v6(&mut self.table_query_buf) {
            self.udp_v6_index.reserve(rows.len());
            for row in rows {
                let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                let key = UdpV6Key {
                    local_ip: row.ucLocalAddr,
                    local_port,
                };
                self.udp_v6_index.insert(key, row.dwOwningPid);
            }
        }
    }
}

fn ipv4_addr_from_row(addr: u32) -> Ipv4Addr {
    Ipv4Addr::from(u32::from_be(addr))
}

/// Retrieve the IPv4 TCP connection table with owning PIDs.
/// Reuses the provided buffer to avoid per-call allocation.
fn get_tcp_table_v4(buf: &mut Vec<u8>) -> Option<&[MIB_TCPROW_OWNER_PID]> {
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

/// Retrieve the IPv6 TCP connection table with owning PIDs.
/// Reuses the provided buffer to avoid per-call allocation.
fn get_tcp_table_v6(buf: &mut Vec<u8>) -> Option<&[MIB_TCP6ROW_OWNER_PID]> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        buf.resize(size as usize, 0);
        let ret = GetExtendedTcpTable(
            Some(buf.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        if ret != NO_ERROR.0 {
            error!("GetExtendedTcpTable(AF_INET6) failed with error: {}", ret);
            return None;
        }

        let table = &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
        let count = table.dwNumEntries as usize;
        let rows_ptr = table.table.as_ptr();
        Some(std::slice::from_raw_parts(rows_ptr, count))
    }
}

/// Retrieve the IPv4 UDP endpoint table with owning PIDs.
/// Reuses the provided buffer to avoid per-call allocation.
fn get_udp_table_v4(buf: &mut Vec<u8>) -> Option<&[MIB_UDPROW_OWNER_PID]> {
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

/// Retrieve the IPv6 UDP endpoint table with owning PIDs.
/// Reuses the provided buffer to avoid per-call allocation.
fn get_udp_table_v6(buf: &mut Vec<u8>) -> Option<&[MIB_UDP6ROW_OWNER_PID]> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedUdpTable(
            None,
            &mut size,
            false,
            AF_INET6.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );

        buf.resize(size as usize, 0);
        let ret = GetExtendedUdpTable(
            Some(buf.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET6.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );

        if ret != NO_ERROR.0 {
            error!("GetExtendedUdpTable(AF_INET6) failed with error: {}", ret);
            return None;
        }

        let table = &*(buf.as_ptr() as *const MIB_UDP6TABLE_OWNER_PID);
        let count = table.dwNumEntries as usize;
        let rows_ptr = table.table.as_ptr();
        Some(std::slice::from_raw_parts(rows_ptr, count))
    }
}

/// Get the process executable name and full path by PID.
///
/// Strategy:
/// 1. Try `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ` + `K32GetModuleFileNameExA`
///    — works for user processes.
/// 2. Fallback: `PROCESS_QUERY_LIMITED_INFORMATION` + `QueryFullProcessImageNameW`
///    — works for service/system processes running as SYSTEM/LOCAL SERVICE/etc.
fn get_process_name_and_path(pid: u32) -> (String, String) {
    if pid == 0 {
        return ("System Idle".to_string(), "".to_string());
    }
    if pid == 4 {
        return ("System".to_string(), "C:\\Windows\\System32\\ntoskrnl.exe".to_string());
    }

    unsafe {
        // ── Attempt 1: full access (user processes) ──
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) {
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
                return (name, full_path);
            }
        }

        // ── Attempt 2: limited access (service/system processes) ──
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 260];
            let mut size = buf.len() as u32;
            let result = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = windows::Win32::Foundation::CloseHandle(handle);

            if result.is_ok() && size > 0 {
                let full_path = String::from_utf16_lossy(&buf[..size as usize]);
                let name = full_path
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&full_path)
                    .to_string();
                return (name, full_path);
            }
        }

        (format!("PID:{}", pid), String::new())
    }
}

// ─────────────────────────────────────────────────────
//  SCM: Query Service Control Manager for service PIDs
// ─────────────────────────────────────────────────────

/// Query the Windows Service Control Manager and return a map of
/// PID → service display name for all running services.
/// Multiple services sharing a PID (common with svchost.exe) are joined.
fn query_service_pids() -> HashMap<u32, String> {
    use windows::Win32::System::Services::{
        OpenSCManagerW, EnumServicesStatusExW, CloseServiceHandle,
        SC_MANAGER_ENUMERATE_SERVICE, SC_ENUM_PROCESS_INFO,
        SERVICE_WIN32, SERVICE_ACTIVE,
        ENUM_SERVICE_STATUS_PROCESSW,
    };

    let mut pid_names: HashMap<u32, String> = HashMap::new();

    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_ENUMERATE_SERVICE);
        let scm = match scm {
            Ok(h) => h,
            Err(e) => {
                debug!("Failed to open SCM: {}", e);
                return pid_names;
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
            return pid_names;
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
                if pid == 0 {
                    continue;
                }

                // Prefer the display name; fall back to service name
                let display = if !entry.lpDisplayName.is_null() {
                    entry.lpDisplayName.to_string().unwrap_or_default()
                } else if !entry.lpServiceName.is_null() {
                    entry.lpServiceName.to_string().unwrap_or_default()
                } else {
                    continue;
                };

                if display.is_empty() {
                    continue;
                }

                // Multiple services can share a PID (svchost grouping)
                pid_names.entry(pid)
                    .and_modify(|existing| {
                        // Limit concatenated length to keep UI readable
                        if existing.len() < 80 {
                            existing.push_str(", ");
                            existing.push_str(&display);
                        }
                    })
                    .or_insert(display);
            }
        }

        let _ = CloseServiceHandle(scm);
    }

    debug!("SCM: found {} service PIDs", pid_names.len());
    pid_names
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
