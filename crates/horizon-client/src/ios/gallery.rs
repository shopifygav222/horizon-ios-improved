//! iOS screenshot export stubs. No MediaStore on iOS.
use std::path::Path;

pub fn save(_file: &Path) -> std::result::Result<(), String> {
    Ok(())
}

pub fn poll() -> Vec<std::result::Result<String, String>> {
    Vec::new()
}
