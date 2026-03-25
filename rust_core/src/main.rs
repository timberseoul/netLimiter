mod divert;
mod ipc;
mod limiter;
mod process;
mod stats;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use crossbeam_channel::bounded;
use log::info;

use divert::capture::{self, CAPTURE_DROP_COUNT, CAPTURE_TOTAL_COUNT};
use divert::parser::{Direction, ParsedPacket};
use ipc::pipe_server;
use process::pid_map::PidMapper;
use stats::flow_stat::FlowAggregator;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    info!("NetLimiter Core starting...");

    // Shared state — lock-free where possible
    let running = Arc::new(AtomicBool::new(true));
    let latest_stats: Arc<ArcSwap<Vec<stats::flow_stat::ProcessStats>>> =
        Arc::new(ArcSwap::from_pointee(Vec::new()));

    // Channel for parsed packets — small stack-only structs (~48 bytes each)
    let (tx, rx) = bounded::<ParsedPacket>(65536);

    // Start WinDivert capture (sniff all TCP and UDP traffic)
    // Parsing now happens in the capture thread — no per-packet heap alloc
    let capture_running = running.clone();
    let capture_handle = capture::start_capture("tcp or udp", tx, capture_running);

    // Start IPC named pipe server
    let ipc_stats = latest_stats.clone();
    let ipc_running = running.clone();
    let ipc_handle = pipe_server::start_pipe_server(ipc_stats, ipc_running);

    // Set up Ctrl+C handler
    let ctrlc_running = running.clone();
    ctrlc_handler(ctrlc_running);

    // Main processing loop
    let mut pid_mapper = PidMapper::new();
    let mut aggregator = FlowAggregator::new();
    let mut last_snapshot = std::time::Instant::now();

    info!("Processing loop started. Press Ctrl+C to stop.");

    let mut processed_count: u64 = 0;
    let mut last_log = std::time::Instant::now();

    while running.load(Ordering::Relaxed) {
        // Refresh cached TCP/UDP tables once per cycle, NOT per packet
        pid_mapper.maybe_refresh_tables();

        // ── Batch drain: pull ALL available packets from the channel ──
        let mut batch_count: u64 = 0;

        // First packet: block up to 50ms to avoid busy-spin
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(parsed) => {
                process_packet(&parsed, &mut pid_mapper, &mut aggregator);
                batch_count += 1;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // No packets, fall through to snapshot check
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                info!("Capture channel disconnected");
                break;
            }
        }

        // Drain remaining queued packets without blocking
        loop {
            match rx.try_recv() {
                Ok(parsed) => {
                    process_packet(&parsed, &mut pid_mapper, &mut aggregator);
                    batch_count += 1;
                }
                Err(_) => break,
            }
        }

        processed_count += batch_count;

        // Every 1 second: compute speeds, reset deltas, push to IPC, log stats
        if last_snapshot.elapsed() >= Duration::from_secs(1) {
            let stats = aggregator.snapshot();
            // ArcSwap store — lock-free, readers see new data on next load
            latest_stats.store(Arc::new(stats));
            last_snapshot = std::time::Instant::now();

            // Log throughput diagnostics every 5 seconds
            if last_log.elapsed() >= Duration::from_secs(5) {
                let total = CAPTURE_TOTAL_COUNT.load(Ordering::Relaxed);
                let dropped = CAPTURE_DROP_COUNT.load(Ordering::Relaxed);
                info!(
                    "[diag] captured={} processed={} dropped={} channel_pending={}",
                    total, processed_count, dropped, rx.len()
                );
                last_log = std::time::Instant::now();
            }
        }
    }

    info!("Shutting down...");

    // Wait for threads to finish
    let _ = capture_handle.join();
    let _ = ipc_handle.join();

    info!("NetLimiter Core stopped.");
}

/// Process a single parsed packet: PID lookup → record.
/// Parsing already happened in the capture thread.
#[inline]
fn process_packet(
    parsed: &ParsedPacket,
    pid_mapper: &mut PidMapper,
    aggregator: &mut FlowAggregator,
) {
    if let Some(pid) = pid_mapper.lookup_pid(
        parsed.src_ip,
        parsed.src_port,
        parsed.dst_ip,
        parsed.dst_port,
        parsed.protocol,
        parsed.direction == Direction::Outbound,
    ) {
        let name = pid_mapper.get_process_name(pid); // Arc<str> — O(1) clone
        let category = pid_mapper.get_process_category(pid);
        let (upload, download) = match parsed.direction {
            Direction::Outbound => (parsed.length as u64, 0u64),
            Direction::Inbound => (0u64, parsed.length as u64),
        };
        aggregator.record(pid, &name, category, upload, download);
    }
}

fn ctrlc_handler(running: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        unsafe {
            use windows::Win32::System::Console::{
                SetConsoleCtrlHandler, CTRL_C_EVENT, CTRL_BREAK_EVENT,
            };

            static mut RUNNING_PTR: Option<*const AtomicBool> = None;

            unsafe extern "system" fn handler(ctrl_type: u32) -> windows::Win32::Foundation::BOOL {
                if ctrl_type == CTRL_C_EVENT || ctrl_type == CTRL_BREAK_EVENT {
                    if let Some(ptr) = RUNNING_PTR {
                        let running = &*ptr;
                        running.store(false, Ordering::SeqCst);
                    }
                    return windows::Win32::Foundation::TRUE;
                }
                windows::Win32::Foundation::FALSE
            }

            RUNNING_PTR = Some(Arc::as_ptr(&running));
            // Keep the Arc alive — intentional leak for static handler lifetime
            std::mem::forget(running);

            let _ = SetConsoleCtrlHandler(Some(handler), true);
        }
    });
}
