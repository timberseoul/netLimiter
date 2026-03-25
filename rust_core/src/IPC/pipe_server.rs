use std::io::{BufRead, BufReader, BufWriter, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use arc_swap::ArcSwap;
use log::{error, info};

use super::protocol::{IpcRequest, IpcResponse};
use crate::stats::flow_stat::ProcessStats;

const PIPE_NAME: &str = r"\\.\pipe\netlimiter_ipc";

/// Start the named pipe server on a background thread.
/// It listens for JSON requests from the Go TUI and responds with stats.
pub fn start_pipe_server(
    stats_ref: Arc<ArcSwap<Vec<ProcessStats>>>,
    running: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        info!("Starting named pipe server on {}", PIPE_NAME);

        while running.load(Ordering::Relaxed) {
            match create_named_pipe_instance() {
                Ok(pipe) => {
                    info!("Waiting for client connection...");
                    if !connect_pipe(&pipe) {
                        error!("Failed to connect pipe");
                        continue;
                    }
                    info!("Client connected");

                    handle_client(pipe, &stats_ref, &running);

                    info!("Client disconnected");
                }
                Err(e) => {
                    error!("Failed to create pipe: {}", e);
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }

        info!("Pipe server stopped");
    })
}

/// Platform-specific named pipe creation and handling using raw Win32 API.
#[cfg(windows)]
fn create_named_pipe_instance() -> Result<PipeHandle, String> {
    use std::ffi::CString;

    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{
        CreateNamedPipeA, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
        PIPE_WAIT,
    };
    use windows::core::PCSTR;

    unsafe {
        let pipe_name = CString::new(PIPE_NAME).unwrap();
        let handle = CreateNamedPipeA(
            PCSTR(pipe_name.as_ptr() as *const u8),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            65536,
            65536,
            0,
            None,
        );

        match handle {
            Ok(h) => Ok(PipeHandle(h)),
            Err(e) => Err(format!("CreateNamedPipe failed: {}", e)),
        }
    }
}

#[cfg(windows)]
fn connect_pipe(pipe: &PipeHandle) -> bool {
    use windows::Win32::System::Pipes::ConnectNamedPipe;

    unsafe {
        let result = ConnectNamedPipe(pipe.0, None);
        // ConnectNamedPipe returns false if the client connected between
        // CreateNamedPipe and ConnectNamedPipe, but that's still a valid connection
        result.is_ok() || std::io::Error::last_os_error().raw_os_error() == Some(535)
    }
}

#[cfg(windows)]
fn handle_client(
    pipe: PipeHandle,
    stats_ref: &Arc<ArcSwap<Vec<ProcessStats>>>,
    running: &Arc<AtomicBool>,
) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Pipes::DisconnectNamedPipe;

    let handle = pipe.0;

    // Wrap the pipe handle in a Read/Write adapter
    let mut reader = PipeReader(handle);
    let mut buf_reader = BufReader::new(&mut reader);
    let mut line = String::new();

    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }

        line.clear();
        match buf_reader.read_line(&mut line) {
            Ok(0) => break, // Client disconnected
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let response = match serde_json::from_str::<IpcRequest>(trimmed) {
                    Ok(req) => match req.command.as_str() {
                        "get_stats" => {
                            // ArcSwap load — lock-free, no deep clone
                            let stats = stats_ref.load();
                            IpcResponse::stats_ref(&stats)
                        }
                        "ping" => IpcResponse::ack(),
                        _ => IpcResponse::error(&format!("Unknown command: {}", req.command)),
                    },
                    Err(e) => IpcResponse::error(&format!("Invalid JSON: {}", e)),
                };

                let mut writer = BufWriter::new(PipeWriter(handle));
                if serde_json::to_writer(&mut writer, &response).is_err() {
                    break;
                }
                if writer.write_all(b"\n").is_err() {
                    break;
                }
                if writer.flush().is_err() {
                    break;
                }
            }
            Err(e) => {
                error!("Read error: {}", e);
                break;
            }
        }
    }

    unsafe {
        let _ = DisconnectNamedPipe(handle);
        let _ = CloseHandle(handle);
    }
}

#[cfg(windows)]
struct PipeHandle(windows::Win32::Foundation::HANDLE);

// Safety: We only access the handle from a single thread per client
#[cfg(windows)]
unsafe impl Send for PipeHandle {}

#[cfg(windows)]
struct PipeReader(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl std::io::Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use windows::Win32::Storage::FileSystem::ReadFile;

        unsafe {
            let mut bytes_read: u32 = 0;
            let result = ReadFile(self.0, Some(buf), Some(&mut bytes_read), None);
            match result {
                Ok(()) => Ok(bytes_read as usize),
                Err(_) => Err(std::io::Error::last_os_error()),
            }
        }
    }
}

#[cfg(windows)]
struct PipeWriter(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl std::io::Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use windows::Win32::Storage::FileSystem::WriteFile;

        unsafe {
            let mut bytes_written: u32 = 0;
            let result = WriteFile(self.0, Some(buf), Some(&mut bytes_written), None);
            match result {
                Ok(()) => Ok(bytes_written as usize),
                Err(_) => Err(std::io::Error::last_os_error()),
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        use windows::Win32::Storage::FileSystem::FlushFileBuffers;

        unsafe {
            FlushFileBuffers(self.0).map_err(|_| std::io::Error::last_os_error())
        }
    }
}
