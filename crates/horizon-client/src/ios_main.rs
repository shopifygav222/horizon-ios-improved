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

/// Standard error, into `performance/stderr.log` as it is written (the last run's kept as
/// `previous-stderr.log`): it holds what a crash leaves, Rust's own fatal messages and the log's
/// last lines, where the buffered diagnostic log may not have caught up. The diagnostics export
/// takes it with the other logs.
fn capture_stderr(home: &std::path::Path) {
    use std::os::fd::AsRawFd;
    let dir = home.join("performance");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::rename(dir.join("stderr.log"), dir.join("previous-stderr.log"));
    if let Ok(file) = std::fs::File::create(dir.join("stderr.log")) {
        // SAFETY: descriptor 2 becomes a copy of the file's; `file` closes only its own.
        unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) };
    }
}

/// Every launcher and game callback runs on a stack of its own. Desktop links the game's main
/// thread with 8 MB (crates/horizon-client/build.rs: whole `Game`s are built deep in the event
/// loop); iOS fixes the main thread's stack at 1 MB, which Play overran. UIKit's frames stay on
/// the main thread's stack; a callback that already has room runs where it is.
const GAME_STACK: usize = 16 << 20;
const GAME_STACK_LEFT: usize = 8 << 20;

fn on_game_stack<R>(callback: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(GAME_STACK_LEFT, GAME_STACK, callback)
}

struct GameStack<A>(A);

impl<A: winit::application::ApplicationHandler> winit::application::ApplicationHandler
    for GameStack<A>
{
    fn new_events(
        &mut self,
        events: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        on_game_stack(|| self.0.new_events(events, cause));
    }
    fn resumed(&mut self, events: &winit::event_loop::ActiveEventLoop) {
        on_game_stack(|| self.0.resumed(events));
    }
    fn user_event(&mut self, events: &winit::event_loop::ActiveEventLoop, event: ()) {
        on_game_stack(|| self.0.user_event(events, event));
    }
    fn window_event(
        &mut self,
        events: &winit::event_loop::ActiveEventLoop,
        id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        on_game_stack(|| self.0.window_event(events, id, event));
    }
    fn device_event(
        &mut self,
        events: &winit::event_loop::ActiveEventLoop,
        id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        on_game_stack(|| self.0.device_event(events, id, event));
    }
    fn about_to_wait(&mut self, events: &winit::event_loop::ActiveEventLoop) {
        on_game_stack(|| self.0.about_to_wait(events));
    }
    fn suspended(&mut self, events: &winit::event_loop::ActiveEventLoop) {
        on_game_stack(|| self.0.suspended(events));
    }
    fn exiting(&mut self, events: &winit::event_loop::ActiveEventLoop) {
        on_game_stack(|| self.0.exiting(events));
    }
    fn memory_warning(&mut self, events: &winit::event_loop::ActiveEventLoop) {
        on_game_stack(|| self.0.memory_warning(events));
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
    let home = data_dir();
    if let Some(home) = &home {
        capture_stderr(home);
    }
    init_logging();
    horizon_protocol::install_system_srv_lookup(dns::query_srv);
    let Some(home) = home else {
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
    let mut launcher = GameStack(on_game_stack(|| launcher::Launcher::new(home.clone())));
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
