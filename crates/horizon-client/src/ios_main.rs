//! iOS native entry point. The simulation and renderer are shared with desktop and Android.
#![cfg(target_os = "ios")]

#[allow(dead_code, clippy::collapsible_match)]
mod game;
#[path = "ios/game_launch.rs"]
mod game_launch;
#[path = "ios/launcher.rs"]
mod launcher;
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../launcher/src/account.rs"]
mod account;
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../launcher/src/browser.rs"]
mod browser;
#[path = "ios/dns.rs"]
mod dns;
#[path = "ios/catalog.rs"]
mod catalog;
#[path = "ios/device_preferences.rs"]
mod device_preferences;
#[path = "ios/device_runtime.rs"]
mod device_runtime;
#[path = "ios/diagnostics.rs"]
mod diagnostics;
#[path = "ios/display.rs"]
mod display;
#[path = "ios/document_store.rs"]
mod document_store;
#[path = "ios/documents.rs"]
mod documents;
#[path = "ios/exit_reports.rs"]
mod exit_reports;
#[path = "ios/gallery.rs"]
mod gallery;
#[path = "ios/native_ime.rs"]
mod native_ime;
#[path = "ios/performance.rs"]
mod performance;
#[path = "ios/platform.rs"]
mod platform;
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../launcher/src/proxy.rs"]
mod proxy;
#[path = "ios/search_queue.rs"]
mod search_queue;
#[path = "ios/secure_store.rs"]
mod secure_store;
#[path = "ios/shader_catalog.rs"]
mod shader_catalog;
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../launcher/src/skins.rs"]
mod skins;

mod backend {
    pub const VERSION: &str = if crate::PUBLIC_BUILD {
        "0.7.8"
    } else {
        "0.7.8-dev"
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
        HOME.get().expect("ios_main sets home directory").clone()
    }
}

pub(crate) fn keyboard_mouse_input() -> bool {
    horizon_client::android_runtime::active_input() == catalog::InputHost::KeyboardMouse
}
pub(crate) fn configure_input_mode(mode: catalog::InputHost) {
    horizon_client::android_runtime::configure_input(mode);
}
pub(crate) fn touch_visible() -> bool {
    horizon_client::android_runtime::active_input() == catalog::InputHost::Touch
}
pub(crate) fn touch_input(touch: bool) {
    horizon_client::android_runtime::note_input(if touch {
        catalog::InputHost::Touch
    } else {
        catalog::InputHost::KeyboardMouse
    });
}
pub(crate) fn save_input_mode() {
    let home = backend::data_dir();
    let mut config = catalog::Config::load(&home);
    let mode = horizon_client::android_runtime::input_mode();
    if config.input_host != mode {
        config.input_host = mode;
        if let Err(e) = config.save(&home) {
            log::warn!("Saving input mode: {e}");
        }
    }
}

pub(crate) const PUBLIC_BUILD: bool = match option_env!("HORIZON_IOS_PUBLIC") {
    Some(value) => value.len() == 1 && value.as_bytes()[0] == b'1',
    None => false,
};

#[global_allocator]
static ALLOCATOR: horizon_threads::Allocator = horizon_threads::Allocator;

fn ios_main() {
    horizon_threads::configure_allocator();
    let home = ios_data_dir();
    backend::initialize(home.clone());
    if let Err(error) = diagnostics::initialize(&home, serde_json::json!({})) {
        log::error!("Cannot initialize local diagnostics: {error}");
    }
    if let Some(cache) = platform::cache_dir() {
        horizon_client::android_runtime::configure_cache_dir(cache.clone());
        horizon_client::official_install::configure_download_cache_dir(cache);
    }
    let config = catalog::Config::load(&home);
    horizon_client::android_runtime::configure_input(config.input_host);
    device_runtime::configure(config.device);
    display::set_scale(config.render_scale);
    let arguments = vec![
        "--home".into(),
        home.to_string_lossy().into_owned(),
    ];
    horizon_client::telemetry::mark_start();
    horizon_client::logging::set_game_home(home.clone());
    horizon_client::logging::init(&home, "Render thread");
    log::info!("Horizon iOS {}", horizon_client::crash::version());
    horizon_client::crash::install(home.clone(), "client");
    if let Err(error) = game::ios_game(arguments) {
        log::error!("Horizon: {error}");
        horizon_client::logging::flush();
        horizon_client::crash::report_error(&home, "client", &*error);
    }
    horizon_client::logging::flush();
}

fn ios_data_dir() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|h| h.join("Documents/horizon"))
        .unwrap_or_else(|| std::path::PathBuf::from("horizon"))
}

/// C entry point called from the iOS launcher framework.
#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_start() {
    ios_main();
}
