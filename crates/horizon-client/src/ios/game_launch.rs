//! Frontend-independent game launch parameters. No launcher widgets or config reads.
use std::path::PathBuf;

#[derive(Clone)]
pub(crate) struct LaunchRequest {
    pub resources: PathBuf,
    pub game_home: PathBuf,
    pub session_file: Option<PathBuf>,
    pub offline_name: String,
    pub server: String,
    pub control_socket: Option<PathBuf>,
    pub extra_arguments: Vec<String>,
    pub open_gl: bool,
    pub gpu_batching: bool,
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
        if let Some(session) = &self.session_file {
            args.extend([
                "--session-file".into(),
                session.to_string_lossy().into_owned(),
            ]);
        } else if !self.offline_name.is_empty() {
            args.extend(["--name".into(), self.offline_name.clone()]);
        }
        if !self.server.is_empty() {
            args.extend(["--server".into(), self.server.clone()]);
        }
        args.extend(self.extra_arguments.iter().cloned());
        args
    }
}

#[cfg(target_os = "ios")]
pub(crate) fn start(
    request: LaunchRequest,
) -> horizon_client::Result<Box<dyn winit::application::ApplicationHandler>> {
    if !request.game_home.is_absolute() || !request.resources.is_absolute() {
        return Err("Game and resource directories must be absolute".into());
    }
    let previous = std::env::current_dir()?;
    std::env::set_current_dir(&request.game_home)?;
    horizon_client::android_runtime::configure_graphics(request.open_gl);
    horizon_client::android_runtime::configure_gpu(request.gpu_batching);
    crate::game::android_reset_bundled_mods();
    let result = crate::game::android_game(request.arguments());
    if result.is_err() {
        let _ = std::env::set_current_dir(previous);
    }
    result
}
