//! iOS software keyboard management.
//! UIKit manages the keyboard lifecycle through first-responder status.
use std::sync::atomic::{AtomicBool, Ordering};

static KEYBOARD_VISIBLE: AtomicBool = AtomicBool::new(false);

pub fn is_visible() -> bool {
    KEYBOARD_VISIBLE.load(Ordering::Relaxed)
}

pub fn show() {
    KEYBOARD_VISIBLE.store(true, Ordering::Relaxed);
}

pub fn hide() {
    KEYBOARD_VISIBLE.store(false, Ordering::Relaxed);
    crate::platform::hide();
}

pub fn refresh_bounds() {}

pub fn set_text(_text: &str) {}

pub fn text() -> Option<String> {
    None
}
