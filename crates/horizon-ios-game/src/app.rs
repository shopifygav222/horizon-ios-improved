//! The app's start, and the event loop's handler: the game, while one runs.
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

/// The game's command line and environment, as Android's (android.rs `command_line`):
/// `--resources` in the private folder, then `horizon-args.txt` there, which may override it: one
/// argument per line, or `NAME=value` for an environment variable.
fn command_line(private: &Path) -> (Vec<String>, Vec<(String, String)>) {
    let mut args = vec![
        "--resources".to_owned(),
        private.join("resources").display().to_string(),
    ];
    let mut env = Vec::new();
    let text = std::fs::read_to_string(private.join("horizon-args.txt")).unwrap_or_default();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once('=') {
            Some((name, value))
                if !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') =>
            {
                env.push((name.to_owned(), value.to_owned()))
            }
            _ => args.push(line.to_owned()),
        }
    }
    (args, env)
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

pub fn main() {
    std::hint::black_box(&MOD_API_LEVEL);
    let (home, private) = folders();
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::create_dir_all(&private);
    capture_stderr(&home);
    let (args, env) = command_line(&private);
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
    horizon_client::ios::install();
    horizon_protocol::install_system_srv_lookup(crate::dns::query_srv);
    horizon_client::ios::set_args(args);
    // Options > Diagnostics, as Android's: the session's checkpoints, from the start.
    if let Err(error) = horizon_client::diagnostics::initialize(
        &private,
        serde_json::json!({"abi": std::env::consts::ARCH, "system": "ios"}),
    ) {
        eprintln!("Horizon: could not initialize diagnostics: {error}");
    }
    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            eprintln!("Horizon: no event loop: {e}");
            return;
        }
    };
    let mut app = App {
        home,
        private,
        game: None,
        started: false,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("Horizon: the event loop failed: {e}");
    }
}

/// The event loop's handler.
struct App {
    home: PathBuf,
    private: PathBuf,
    /// The game Starling's start made, while it runs.
    game: Option<Box<dyn ApplicationHandler>>,
    /// The first start happened (at the first `resumed`).
    started: bool,
}

impl App {
    /// Starling's start, as the desktop binary's `main` makes it, up to the game it hands over.
    fn start_game(&mut self, event_loop: &ActiveEventLoop) {
        let (home, private) = (self.home.clone(), self.private.clone());
        let game = on_game_stack(|| {
            horizon_client::entry::start(Default::default(), || {
                log::info!(
                    "Horizon: iOS, game folder {}, private files {}, arguments {:?}",
                    home.display(),
                    private.display(),
                    horizon_client::ios::args()
                );
                // Read from the app's bundle (`assets/`), as Android's from the APK.
                horizon_client::preparer::put_shader_pack(&home);
                // The saved Microsoft sign-in renews while the game loads.
                horizon_client::accounts::resume();
            });
            horizon_client::ios::started_game()
        });
        let Some(mut game) = game else {
            let why = horizon_client::ios::take_failure()
                .unwrap_or_else(|| "the game ended before it started".to_owned());
            log::error!("Horizon: {why}");
            return;
        };
        on_game_stack(|| {
            game.new_events(event_loop, StartCause::Init);
            game.resumed(event_loop);
        });
        self.game = Some(game);
    }

    /// After each callback: a game that asked to end (Quit) or to start again ends here, saving
    /// what it holds, as the desktop's does when its loop returns.
    fn after_callback(&mut self, event_loop: &ActiveEventLoop) {
        let exit = winit::platform::ios::take_exit_request();
        let restart = horizon_client::ios::take_restart();
        if !exit && !restart {
            return;
        }
        if let Some(mut game) = self.game.take() {
            on_game_stack(|| {
                game.exiting(event_loop);
                drop(game);
            });
        }
        // The game ended as it meant to: the recording stops and the session is marked clean.
        horizon_client::performance::stop();
        if !restart {
            horizon_client::diagnostics::finish();
        }
        if restart {
            self.start_game(event_loop);
        }
    }

    /// `callback` with the game on its stack, then what it asked for.
    fn with_game(
        &mut self,
        event_loop: &ActiveEventLoop,
        callback: impl FnOnce(&mut dyn ApplicationHandler),
    ) {
        if let Some(game) = self.game.as_deref_mut() {
            on_game_stack(|| callback(game));
            self.after_callback(event_loop);
        }
    }
}

impl ApplicationHandler for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.with_game(event_loop, |game| game.new_events(event_loop, cause));
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.started {
            self.started = true;
            self.start_game(event_loop);
            self.after_callback(event_loop);
            return;
        }
        self.with_game(event_loop, |game| game.resumed(event_loop));
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ()) {
        self.with_game(event_loop, |game| game.user_event(event_loop, event));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.with_game(event_loop, |game| game.window_event(event_loop, id, event));
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.with_game(event_loop, |game| game.device_event(event_loop, id, event));
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Files other apps handed this one ("Open in", the share sheet): into the import inbox,
        // which the game installs from.
        for file in winit::platform::ios::take_opened_files() {
            horizon_client::ios::put_in_inbox(&file.path);
        }
        self.with_game(event_loop, |game| game.about_to_wait(event_loop));
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.with_game(event_loop, |game| game.suspended(event_loop));
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(mut game) = self.game.take() {
            on_game_stack(|| game.exiting(event_loop));
        }
    }

    /// iOS is short of memory: the allocators give back every free page now, before iOS ends
    /// the app for what they keep.
    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        log::warn!("Horizon: iOS is short of memory: freeing what the game does not use");
        horizon_threads::release_free_memory();
        self.with_game(event_loop, |game| game.memory_warning(event_loop));
    }
}
