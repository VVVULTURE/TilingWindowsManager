//! Per-user locations. Nothing is written outside the current user's
//! profile. `TWM_CONFIG_DIR`, `TWM_DATA_DIR` and `TWM_INSTANCE` let tests run
//! an isolated instance next to a normal one.

use std::path::PathBuf;

fn env_dir(var: &str, base: &str) -> PathBuf {
    if let Some(d) = std::env::var_os(var) {
        return PathBuf::from(d);
    }
    let base = std::env::var_os(base).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("TilingWindowsManager")
}

/// %APPDATA%\TilingWindowsManager — config.toml and themes\.
pub fn config_dir() -> PathBuf {
    env_dir("TWM_CONFIG_DIR", "APPDATA")
}

/// %LOCALAPPDATA%\TilingWindowsManager — logs and crash-recovery state.
pub fn data_dir() -> PathBuf {
    env_dir("TWM_DATA_DIR", "LOCALAPPDATA")
}

pub fn default_config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}

pub fn log_file() -> PathBuf {
    data_dir().join("twm.log")
}

pub fn recovery_file() -> PathBuf {
    data_dir().join("hidden-windows.json")
}

pub fn instance() -> String {
    std::env::var("TWM_INSTANCE").ok().filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).unwrap_or_default()
}

fn session_id() -> u32 {
    let mut s = 0u32;
    unsafe {
        let _ = windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(std::process::id(), &mut s);
    }
    s
}

/// Named pipe for twmctl/IPC; one per logon session (and test instance).
pub fn pipe_name() -> String {
    let inst = instance();
    if inst.is_empty() {
        format!(r"\\.\pipe\TilingWindowsManager-{}", session_id())
    } else {
        format!(r"\\.\pipe\TilingWindowsManager-{}-{inst}", session_id())
    }
}

pub fn mutex_name() -> String {
    format!("Local\\TilingWindowsManager-{}", instance())
}
