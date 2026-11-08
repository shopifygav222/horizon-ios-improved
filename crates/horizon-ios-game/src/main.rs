//! Starling on iOS (this port, IOS-SYNC.md): the app's program. UIKit makes one event loop a
//! process, here; what runs in it is Horizon Android's launcher (launcher.rs and its pages, the
//! modules below), and Starling's game when the player plays (game_launch.rs), every callback on a
//! 16 MB stack: iOS gives its main thread 1 MB, and the game goes deeper than that.
//!
//! The game folder is the app's Documents, which the Files app shows as "Starling", as Android's
//! is `Starling` in shared storage; the private files (the prepared resources, caches, the import
//! inbox, `horizon-args.txt`) are in Library/Application Support/Starling, as Android's are in
//! the app's own files. The launcher's modules are Horizon Android's iOS port's, with Starling's
//! game under them.
#[cfg(target_os = "ios")]
mod app;
#[cfg(target_os = "ios")]
mod catalog;
#[cfg(target_os = "ios")]
mod device_math;
#[cfg(target_os = "ios")]
mod device_preferences;
#[cfg(target_os = "ios")]
mod device_runtime;
#[cfg(target_os = "ios")]
mod diagnostics;
#[cfg(target_os = "ios")]
mod display;
#[cfg(target_os = "ios")]
mod dns;
#[cfg(target_os = "ios")]
mod document_store;
#[cfg(target_os = "ios")]
mod documents;
#[cfg(target_os = "ios")]
mod exit_reports;
#[cfg(target_os = "ios")]
mod gallery;
#[cfg(target_os = "ios")]
mod game_launch;
#[cfg(target_os = "ios")]
mod launcher;
#[cfg(target_os = "ios")]
mod native_ime;
#[cfg(target_os = "ios")]
mod official_install;
#[cfg(target_os = "ios")]
mod performance;
#[cfg(target_os = "ios")]
mod platform;
#[cfg(target_os = "ios")]
mod search_queue;
#[cfg(target_os = "ios")]
mod secure_store;
#[cfg(target_os = "ios")]
mod shader_catalog;
// Horizon Android's desktop launcher's account, catalog, proxy and wardrobe code, which its
// mobile launcher shares, as that launcher carried them (shared/; formatted there, in Rust 2021).
#[cfg(target_os = "ios")]
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "shared/account.rs"]
mod account;
#[cfg(target_os = "ios")]
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "shared/browser.rs"]
mod browser;
#[cfg(target_os = "ios")]
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "shared/proxy.rs"]
mod proxy;
#[cfg(target_os = "ios")]
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "shared/skins.rs"]
mod skins;

/// What the shared account code needs of the launcher: its version and its data folder.
#[cfg(target_os = "ios")]
mod backend {
    /// Starling's own version (packaging/android/build_apk.py's VERSION_NAME).
    pub const VERSION: &str = if crate::PUBLIC_BUILD {
        "0.2.3"
    } else {
        "0.2.3-dev"
    };
    static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    pub fn initialize(home: std::path::PathBuf) {
        let _ = HOME.set(home);
    }
    pub fn game_home() -> std::path::PathBuf {
        data_dir()
    }
    pub fn game_mod_api_level() -> Option<u32> {
        Some(horizon_mods::api_level::LEVEL)
    }
    pub fn data_dir() -> std::path::PathBuf {
        HOME.get().expect("main sets account home").clone()
    }
}

#[cfg(target_os = "ios")]
pub(crate) fn configure_input_mode(mode: catalog::InputHost) {
    horizon_client::android_runtime::configure_input(mode);
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
