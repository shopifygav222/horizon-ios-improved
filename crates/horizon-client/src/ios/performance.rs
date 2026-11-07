//! iOS performance recording stubs. Same interface as Android.
use horizon_client::{control::FrameStats, spans};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn start(_home: &Path, _metadata: Value) -> horizon_client::Result<()> {
    Ok(())
}

pub fn stop() {
    ACTIVE.store(false, Ordering::Relaxed);
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn slow_frame(_cpu: f32, _timings: &[spans::Span]) {}

pub fn record(
    _frame: f32,
    _cpu: f32,
    _gpu: &[f32],
    _phases: [f32; 4],
) -> Option<Value> {
    None
}

pub fn submit(_row: Value, _context: Value) {}

pub fn reports(_home: &Path) -> Vec<std::path::PathBuf> {
    Vec::new()
}

pub fn profile_active() -> bool {
    false
}
