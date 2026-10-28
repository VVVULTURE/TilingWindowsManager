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
