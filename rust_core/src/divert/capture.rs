use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;

use crossbeam_channel::Sender;
use log::{error, info};
use windivert::prelude::*;

use super::parser::{self, ParsedPacket};

/// Starts the WinDivert capture loop on a background thread.
/// Packets are parsed in this thread and only the small, stack-allocated
/// ParsedPacket (~48 bytes) is sent through the channel, eliminating
/// per-packet Vec<u8> heap allocations.
/// `running` controls the loop lifetime.
/// Shared drop counter so the main thread can monitor packet loss.
pub static CAPTURE_DROP_COUNT: AtomicU64 = AtomicU64::new(0);
pub static CAPTURE_TOTAL_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn start_capture(
    filter: &str,
    tx: Sender<ParsedPacket>,
    running: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    let filter = filter.to_string();

    thread::spawn(move || {
        info!("Opening WinDivert handle with filter: {}", filter);

        // Use sniff mode so the original packet is not blocked, just copied
        let flags = WinDivertFlags::new().set_sniff();
        let handle = match WinDivert::network(&filter, 0, flags) {
            Ok(h) => h,
            Err(e) => {
                error!("Failed to open WinDivert handle: {:?}", e);
                return;
            }
        };

        // Set queue parameters for better capture performance
        if let Err(e) = handle.set_param(WinDivertParam::QueueLength, 16384) {
            error!("Failed to set QueueLength: {:?}", e);
        }
        if let Err(e) = handle.set_param(WinDivertParam::QueueTime, 8000) {
            error!("Failed to set QueueTime: {:?}", e);
        }
        // Set queue size to 32MB (default 4MB is too small for 100Mbps+)
        if let Err(e) = handle.set_param(WinDivertParam::QueueSize, 33554432) {
            error!("Failed to set QueueSize: {:?}", e);
        }

        let mut buffer = vec![0u8; 65535];

        info!("Capture loop started");

        while running.load(Ordering::Relaxed) {
            match handle.recv(Some(&mut buffer)) {
                Ok(packet) => {
                    let outbound = packet.address.outbound();
                    CAPTURE_TOTAL_COUNT.fetch_add(1, Ordering::Relaxed);

                    // Parse in the capture thread — only send the small
                    // ParsedPacket struct through the channel (no heap alloc)
                    let parsed = match parser::parse_packet(&packet.data, outbound) {
                        Some(p) => p,
                        None => continue, // Not TCP/UDP or malformed
                    };

                    // Use try_send to avoid blocking the capture thread.
                    // If the channel is full, drop this packet and count it
                    // rather than blocking and causing WinDivert queue overflow.
                    match tx.try_send(parsed) {
                        Ok(()) => {}
                        Err(crossbeam_channel::TrySendError::Full(_)) => {
                            CAPTURE_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                            info!("Capture channel closed, stopping capture loop");
                            break;
                        }
                    }
                }
                Err(e) => {
                    if running.load(Ordering::Relaxed) {
                        error!("Recv error: {:?}", e);
                    }
                    break;
                }
            }
        }

        info!("Capture loop stopped");
    })
}
