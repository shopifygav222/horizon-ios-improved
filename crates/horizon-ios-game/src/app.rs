//! The app's start (its folders, environment, logs and platform hooks), and the event loop's
//! handler: Horizon Android's launcher, which runs Starling's game while one plays, every callback
//! on the game's stack.
use std::path::{Path, PathBuf};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

/// The stack every event-loop callback runs on, and how much of it must be left before one: the
/// desktop's main thread has 8 MB; a joining splitscreen player's Game is ~67 KB of it alone.
const GAME_STACK: usize = 16 << 20;
const GAME_STACK_LEFT: usize = 8 << 20;

/// The mod API level this game serves, kept in the program as the desktop's main.rs keeps it.
static MOD_API_LEVEL: [u8; 27] = horizon_mods::api_level::MARK;

fn on_game_stack<R>(callback: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(GAME_STACK_LEFT, GAME_STACK, callback)
}

/// The game folder (Documents) and the private one (Library/Application Support/Starling).
fn folders() -> (PathBuf, PathBuf) {
    let home = std::env::var_os("HOME").map_or_else(std::env::temp_dir, PathBuf::from);
    (
        home.join("Documents"),
        home.join("Library/Application Support/Starling"),
    )
}

/// Standard error, which iOS sends nowhere a player can read, into the game folder's
/// `logs/stderr.log` (the last run's kept as `previous-stderr.log`): what a crash leaves there (a
/// panic, a native abort's message) reaches a bug report.
fn capture_stderr(home: &Path) {
    use std::os::fd::AsRawFd;
    let dir = home.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::rename(dir.join("stderr.log"), dir.join("previous-stderr.log"));
    if let Ok(file) = std::fs::File::create(dir.join("stderr.log")) {
        // SAFETY: both descriptors are open; dup2 points standard error at the file, which stays
        // open through it once `file` closes its own descriptor.
        unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) };
    }
}

/// The environment `horizon-args.txt` in the private folder sets, as Android's (android.rs
/// `command_line`): its `NAME=value` lines (`HORIZON_GPU_CULL=0`...), for tests.
fn test_environment(private: &Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(private.join("horizon-args.txt")).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .filter(|(name, _)| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        })
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

/// The RAM limit Starling's governor keeps the game under (ram_limit.rs: far terrain off, then a
/// lower render distance, before the game would hold more), in MiB: 85% of what iOS lets this
/// app hold, which iOS enforces by ending the app without a word. Empty when iOS doesn't say.
fn ram_limit_mb() -> String {
    // What the app holds now (its start: little), and what iOS lets it take beyond that.
    let held = 64 << 20;
    horizon_gamecontroller::available_memory()
        .filter(|&available| available > 0)
        .map_or_else(String::new, |available| {
            (((available + held) / 100 * 85) >> 20).to_string()
        })
}

/// Starling's log's console: standard error (`logs/stderr.log`), and the launcher's diagnostics,
/// which its Diagnostics export takes.
fn console(level: log::Level, message: &str) {
    eprintln!("Horizon {level}: {message}");
    crate::diagnostics::log_line(level, "horizon", message);
}

pub fn main() {
    std::hint::black_box(&MOD_API_LEVEL);
    let (home, private) = folders();
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::create_dir_all(&private);
    capture_stderr(&home);
    let env = test_environment(&private);
    let ram_limit = ram_limit_mb();
    // SAFETY: no other thread of this program runs yet; the game reads these from its threads
    // afterwards.
    unsafe {
        std::env::set_var("HORIZON_HOME", &home);
        std::env::set_var("HORIZON_PRIVATE", &private);
        for (name, value) in &env {
            std::env::set_var(name, value);
        }
        // As Starling's Android app: no telemetry unless the test arguments ask, and no Discord
        // on a phone.
        for (name, value) in [
            ("HORIZON_TELEMETRY_URL", "off"),
            ("HORIZON_NO_DISCORD", "1"),
            ("HORIZON_MEMORY_LIMIT_MB", ram_limit.as_str()),
        ] {
            if std::env::var_os(name).is_none() && !value.is_empty() {
                std::env::set_var(name, value);
            }
        }
    }
    if let Err(e) = std::env::set_current_dir(&home) {
        eprintln!("Horizon: cannot enter {}: {e}", home.display());
    }
    // One log for the launcher and the game: Starling's (logs/latest.log), opened now.
    horizon_client::logging::set_console(console);
    horizon_client::logging::init(&home, "Render thread");
    crate::backend::initialize(home.clone());
    if let Err(error) = crate::diagnostics::initialize(&home, serde_json::json!({})) {
        log::error!("Cannot initialize local diagnostics: {error}");
    }
    if let Some(cache) = crate::platform::cache_dir() {
        horizon_client::android_runtime::configure_cache_dir(cache);
    }
    horizon_client::ios::install();
    horizon_protocol::install_system_srv_lookup(crate::dns::query_srv);
    horizon_client::crash::install(home.clone(), "client");
    crate::exit_reports::install(&home);
    let _ = std::fs::remove_file(home.join("ios-error.txt"));
    let launcher = on_game_stack(|| crate::launcher::Launcher::new(home.clone()));
    let mut app = App { launcher };
    // UIKit's main loop never returns; this continues only when winit could not start it.
    let result = EventLoop::new()
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })
        .and_then(|events| events.run_app(&mut app).map_err(Into::into));
    if let Err(error) = result {
        log::error!("Horizon: {error}");
        crate::diagnostics::record_error(&error.to_string());
        horizon_client::crash::report_error(&home, "client", &*error);
        let _ = std::fs::write(home.join("ios-error.txt"), error.to_string());
    }
    crate::diagnostics::finish();
}

/// The event loop's handler: the launcher, on the game's stack.
struct App {
    launcher: crate::launcher::Launcher,
}

impl App {
    /// `callback` on the game's stack, then what a running game asked for: iOS can't end an app
    /// (winit records the exit, `take_exit_request`) nor start it again (`take_restart`), so a game
    /// that quits goes back to the launcher, and one that restarts plays again from there.
    fn run(&mut self, callback: impl FnOnce(&mut crate::launcher::Launcher)) {
        on_game_stack(|| callback(&mut self.launcher));
        let exit = winit::platform::ios::take_exit_request();
        let restart = horizon_client::ios::take_restart();
        if self.launcher.playing() && (exit || restart) {
            horizon_client::android_runtime::return_to_launcher();
            if restart {
                self.launcher.play_again();
            }
        }
        // Files other apps handed this one ("Open in", the share sheet): into the import inbox,
        // which the game installs from.
        for file in winit::platform::ios::take_opened_files() {
            horizon_client::ios::put_in_inbox(&file.path);
        }
    }
}

impl ApplicationHandler for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.run(|launcher| launcher.new_events(event_loop, cause));
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.run(|launcher| launcher.resumed(event_loop));
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ()) {
        self.run(|launcher| launcher.user_event(event_loop, event));
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.run(|launcher| launcher.window_event(event_loop, id, event));
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.run(|launcher| launcher.device_event(event_loop, id, event));
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.run(|launcher| launcher.about_to_wait(event_loop));
    }
    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.run(|launcher| launcher.suspended(event_loop));
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        on_game_stack(|| self.launcher.exiting(event_loop));
    }
    /// iOS is short of memory: the allocators give back every free page now, before iOS ends
    /// the app for what they keep.
    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        log::warn!("Horizon: iOS is short of memory: freeing what the game does not use");
        horizon_threads::release_free_memory();
        self.run(|launcher| launcher.memory_warning(event_loop));
    }
}
