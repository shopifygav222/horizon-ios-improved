//! F2 screenshots added to the photo library, as Android publishes them through MediaStore. The
//! PNG also stays in the game's screenshots folder, which the Files app shows.
use std::{path::Path, sync::Mutex};

static RESULTS: Mutex<Vec<std::result::Result<String, String>>> = Mutex::new(Vec::new());

pub fn save(file: &Path) -> std::result::Result<(), String> {
    let path = file
        .to_str()
        .ok_or("The screenshot's path is not valid text")?;
    let result = if horizon_gamecontroller::save_to_photos(path) {
        Ok("Saved the screenshot to Photos".to_owned())
    } else {
        Err("Could not save the screenshot to Photos".to_owned())
    };
    RESULTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(result);
    Ok(())
}

pub fn poll() -> Vec<std::result::Result<String, String>> {
    std::mem::take(&mut *RESULTS.lock().unwrap_or_else(|e| e.into_inner()))
}
