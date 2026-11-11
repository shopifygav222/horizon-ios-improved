//! iOS: the control system's keyboard, haptics and folders (Android's are mobile_platform.rs).
//! The keyboard is the window's own, which game/touch.rs shows while a field here is `active`;
//! what it types comes as key events, which `key` queues for `poll` in the shape Android's text
//! editor gives them.
use super::Output;
use std::sync::Mutex;

#[derive(Default)]
struct State {
    active: bool,
    typed: Vec<Output>,
}
static STATE: Mutex<State> = Mutex::new(State {
    active: false,
    typed: Vec::new(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}
pub fn active() -> bool {
    state().active
}
/// The keyboard for a field of the control system (a preset's name, a custom button's text).
/// The field holds its own text: what is typed from here on is added to it.
pub fn show(_text: &str) {
    let mut s = state();
    s.active = true;
    s.typed.clear();
}
pub fn hide() {
    let mut s = state();
    s.active = false;
    s.typed.clear();
}
pub fn poll() -> Vec<Output> {
    std::mem::take(&mut state().typed)
}
/// A key from the keyboard while a field here has it: queued for `poll`. True when it was taken
/// (every key is, releases too, so none reaches the game under the field).
pub fn key(event: &winit::event::KeyEvent) -> bool {
    use winit::keyboard::{Key, NamedKey};
    let mut s = state();
    if !s.active {
        return false;
    }
    if event.state != winit::event::ElementState::Pressed {
        return true;
    }
    let typed = match &event.logical_key {
        Key::Named(NamedKey::Backspace) => Some(Output::Backspace),
        Key::Named(NamedKey::Enter) => Some(Output::Enter),
        Key::Named(NamedKey::Escape) => Some(Output::Close),
        _ => event
            .text
            .as_ref()
            .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
            .map(|text| Output::Text(text.to_string())),
    };
    s.typed.extend(typed);
    true
}
pub fn vibrate(p: horizon_client::gamepad::Pulse) {
    crate::android::vibrate(p.millis);
}
/// UIKit's haptics are single taps: nothing runs on to cancel.
pub fn cancel_vibration() {}
pub fn open_documents(folder: &str) -> bool {
    // The Files app takes a whole path: the game folder's, which is where the game runs.
    let dir = std::env::current_dir().unwrap_or_default().join(folder);
    horizon_client::platform::open_folder(&dir).unwrap_or(false)
}
pub fn field_at(p: [f32; 2]) -> bool {
    horizon_client::widgets::last_targets().iter().any(|t| {
        let [x, y, w, h] = t.rect;
        matches!(t.kind, horizon_client::widgets::Kind::Field)
            && p[0] >= x
            && p[0] < x + w
            && p[1] >= y
            && p[1] < y + h
    })
}
