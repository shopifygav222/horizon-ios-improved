//! iOS system input and platform services, called from the Rust client: the same functions as
//! Android's, with UIKit in place of JNI (the calls themselves live in horizon-gamecontroller).
use horizon_client::osk::Output;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};
use winit::window::Window;

thread_local! {
    static EDIT: RefCell<Option<String>> = const { RefCell::new(None) };
    static MULTILINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static WINDOW: RefCell<Weak<Window>> = const { RefCell::new(Weak::new()) };
}

/// The window on screen now: the launcher's, or the game's once it starts. The software keyboard
/// belongs to it.
pub fn attach_window(window: &Arc<Window>) {
    WINDOW.with(|slot| *slot.borrow_mut() = Arc::downgrade(window));
}

pub(crate) fn with_window<R>(f: impl FnOnce(&Window) -> R) -> Option<R> {
    WINDOW
        .with(|slot| slot.borrow().upgrade())
        .map(|window| f(&window))
}

pub fn set_orientation(orientation: crate::catalog::Orientation) {
    crate::display::request_orientation(orientation);
}

pub fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join("Library/Caches"))
}

pub fn open_repository() -> bool {
    open_url("https://github.com/Vasysik/horizon-android")
}

/// Show one of the launcher's folders in the Files app (Android's document IDs, resolved the way
/// its DocumentsProvider does). Horizon's Documents folder is shared with Files.
pub fn open_documents(id: &str) -> bool {
    let Some(folder) = document_folder(&crate::backend::data_dir(), id) else {
        return false;
    };
    if std::fs::create_dir_all(&folder).is_err() {
        return false;
    }
    let Ok(url) = url::Url::from_directory_path(&folder) else {
        return false;
    };
    horizon_gamecontroller::open_url(&format!("shareddocuments://{}", url.path()))
}

fn document_folder(home: &Path, id: &str) -> Option<PathBuf> {
    if id == crate::document_store::ROOT {
        return Some(home.to_owned());
    }
    if id == "reports" {
        return Some(home.join("performance"));
    }
    let (profile, folder) = id.split_once('/')?;
    let profile: u32 = profile.strip_prefix('p')?.parse().ok()?;
    let root = if profile == 0 {
        home.to_owned()
    } else {
        home.join("profiles").join(format!("profile-{profile}"))
    };
    let folder = match folder {
        "mods" => "horizon-mods",
        "packs" => "resourcepacks",
        "shaders" => "shaderpacks",
        "worlds" => "saves",
        "configuration" => "",
        _ => return None,
    };
    Some(root.join(folder))
}

pub fn open_url(url: &str) -> bool {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return false;
    }
    horizon_gamecontroller::open_url(url)
}

pub fn copy_login_code(text: &str) -> bool {
    horizon_gamecontroller::set_clipboard_text(text)
}

pub fn active() -> bool {
    EDIT.with(|e| e.borrow().is_some())
}

/// Quit, as Android's `Activity.finish`: UIKit has no call for it and winit ignores `exit` on iOS.
pub fn finish() {
    std::process::exit(0);
}

pub fn capture_mouse(capture: bool) -> bool {
    crate::native_ime::capture_mouse(capture)
}

pub fn show_secret(text: &str) {
    MULTILINE.with(|mode| mode.set(false));
    if crate::native_ime::show_secret(text) {
        EDIT.with(|e| *e.borrow_mut() = Some(text.into()));
    }
}

pub fn show(text: &str) {
    show_editor(text, false);
}

fn show_editor(text: &str, multiline: bool) {
    MULTILINE.with(|mode| mode.set(multiline));
    if crate::native_ime::show(text) {
        EDIT.with(|e| *e.borrow_mut() = Some(text.into()));
    } else {
        log::warn!("iOS system keyboard is unavailable");
    }
}

pub fn hide() {
    EDIT.with(|e| *e.borrow_mut() = None);
    crate::native_ime::hide();
}

/// Android diffs the IME's text here; UIKit's keyboard types through KeyboardInput events instead,
/// so `native_ime::text` has nothing and this stays empty.
pub fn poll() -> Vec<Output> {
    EDIT.with(|edit| {
        let mut edit = edit.borrow_mut();
        let Some(old) = edit.as_mut() else {
            return vec![];
        };
        let Some(new) = crate::native_ime::text() else {
            return vec![];
        };
        if new == *old {
            return vec![];
        }
        let out =
            horizon_client::ime_edit::editor_changes(old, &new, MULTILINE.with(|mode| mode.get()));
        *old = new;
        out
    })
}

/// Only Microsoft's device-code verification pages can be opened by the login panel.
pub fn open_login_url(url: &str) -> Result<(), String> {
    let fixed = [
        "https://www.microsoft.com/link",
        "https://microsoft.com/link",
        "https://microsoft.com/devicelogin",
        "https://www.microsoft.com/devicelogin",
        "https://aka.ms/mce-reviewappid",
    ]
    .contains(&url);
    let verification = url::Url::parse(url).is_ok_and(|u| {
        let pairs = u.query_pairs().collect::<Vec<_>>();
        u.scheme() == "https"
            && matches!(u.host_str(), Some("microsoft.com" | "www.microsoft.com"))
            && matches!(u.path(), "/link" | "/devicelogin")
            && u.username().is_empty()
            && u.password().is_none()
            && u.port_or_known_default() == Some(443)
            && u.fragment().is_none()
            && pairs.len() == 1
            && pairs[0].0 == "otc"
            && pairs[0].1.len() <= 32
            && pairs[0]
                .1
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    let authorize = url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("login.microsoftonline.com")
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
            && u.path() == "/consumers/oauth2/v2.0/authorize"
            && u.fragment().is_none()
    });
    if !fixed && !authorize && !verification {
        return Err("Unexpected Microsoft sign-in URL.".into());
    }
    if horizon_gamecontroller::open_url(url) {
        Ok(())
    } else {
        Err("Could not open the browser for Microsoft sign-in.".into())
    }
}

/// GameController keeps the active pad current; hand it to the game as Android's event-fed pads are.
pub fn poll_gamepad_connections() {
    horizon_client::android_runtime::set_external_gamepad(
        horizon_gamecontroller::active().map(|pad| pad_state(&pad)),
    );
}

fn pad_state(pad: &horizon_gamecontroller::Snapshot) -> horizon_client::gamepad::PadState {
    use horizon_client::gamepad::{Button, PadState};
    let mut state = PadState {
        buttons: 0,
        left: pad.left_stick,
        right: pad.right_stick,
        triggers: pad.triggers,
    };
    for (button, down) in [
        (Button::South, pad.a),
        (Button::East, pad.b),
        (Button::West, pad.x),
        (Button::North, pad.y),
        (Button::LeftBumper, pad.left_shoulder),
        (Button::RightBumper, pad.right_shoulder),
        (Button::Select, pad.options),
        (Button::Start, pad.menu),
        (Button::LeftStick, pad.left_stick_button),
        (Button::RightStick, pad.right_stick_button),
        (Button::DPadUp, pad.dpad_up),
        (Button::DPadDown, pad.dpad_down),
        (Button::DPadLeft, pad.dpad_left),
        (Button::DPadRight, pad.dpad_right),
    ] {
        state.set(button, down);
    }
    state
}

/// UIKit's screen scale, the dp-to-pixel factor Android reads from DisplayMetrics.
pub fn display_density() -> f32 {
    horizon_gamecontroller::screen_scale()
        .filter(|scale| scale.is_finite())
        .map_or(2.0, |scale| scale as f32)
        .clamp(1.0, 5.0)
}
