//! Background IPC to the running manager, so a slow or absent manager never
//! blocks the settings window. Each request kind has one slot holding the
//! last answer; at most one request per slot is in flight.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use twm_core::ipc::Request;

#[derive(Default)]
struct Slot {
    result: Option<Result<Value, String>>,
    at: Option<Instant>,
    busy: bool,
}

#[derive(Clone)]
pub struct Link {
    pipe: String,
    slots: Arc<Mutex<HashMap<String, Slot>>>,
}

impl Link {
    pub fn new(pipe: String) -> Link {
        Link { pipe, slots: Arc::default() }
    }

    /// Start `req` in the background unless one for `key` is already running.
    pub fn request(&self, key: &str, req: Request, ctx: &egui::Context) {
        {
            let mut slots = self.slots.lock().unwrap();
            let s = slots.entry(key.to_string()).or_default();
            if s.busy {
                return;
            }
            s.busy = true;
        }
        let (pipe, slots, key, ctx) = (self.pipe.clone(), self.slots.clone(), key.to_string(), ctx.clone());
        std::thread::spawn(move || {
            let r = twm_core::ipc::call(&pipe, &req).and_then(|r| if r.ok { Ok(r.data) } else { Err(r.error.unwrap_or_default()) });
            let mut slots = slots.lock().unwrap();
            let s = slots.entry(key).or_default();
            s.result = Some(r);
            s.at = Some(Instant::now());
            s.busy = false;
            ctx.request_repaint();
        });
    }

    /// Re-request when the last answer is older than `max_age` (or missing).
    pub fn refresh(&self, key: &str, req: Request, max_age: Duration, ctx: &egui::Context) {
        let stale = self.slots.lock().unwrap().get(key).and_then(|s| s.at).is_none_or(|t| t.elapsed() >= max_age);
        if stale {
            self.request(key, req, ctx);
        }
    }

    pub fn get(&self, key: &str) -> Option<Result<Value, String>> {
        self.slots.lock().unwrap().get(key).and_then(|s| s.result.clone())
    }

    pub fn take(&self, key: &str) -> Option<Result<Value, String>> {
        self.slots.lock().unwrap().get_mut(key).and_then(|s| s.result.take())
    }
}
