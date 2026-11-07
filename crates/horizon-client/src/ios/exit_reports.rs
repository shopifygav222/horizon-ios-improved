//! Read the previous run's crash report, if any. iOS does not expose
//! process-exit reasons the way Android's ApplicationExitInfo does, so
//! we rely on Horizon's own error file and the diagnostics log.
use serde_json::{Value, json};
use std::path::Path;

pub fn install(home: &Path) {
    let dir = home.join("diagnostics");
    let _ = std::fs::create_dir_all(&dir);
    let error_path = home.join("ios-error.txt");
    let report_path = dir.join("ios-exits.json");
    if !error_path.exists() && !report_path.exists() {
        let _ = std::fs::write(
            &report_path,
            serde_json::to_vec_pretty(&json!({
                "schema": 2,
                "collection_status": "clean",
                "collected_at_ms": crate::diagnostics::timestamp(),
            }))
            .unwrap_or_default(),
        );
        return;
    }
    let previous_error = std::fs::read_to_string(&error_path).ok();
    let _ = std::fs::remove_file(&error_path);
    let report = json!({
        "schema": 2,
        "collection_status": "collected",
        "collected_at_ms": crate::diagnostics::timestamp(),
        "previous_error": previous_error,
    });
    if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
        let _ = std::fs::write(&report_path, bytes);
    }
}

pub fn bundle(home: &Path) -> Value {
    let report_path = home.join("diagnostics").join("ios-exits.json");
    std::fs::read(&report_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({"schema":2,"collection_status":"unavailable"}))
}
