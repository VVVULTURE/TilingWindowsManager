//! Named-pipe server. Runs on its own thread; each request is handed to the
//! main thread (which owns all window state) and the reply is written back.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;
use twm_core::ipc::{Request, Response};
use windows::core::HSTRING;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};

pub const WM_APP_IPC: u32 = WM_APP + 2;

pub struct Pending {
    pub request: Result<Request, String>,
    pub reply: Sender<Response>,
}

static INBOX: Mutex<Option<Receiver<Pending>>> = Mutex::new(None);

/// Takes all requests waiting for the main thread.
pub fn take_pending() -> Vec<Pending> {
    let g = INBOX.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default()
}

pub fn start(main_thread: u32) {
    let (tx, rx) = channel::<Pending>();
    *INBOX.lock().unwrap() = Some(rx);
    let name = HSTRING::from(crate::paths::pipe_name());
    let _ = std::thread::Builder::new().name("ipc".into()).spawn(move || loop {
        let pipe = unsafe {
            CreateNamedPipeW(
                &name,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                None,
            )
        };
        if pipe.is_invalid() {
            log::error!("CreateNamedPipe failed: {:?}", windows::core::Error::from_thread());
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        if connected.is_ok() || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
            serve(pipe, &tx, main_thread);
        }
        unsafe {
            let _ = DisconnectNamedPipe(pipe);
            let _ = CloseHandle(pipe);
        }
    });
}

fn serve(pipe: HANDLE, tx: &Sender<Pending>, main_thread: u32) {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    while !data.contains(&b'\n') && data.len() < 1 << 20 {
        let mut n = 0u32;
        let ok = unsafe { ReadFile(pipe, Some(&mut buf), Some(&mut n), None) };
        if ok.is_err() || n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    let line = String::from_utf8_lossy(&data);
    let line = line.lines().next().unwrap_or("").trim();
    let request = serde_json::from_str::<Request>(line).map_err(|e| format!("bad request: {e}"));
    let (rtx, rrx) = channel();
    let resp = if tx.send(Pending { request, reply: rtx }).is_ok() {
        unsafe {
            let _ = PostThreadMessageW(main_thread, WM_APP_IPC, WPARAM(0), LPARAM(0));
        }
        rrx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| Response::err("timed out waiting for the window manager"))
    } else {
        Response::err("window manager is shutting down")
    };
    let mut out = serde_json::to_vec(&resp).unwrap_or_default();
    out.push(b'\n');
    let mut written = 0u32;
    unsafe {
        let _ = WriteFile(pipe, Some(&out), Some(&mut written), None);
        let _ = FlushFileBuffers(pipe);
    }
}
