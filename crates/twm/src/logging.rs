//! Minimal file logger for the `log` facade. Size-capped: when the file
//! exceeds 2 MiB it is rotated to `twm.log.1` (one generation kept), so
//! logging can never fill the disk.

use log::{Level, LevelFilter, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use windows::Win32::System::SystemInformation::GetLocalTime;

const MAX_BYTES: u64 = 2 * 1024 * 1024;

struct Logger {
    path: PathBuf,
    file: Mutex<Option<File>>,
    also_stderr: bool,
}

impl log::Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= log::max_level()
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let t = unsafe { GetLocalTime() };
        let line = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {:<5} {}\n",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds, r.level(), r.args()
        );
        if self.also_stderr {
            eprint!("{line}");
        }
        let mut g = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if g.as_ref().and_then(|f| f.metadata().ok()).is_some_and(|m| m.len() > MAX_BYTES) {
            *g = None;
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
        }
        if g.is_none() {
            *g = OpenOptions::new().create(true).append(true).open(&self.path).ok();
        }
        if let Some(f) = g.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.flush();
        }
    }
}

pub fn init(path: PathBuf, level: &str, also_stderr: bool) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let filter = match level {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => LevelFilter::Info,
    };
    let logger = Box::leak(Box::new(Logger { path, file: Mutex::new(None), also_stderr }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(filter);
    }
    let _ = Level::Info;
}
