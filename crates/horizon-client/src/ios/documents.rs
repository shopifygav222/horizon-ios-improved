//! iOS document import stubs. iOS uses UIFileSharingEnabled for file access
//! instead of Android's SAF (Storage Access Framework).
use horizon_client::content_files::Kind;
use horizon_client::hud::{HudAtlas, HudBatch};
use horizon_client::ui_theme::Ui;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Notice {
    pub text: String,
    pub changed: bool,
    pub kind: Option<Kind>,
}

#[derive(Clone, Copy)]
pub enum Choice {
    KeepBoth,
    Replace,
    Cancel,
}

pub fn busy() -> bool {
    false
}

pub fn poll() -> Option<Notice> {
    None
}

pub fn draw(_batch: &mut HudBatch, _atlas: &HudAtlas) {}

pub fn draw_ui(_ui: &mut Ui<'_>) {}

pub fn touch(_p: [f32; 2]) -> Option<Choice> {
    None
}

pub fn pending_notice() -> Option<Notice> {
    None
}

pub fn handle_event(_event: &winit::event::WindowEvent) -> bool {
    false
}

pub fn begin(_kind: Kind, _home: PathBuf) -> Result<(), String> {
    Err("Document import is not yet supported on iOS. Use the Files app to add files to the Horizon folder.".into())
}

pub fn begin_into_dir(_target_dir: PathBuf) -> Result<(), String> {
    Err("Document import is not yet supported on iOS. Use the Files app.".into())
}

pub fn export_file(
    _home: &Path,
    _profile: u32,
    _file_path: &Path,
) -> Result<String, String> {
    Err("File export is not yet supported on iOS. Files are available in the Files app.".into())
}
