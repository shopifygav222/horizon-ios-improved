//! Starling on iOS (this port, IOS-SYNC.md): the app's program. UIKit makes one event loop a
//! process, here; what runs in it is Horizon Android's launcher (launcher.rs and its pages, the
//! modules below), and Starling's game when the player plays (game_launch.rs), every callback on a
//! 16 MB stack: iOS gives its main thread 1 MB, and the game goes deeper than that.
//!
//! The game folder is the app's Documents, which the Files app shows as "Starling", as Android's
//! is `Starling` in shared storage; the private files (the prepared resources, caches, the import
//! inbox, `horizon-args.txt`) are in Library/Application Support/Starling, as Android's are in
//! the app's own files. The launcher's modules are Horizon Android's iOS port's, with Starling's
//! game under them: its accounts, catalog, diagnostics and controls are Starling's own
//! (horizon_client::accounts, starling-catalog), so the launcher and the game keep one of each.
#[cfg(target_os = "ios")]
mod app;
#[cfg(target_os = "ios")]
mod catalog;
#[cfg(target_os = "ios")]
mod content_files;
#[cfg(target_os = "ios")]
mod display;
#[cfg(target_os = "ios")]
mod dns;
#[cfg(target_os = "ios")]
mod document_store;
#[cfg(target_os = "ios")]
mod documents;
#[cfg(target_os = "ios")]
mod gallery;
#[cfg(target_os = "ios")]
mod game_launch;
#[cfg(target_os = "ios")]
mod ime_edit;
#[cfg(target_os = "ios")]
mod launcher;
#[cfg(target_os = "ios")]
mod native_ime;
#[cfg(target_os = "ios")]
mod official_install;
#[cfg(target_os = "ios")]
mod platform;
#[cfg(target_os = "ios")]
mod search_queue;
#[cfg(target_os = "ios")]
mod shader_catalog;
/// The game folder (the app's Documents), which the launcher's pages and file manager show.
#[cfg(target_os = "ios")]
static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
#[cfg(target_os = "ios")]
pub(crate) fn set_home(home: std::path::PathBuf) {
    let _ = HOME.set(home);
}
#[cfg(target_os = "ios")]
pub(crate) fn home() -> std::path::PathBuf {
    HOME.get().cloned().unwrap_or_default()
}
/// Whether the game is on screen, kept in a file (`logs/playing`) so the next start knows a game
/// that never came back to the launcher.
#[cfg(target_os = "ios")]
pub(crate) fn set_playing(playing: bool) {
    let marker = home().join("logs/playing");
    if playing {
        let _ = std::fs::write(marker, b"");
    } else {
        let _ = std::fs::remove_file(marker);
    }
}

/// Why the last run's game ended, when it ended by itself: read once at the start (app.rs), for
/// the launcher's Play page.
#[cfg(target_os = "ios")]
static LAST_CRASH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
#[cfg(target_os = "ios")]
pub(crate) fn set_last_crash(crash: Option<String>) {
    let _ = LAST_CRASH.set(crash);
}
#[cfg(target_os = "ios")]
pub(crate) fn last_crash() -> Option<String> {
    LAST_CRASH.get().cloned().flatten()
}

/// A build for others (CI's), as Horizon Android's: no developer launch arguments.
#[cfg(target_os = "ios")]
pub(crate) const PUBLIC_BUILD: bool = match option_env!("HORIZON_IOS_PUBLIC") {
    Some(value) => value.len() == 1 && value.as_bytes()[0] == b'1',
    None => false,
};

fn main() {
    #[cfg(target_os = "ios")]
    app::main();
    #[cfg(not(target_os = "ios"))]
    eprintln!("horizon-ios-game is Starling's iOS app: build it for aarch64-apple-ios");
}
