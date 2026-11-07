//! Client-wide display/performance preferences, independent of touch layouts.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DISPLAY_RATES: [u16; 7] = [0, 30, 60, 90, 120, 144, 165];

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub display_hz: u16,
    pub performance_hints: bool,
    pub gpu_profile: bool,
}

impl Settings {
    pub fn sanitize(&mut self) {
        if !DISPLAY_RATES.contains(&self.display_hz) {
            self.display_hz = 0;
        }
    }

    pub fn migrate_missing(&mut self, client: &Value, touch: &Value) {
        if client.get("display_hz").is_none() {
            self.display_hz = touch["display_hz"]
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .unwrap_or(0);
        }
        if client.get("performance_hints").is_none() {
            self.performance_hints = touch["performance_hints"].as_bool().unwrap_or(false);
        }
        if client.get("gpu_profile").is_none() {
            self.gpu_profile = touch["gpu_profile"].as_bool().unwrap_or(false);
        }
        self.sanitize();
    }

    pub fn display_label(self) -> String {
        if self.display_hz == 0 {
            "System default".into()
        } else {
            format!("{} Hz (request)", self.display_hz)
        }
    }
}
