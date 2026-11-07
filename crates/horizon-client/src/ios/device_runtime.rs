//! iOS device runtime stubs. No ADPF or native frame rate requests.
use std::time::Instant;

pub struct Frame {
    start: Instant,
    enabled: bool,
}

impl Drop for Frame {
    fn drop(&mut self) {
        let _ = (self.start, self.enabled);
    }
}

pub fn frame(enabled: bool, _hz: u16, _active: bool) -> Frame {
    Frame {
        start: Instant::now(),
        enabled,
    }
}

pub fn suspend() {}

static PREFERENCES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn configure(mut settings: crate::device_preferences::Settings) {
    settings.sanitize();
    let bits = u32::from(settings.display_hz)
        | if settings.performance_hints { 1 << 16 } else { 0 }
        | if settings.gpu_profile { 1 << 17 } else { 0 };
    PREFERENCES.store(bits, std::sync::atomic::Ordering::Relaxed);
}

pub fn preferences() -> crate::device_preferences::Settings {
    let bits = PREFERENCES.load(std::sync::atomic::Ordering::Relaxed);
    crate::device_preferences::Settings {
        display_hz: bits as u16,
        performance_hints: bits & (1 << 16) != 0,
        gpu_profile: bits & (1 << 17) != 0,
    }
}

pub fn diagnostics() -> serde_json::Value {
    serde_json::json!({
        "os": "iOS",
        "arch": std::env::consts::ARCH,
    })
}
