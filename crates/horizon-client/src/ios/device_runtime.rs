//! Frame pacing, gyro aiming and frame-work hints on iOS. iOS sets the display rate itself
//! (ProMotion follows the frames the game presents) and has no ADPF, so those report as
//! unavailable; the gyroscope is CoreMotion's, through the same math as Android's.
#[path = "../android/device_math.rs"]
mod math;
use std::cell::RefCell;

#[derive(Default)]
struct Gyro {
    enabled: bool,
    available: bool,
    last: f64,
    samples: u64,
}

thread_local! {
    static GYRO: RefCell<Gyro> = RefCell::new(Gyro::default());
}

pub struct Frame;

pub fn frame(_enabled: bool, _hz: u16, _active: bool) -> Frame {
    Frame
}

/// Gyro uses device axes, remapped by the interface's rotation (Android's convention).
pub fn gyro(enabled: bool, sensitivity: f32, rotation: i32) -> [f32; 2] {
    GYRO.with(|slot| {
        let mut gyro = slot.borrow_mut();
        if gyro.enabled != enabled {
            gyro.enabled = enabled;
            gyro.last = 0.0;
            if enabled {
                gyro.available = horizon_gamecontroller::gyro_start();
            } else {
                horizon_gamecontroller::gyro_stop();
            }
        }
        if !enabled || !gyro.available {
            return [0.0; 2];
        }
        let Some((rate, time)) = horizon_gamecontroller::gyro_sample() else {
            return [0.0; 2];
        };
        // Each sample counts once: its rate over the time since the last one the game used.
        let seconds = (time - gyro.last) as f32;
        let valid = gyro.last != 0.0 && seconds > 0.0 && seconds < 0.1;
        gyro.last = time;
        if !valid {
            return [0.0; 2];
        }
        gyro.samples += 1;
        math::gyro_delta(rate, seconds, rotation, sensitivity).map(|v| v.clamp(-20.0, 20.0))
    })
}

pub fn diagnostics() -> serde_json::Value {
    GYRO.with(|slot| {
        let gyro = slot.borrow();
        serde_json::json!({
            "display_hz_requested": preferences().display_hz,
            "display_request_result": "system_managed",
            "adpf_session_active": false,
            "adpf_unavailable": true,
            "gyro_available": gyro.available,
            "gyro_active": gyro.enabled && gyro.available,
            "gyro_samples": gyro.samples,
        })
    })
}

/// Release sensor delivery immediately when the window loses focus.
pub fn suspend() {
    let _ = gyro(false, 1.0, 0);
}

// One atomic snapshot avoids mixing frequency and flags when the client changes
// its preferences. Touch profiles never own these system settings.
static PREFERENCES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub fn configure(mut settings: crate::device_preferences::Settings) {
    settings.sanitize();
    let bits = u32::from(settings.display_hz)
        | (u32::from(settings.performance_hints) << 16)
        | (u32::from(settings.gpu_profile) << 17);
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
