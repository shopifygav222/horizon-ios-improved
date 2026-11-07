//! iOS diagnostics stubs. Lightweight heartbeat and error recording.
use serde_json::Value;
use std::{
    path::Path,
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

struct State {
    heartbeat: Instant,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn initialize(_home: &Path, _device: Value) -> std::io::Result<()> {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.is_none() {
        *state = Some(State {
            heartbeat: Instant::now(),
        });
    }
    Ok(())
}

pub fn heartbeat(_phase: &str, _context: impl FnOnce() -> Value) {}

pub fn record_error(error: &str) {
    log::error!("iOS diagnostics: {error}");
}

pub fn log_line(_level: log::Level, _target: &str, _message: &str) {}

pub fn request_flush() {}
