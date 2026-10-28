//! IPC protocol between `twm` and `twmctl` (hyprctl-style). One JSON
//! request line per connection, answered by one JSON response line, over a
//! per-session named pipe that rejects remote clients.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    Ping,
    Version,
    /// Run an action, e.g. `focus left` (same syntax as key bindings).
    Dispatch { action: String },
    /// Monitors, workspaces, managed windows and focus.
    State,
    /// Every visible top-level window with its classification.
    Windows,
    /// Why a window is (not) managed, and which rules matched.
    Explain { hwnd: u64 },
    Binds,
    /// Diagnostics from the last configuration load.
    Diagnostics,
    Reload,
    /// Timing/memory counters.
    Stats,
    Quit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(data: serde_json::Value) -> Response {
        Response { ok: true, data, error: None }
    }
    pub fn err(e: impl Into<String>) -> Response {
        Response { ok: false, data: serde_json::Value::Null, error: Some(e.into()) }
    }
}

/// Send one request to the pipe `name` and wait for the response line.
/// `Err` means the manager is not running or did not answer.
pub fn call(name: &str, req: &Request) -> Result<Response, String> {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut pipe = loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(name) {
            Ok(p) => break p,
            // ERROR_PIPE_BUSY: another client is being served; retry briefly.
            Err(e) if e.raw_os_error() == Some(231) && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("TilingWindowsManager is not running ({e})")),
        }
    };
    let mut line = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    pipe.write_all(&line).map_err(|e| e.to_string())?;
    // Read one response line. The server disconnects right after writing,
    // which surfaces as ERROR_PIPE_NOT_CONNECTED rather than EOF, so stop
    // at the newline instead of reading to the end.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    while !buf.contains(&b'\n') {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) if !buf.is_empty() => break,
            Err(e) => return Err(format!("no response from TilingWindowsManager ({e})")),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    serde_json::from_str(text.trim()).map_err(|e| format!("bad response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let r = Request::Dispatch { action: "focus left".into() };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, r#"{"cmd":"dispatch","action":"focus left"}"#);
        assert_eq!(serde_json::from_str::<Request>(r#"{"cmd":"state"}"#).unwrap(), Request::State);
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"nope"}"#).is_err());
    }
}
