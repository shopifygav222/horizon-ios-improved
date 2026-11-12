//! Frontend-independent game launch parameters, and the game started with them: Starling's own
//! start (`horizon_client::entry::start`, as the desktop binary's `main`), whose game is handed to
//! this app's event loop (`horizon_client::ios::take_game`). No launcher widgets or config reads.
//! Who plays is Starling's to say (horizon_client::accounts): no session file, no name argument.
use std::path::PathBuf;

#[derive(Clone)]
pub(crate) struct LaunchRequest {
    pub resources: PathBuf,
    pub game_home: PathBuf,
    pub server: String,
    pub control_socket: Option<PathBuf>,
    pub extra_arguments: Vec<String>,
}
impl LaunchRequest {
    pub fn arguments(&self) -> Vec<String> {
        let mut args = vec![
            "--resources".into(),
            self.resources.to_string_lossy().into_owned(),
            "--game-dir".into(),
            self.game_home.to_string_lossy().into_owned(),
        ];
        if let Some(socket) = &self.control_socket {
            args.extend([
                "--control-socket".into(),
                socket.to_string_lossy().into_owned(),
            ]);
        }
        if !self.server.is_empty() {
            args.extend(["--server".into(), self.server.clone()]);
        }
        args.extend(self.extra_arguments.iter().cloned());
        args
    }
}

/// Starling's game, started as its desktop binary starts it, ready for the event loop.
pub(crate) fn start(
    request: LaunchRequest,
) -> horizon_client::Result<Box<dyn winit::application::ApplicationHandler>> {
    if !request.game_home.is_absolute() || !request.resources.is_absolute() {
        return Err("Game and resource directories must be absolute".into());
    }
    let previous = std::env::current_dir()?;
    std::env::set_current_dir(&request.game_home)?;
    let arguments = request.arguments();
    horizon_client::ios::set_args(arguments.clone());
    let game_home = request.game_home.clone();
    horizon_client::entry::start(Default::default(), || {
        log::info!(
            "Horizon: iOS, game folder {}, arguments {arguments:?}",
            game_home.display()
        );
        // Starling Visuals, from the app's bundle (`assets/`), as Android's from the APK.
        horizon_client::preparer::put_shader_pack(&game_home);
        // The profile Starling keeps (the launcher's accounts page and the title screen's
        // button pick it) plays; a saved sign-in renews while the game loads.
        horizon_client::accounts::resume();
    });
    match horizon_client::ios::started_game() {
        Some(game) => Ok(game),
        None => {
            let _ = std::env::set_current_dir(previous);
            Err(horizon_client::ios::take_failure()
                .unwrap_or_else(|| "The game ended before it started".to_owned())
                .into())
        }
    }
}
