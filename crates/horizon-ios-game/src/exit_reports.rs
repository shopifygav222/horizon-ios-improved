//! The previous run's logs, error and crash reports, kept for the diagnostics export as Android's
//! are. iOS gives an app no history of how its earlier processes ended (Android's
//! ApplicationExitInfo), so the exit report says so instead of listing exits.
use serde_json::json;
use std::{io::Write, path::Path};

pub const FILE: &str = "ios-exits.json";

pub fn collect(home: &Path) {
    let dir = home.join("performance");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    // Preserve raw details locally; the user-requested ZIP redacts text exports.
    for (source, name) in [
        (home.join("logs/latest.log"), "previous-run.log"),
        (home.join("ios-error.txt"), "previous-error.txt"),
    ] {
        if let Ok(data) = crate::diagnostics::tail(&source, 256 * 1024) {
            let _ = std::fs::write(dir.join(name), data);
        }
    }
    if let Ok(entries) = std::fs::read_dir(home.join("crash-reports")) {
        let mut files: Vec<_> = entries
            .flatten()
            .filter(|f| f.file_type().is_ok_and(|t| t.is_file()))
            .collect();
        files.sort_by_key(|f| f.file_name());
        for f in files.into_iter().rev().take(3) {
            if let Ok(data) = crate::diagnostics::tail(&f.path(), 256 * 1024) {
                let _ = std::fs::write(dir.join(f.file_name()), data);
            }
        }
    }
    let report = json!({
        "schema": 2,
        "version": crate::backend::VERSION,
        "source_commit": option_env!("HORIZON_IOS_BUILD_COMMIT"),
        "collected_at_ms": crate::diagnostics::timestamp(),
        "device": {"abi": std::env::consts::ARCH},
        "collection_status": "unsupported_on_ios",
        "exits": [],
        "note": "iOS does not tell apps how their previous process ended. Horizon's own logs, error and crash reports are included instead.",
    });
    if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
        let _ = std::fs::write(dir.join(FILE), bytes);
    }
}

pub fn install(home: &Path) {
    collect(home);
    let dir = home.join("performance");
    let old = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Panic hooks may run immediately before abort: do not depend on the async writer.
        if let Ok(mut file) = std::fs::File::create(dir.join("last-rust-panic.txt")) {
            let _ = write!(
                file,
                "panic_timestamp_ms: {}\n{info}\n{}",
                crate::diagnostics::timestamp(),
                std::backtrace::Backtrace::force_capture()
            );
            let _ = file.sync_all();
        }
        old(info);
    }));
}
