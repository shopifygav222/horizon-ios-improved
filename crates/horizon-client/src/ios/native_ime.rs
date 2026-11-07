//! The iOS system keyboard. winit's view stays UIKit's first responder (so hardware keys reach the
//! game); `set_ime_allowed` swaps in the software keyboard, whose characters then arrive as
//! KeyboardInput events: the path an Android hardware keyboard takes, which every text field
//! already handles. So there is no IME text to mirror, unlike Android's.

pub fn show(_text: &str) -> bool {
    crate::platform::with_window(|window| window.set_ime_allowed(true)).is_some()
}

/// UIKit's keyboard cannot be asked to hide what it types from outside its own text views.
pub fn show_secret(text: &str) -> bool {
    show(text)
}

pub fn hide() {
    let _ = crate::platform::with_window(|window| window.set_ime_allowed(false));
}

pub fn text() -> Option<String> {
    None
}

/// Pointer lock follows the window's own grab; the game falls back to touch-style capture.
pub fn capture_mouse(_capture: bool) -> bool {
    false
}

/// The keyboard's top edge in renderer pixels while it is open, so chat and the login panel stay
/// above it; `None` while it is closed or floating, as on Android.
pub fn viewport_bottom() -> Option<u32> {
    horizon_gamecontroller::keyboard_top()
}

/// Files are added through the Files app (the Documents folder is shared), not a picker.
pub fn pick_document() -> bool {
    false
}

pub fn configure_screen(immersive: bool, avoid_cutout: bool) {
    crate::display::configure_screen(immersive, avoid_cutout);
}

/// UIKit's layout guide is read each time `viewport_bottom` asks; nothing to queue.
pub fn refresh_bounds() {}
