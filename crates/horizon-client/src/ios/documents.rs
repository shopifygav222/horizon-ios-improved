//! Document import on iOS. The Documents folder is shared with the Files app
//! (UIFileSharingEnabled), so files are copied in there rather than through Android's
//! Storage Access Framework picker; these keep Android's interface for the shared launcher code.
use horizon_client::content_files::Kind;
use horizon_client::hud::{HudAtlas, HudBatch};
use horizon_client::ui_theme::Ui;
use std::path::{Path, PathBuf};

const USE_FILES: &str = "Copy it into the Horizon folder with the Files app.";

#[derive(Clone)]
pub struct Notice {
    pub text: String,
    pub changed: bool,
    pub kind: Option<Kind>,
}

pub fn busy() -> bool {
    false
}

pub fn poll() -> Option<Notice> {
    None
}

pub fn begin(_kind: Kind, _home: PathBuf) -> Result<(), String> {
    Err(format!("Importing here is not available on iOS. {USE_FILES}"))
}

pub fn begin_appearance(
    _home: PathBuf,
    _kind: crate::skins::library::Kind,
) -> Result<(), String> {
    Err(format!("Importing a skin here is not available on iOS. {USE_FILES}"))
}

pub fn begin_into_dir(_target_dir: PathBuf) -> Result<(), String> {
    Err(format!("Importing here is not available on iOS. {USE_FILES}"))
}

pub fn export_file(_home: &Path, _profile: u32, _file_path: &Path) -> Result<String, String> {
    Err("Exporting is not available on iOS. Your files are in the Horizon folder in the Files app.".into())
}

pub fn draw(_batch: &mut HudBatch, _atlas: &HudAtlas) {}

pub fn draw_ui(_ui: &mut Ui<'_>) {}

pub fn handle_event(_event: &winit::event::WindowEvent) -> bool {
    false
}
