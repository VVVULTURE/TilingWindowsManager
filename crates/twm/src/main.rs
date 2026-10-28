//! TilingWindowsManager daemon entry point.
//!
//! Runs in the normal (unelevated) user session alongside Explorer. One
//! instance per logon session; `twmctl` talks to it over a named pipe.
#![windows_subsystem = "windows"]

mod daemon;
mod ipc;
mod logging;
mod paths;
mod recovery;
mod win;

use std::path::PathBuf;
use windows::core::HSTRING;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

fn fatal(msg: &str) -> ! {
    log::error!("{msg}");
    unsafe {
        MessageBoxW(None, &HSTRING::from(msg), &HSTRING::from("TilingWindowsManager"), MB_OK | MB_ICONERROR);
    }
    std::process::exit(1);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut config: Option<PathBuf> = None;
    let mut log_level = std::env::var("TWM_LOG").unwrap_or_else(|_| "info".into());
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" | "-c" => config = args.next().map(PathBuf::from),
            "--log-level" => log_level = args.next().unwrap_or_default(),
            "--version" | "-V" => {
                println!("twm {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("twm [--config <file>] [--log-level error|warn|info|debug|trace]\nControl a running instance with twmctl.");
                return;
            }
            other => fatal(&format!("unknown argument `{other}` (see twm --help)")),
        }
    }
    logging::init(paths::log_file(), &log_level, false);

    // Physical-pixel coordinates on every monitor (required to place
    // windows precisely on mixed-DPI setups).
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
            log::warn!("per-monitor DPI awareness v2 unavailable");
        }
    }

    // Single instance per session (and per test instance name).
    let _mutex = unsafe { CreateMutexW(None, true, &HSTRING::from(paths::mutex_name())) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        fatal("TilingWindowsManager is already running. Use the tray icon or `twmctl quit` to stop it.");
    }

    // If anything panics, never leave windows hidden.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let n = recovery::restore_all();
        log::error!("panic: {info} (restored {n} hidden windows)");
        default_hook(info);
    }));

    let config = config.unwrap_or_else(paths::default_config_file);
    log::info!("TilingWindowsManager {} starting; config {}", env!("CARGO_PKG_VERSION"), config.display());
    let mut d = match daemon::Daemon::new(config) {
        Ok(d) => d,
        Err(e) => fatal(&format!("could not start: {e}")),
    };
    d.run();
}
