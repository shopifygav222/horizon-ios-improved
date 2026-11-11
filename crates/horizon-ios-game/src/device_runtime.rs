//! The device preferences of the launcher's settings on iOS: the display rate, the frame-work
//! hints and the GPU profile. iOS sets the display rate itself (ProMotion follows the frames the
//! game presents) and has no ADPF, so those report as the system's; the gyroscope is Starling's
//! own (horizon_client::ios), which the Horizon controls aim with.

/// What the device does with the preferences, for the performance recordings.
pub fn diagnostics() -> serde_json::Value {
    serde_json::json!({
        "display_hz_requested": preferences().display_hz,
        "display_request_result": "system_managed",
        "adpf_session_active": false,
        "adpf_unavailable": true,
        "gpu_profile": preferences().gpu_profile,
    })
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
