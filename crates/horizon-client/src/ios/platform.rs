//! iOS platform services. Haptics use GameController framework via horizon-gamecontroller.
use horizon_client::osk::Output;
use std::cell::RefCell;

thread_local! {
    static EDIT: RefCell<Option<String>> = const { RefCell::new(None) };
    static MULTILINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn vibrate(pulse: horizon_client::gamepad::Pulse) {
    let intensity = pulse.strong.max(pulse.weak).clamp(0.0, 1.0);
    if intensity < 0.01 || pulse.millis == 0 {
        return;
    }
    if intensity < 0.33 {
        horizon_gamecontroller::haptic_light();
    } else if intensity < 0.66 {
        horizon_gamecontroller::haptic_medium();
    } else {
        horizon_gamecontroller::haptic_rigid();
    }
}

pub fn cancel_vibration() {}

pub fn capture_mouse(_capture: bool) -> bool {
    false
}

pub fn show(text: &str) {
    show_editor(text, false);
}

pub fn show_multiline(text: &str) {
    show_editor(text, true);
}

pub fn show_secret(text: &str) {
    MULTILINE.with(|mode| mode.set(false));
    EDIT.with(|e| *e.borrow_mut() = Some(text.into()));
}

fn show_editor(text: &str, multiline: bool) {
    MULTILINE.with(|mode| mode.set(multiline));
    EDIT.with(|e| *e.borrow_mut() = Some(text.into()));
}

pub fn hide() {
    EDIT.with(|e| *e.borrow_mut() = None);
}

pub fn active() -> bool {
    EDIT.with(|e| e.borrow().is_some())
}

pub fn poll() -> Vec<Output> {
    Vec::new()
}

pub fn field_at(p: [f32; 2]) -> bool {
    horizon_client::widgets::last_targets().iter().any(|t| {
        let [x, y, w, h] = t.rect;
        t.kind == horizon_client::widgets::Kind::Field
            && (x..x + w).contains(&p[0])
            && (y..y + h).contains(&p[1])
    })
}

pub fn performance_power() -> serde_json::Value {
    serde_json::json!({})
}

pub fn cache_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|h| h.join("Library/Caches"))
}

pub fn display_scale() -> f32 {
    2.0
}

pub fn set_orientation(_orientation: crate::catalog::Orientation) {}

pub fn open_url(url: &str) -> bool {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return false;
    }
    log::info!("open_url: {url}");
    false
}

pub fn open_documents(_id: &str) -> bool {
    log::info!("open_documents: {_id}");
    false
}

pub fn open_repository() -> bool {
    open_url("https://github.com/Vasysik/horizon-android")
}

pub fn poll_gamepad_connections() {
    let _ = horizon_gamecontroller::controllers();
}
