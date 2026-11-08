//! Starling on iOS (this port): the app's program. UIKit makes one event loop a process, here,
//! and what runs in it is the game Starling's own start makes (`horizon_client::entry::start`,
//! handed over by `horizon_client::ios::take_game`), every callback on a 16 MB stack: iOS gives
//! its main thread 1 MB, and the game goes deeper than that (the desktop links its main thread
//! with 8 MB; Horizon Android's iOS port crashed at Play without it).
//!
//! The game folder is the app's Documents, which the Files app shows as "Starling", as Android's
//! is `Starling` in shared storage; the private files (the prepared resources, caches, the import
//! inbox, `horizon-args.txt`) are in Library/Application Support/Starling, as Android's are in
//! the app's own files.
#[cfg(target_os = "ios")]
mod app;
#[cfg(target_os = "ios")]
mod dns;

fn main() {
    #[cfg(target_os = "ios")]
    app::main();
    #[cfg(not(target_os = "ios"))]
    eprintln!("horizon-ios-game is Starling's iOS app: build it for aarch64-apple-ios");
}
