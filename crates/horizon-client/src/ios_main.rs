//! iOS's native entry point. The simulation, renderer and launcher are shared with Android: the
//! modules here are Android's wherever they hold no Android API, and their iOS counterparts
//! otherwise.
#![cfg(target_os = "ios")]

#[allow(dead_code, clippy::collapsible_match)]
// Shared vanilla match structure and desktop harnesses.
mod game;
#[path = "ios/game_launch.rs"]
mod game_launch;
#[path = "ios/launcher.rs"]
mod launcher;
// Share the desktop launcher's actual search, compatibility checks, downloads,
// dependency resolution and Java content porting; GPUI itself is desktop-only.
// These files are formatted in the launcher's separate Rust 2021 workspace.
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
#[path = "android/device_preferences.rs"]
mod device_preferences;
#[path = "ios/device_runtime.rs"]
mod device_runtime;
#[path = "android/diagnostics.rs"]
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
#[path = "android/performance.rs"]
mod performance;
#[path = "ios/platform.rs"]
mod platform;
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../launcher/src/proxy.rs"]
mod proxy;
#[path = "android/search_queue.rs"]
mod search_queue;
#[path = "ios/secure_store.rs"]
mod secure_store;
#[path = "android/shader_catalog.rs"]
mod shader_catalog;
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../launcher/src/skins.rs"]
mod skins;
// The shared account protocol needs only the launcher's version and data path.
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
        HOME.get().expect("main sets account home").clone()
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

/// Android's logger, with stderr (Xcode's console and the device log) in place of logcat: every
/// line also feeds the local diagnostics the launcher exports.
struct Logger;
impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = record.args().to_string();
        eprintln!("Horizon {} {}: {message}", record.level(), record.target());
        diagnostics::log_line(record.level(), record.target(), &message);
    }
    fn flush(&self) {
        diagnostics::request_flush();
    }
}
static LOGGER: Logger = Logger;

fn init_logging() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// Horizon's folder in the app's Documents, which the Files app shows (UIFileSharingEnabled).
fn data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join("Documents/horizon"))
}

/// The engine tables ship inside the app bundle, as Android's ship in its APK assets; the
/// launcher installs the game from the copy in the data folder.
fn unpack_engine_data(home: &std::path::Path) {
    let Some(bundled) = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("engine-data.zip")))
        .filter(|path| path.is_file())
    else {
        log::error!("This build has no engine data (engine-data.zip in the app bundle)");
        return;
    };
    let copied = (|| -> std::io::Result<()> {
        let partial = home.join("engine-data.partial");
        std::fs::copy(&bundled, &partial)?;
        std::fs::rename(partial, home.join("engine-data.zip"))
    })();
    if let Err(error) = copied {
        log::error!("Cannot unpack engine data: {error}");
    }
}

fn main() {
    init_logging();
    horizon_protocol::install_system_srv_lookup(dns::query_srv);
    let Some(home) = data_dir() else {
        log::error!("iOS supplied no home directory");
        return;
    };
    backend::initialize(home.clone());
    if let Err(error) = diagnostics::initialize(&home, serde_json::json!({})) {
        log::error!("Cannot initialize local diagnostics: {error}");
    }
    let Some(cache) = platform::cache_dir() else {
        log::error!("iOS supplied no application cache directory");
        return;
    };
    horizon_client::android_runtime::configure_cache_dir(cache.clone());
    horizon_client::official_install::configure_download_cache_dir(cache);
    horizon_client::android_runtime::install_clipboard(
        horizon_client::android_runtime::ClipboardHooks {
            get: platform::clipboard_get,
            set: platform::clipboard_set,
        },
    );
    if let Err(error) =
        std::fs::create_dir_all(&home).and_then(|()| std::env::set_current_dir(&home))
    {
        log::error!("Cannot open {}: {error}", home.display());
        return;
    }
    #[cfg(debug_assertions)]
    if secure_store::development_probe() {
        std::process::exit(0);
    }
    horizon_client::crash::install(home.clone(), "client");
    exit_reports::install(&home);
    let _ = std::fs::remove_file(home.join("ios-error.txt"));
    unpack_engine_data(&home);
    let mut launcher = launcher::Launcher::new(home.clone());
    // UIKit's main loop never returns; this continues only when winit could not start it.
    let result = winit::event_loop::EventLoop::new()
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })
        .and_then(|events| events.run_app(&mut launcher).map_err(Into::into));
    if let Err(error) = result {
        log::error!("Horizon: {error}");
        diagnostics::record_error(&error.to_string());
        horizon_client::crash::report_error(&home, "client", &*error);
        let _ = std::fs::write(home.join("ios-error.txt"), error.to_string());
    }
    drop(launcher);
    diagnostics::finish();
}
