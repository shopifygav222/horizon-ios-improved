//! iOS hooks of the game (this port): what `game` reaches through `crate::android::` on Android,
//! which this module answers on iOS under that name (lib.rs), so the phones share the game's
//! code; the platform's hooks (platform.rs) and probes (system_stats.rs); and the game handed to
//! the app's event loop, which UIKit makes once a process (horizon-ios-game runs it).
//!
//! Every UIKit, Security and CoreMotion call is horizon-gamecontroller's: this crate forbids
//! unsafe code. The game folder is the app's Documents, the Files app's "Starling" folder, as
//! Android's is `Starling` in shared storage; the private files (resources, caches, the import
//! inbox) are in Library/Application Support, as Android's are in the app's own files.
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use winit::application::ApplicationHandler;

/// The game's command line, the desktop's arguments: set by the app before each start.
static ARGS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The player chose to start the game again (a world's own mods, world_content.rs).
static RESTART: AtomicBool = AtomicBool::new(false);
/// Why the last start ended before it had a game to run (entry.rs).
static FAILURE: Mutex<Option<String>> = Mutex::new(None);

/// The Keychain service of the key [`seal`] seals with.
const SECRETS: &str = "dev.horizon.starling.secrets";
const SEAL_KEY: &str = "seal-key";
/// ChaCha20-Poly1305's nonce, before each sealed text.
const NONCE: usize = 12;

type Handler = Box<dyn ApplicationHandler>;

thread_local! {
    /// The game the last start made, for the app's event loop to run.
    static GAME: RefCell<Option<Handler>> = const { RefCell::new(None) };
    /// The gyroscope's last sample time, for what it turned since.
    static GYRO_AT: Cell<f64> = const { Cell::new(0.0) };
}

/// The command line of the next start.
pub fn set_args(args: Vec<String>) {
    *ARGS.lock().unwrap_or_else(PoisonError::into_inner) = args;
}

/// The command line the desktop binary would get, without the program name.
pub fn args() -> Vec<String> {
    ARGS.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// `game::run_loading`: the loading screen, then the game, to run on the app's event loop.
pub fn take_game(game: Handler) -> crate::Result<()> {
    GAME.with(|slot| *slot.borrow_mut() = Some(game));
    Ok(())
}

/// What the last start handed over (once).
pub fn started_game() -> Option<Handler> {
    GAME.with(|slot| slot.borrow_mut().take())
}

/// The start failed before it had a game: the app shows `why` instead of ending.
pub fn game_failed(why: &str) {
    *FAILURE.lock().unwrap_or_else(PoisonError::into_inner) = Some(why.to_owned());
}

/// Why the last start failed, once.
pub fn take_failure() -> Option<String> {
    FAILURE.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// Android restarts its activity in a new process; iOS can't, so the app ends the game and
/// starts it again in this one (horizon-ios-game reads [`take_restart`]).
pub fn restart_app() {
    RESTART.store(true, Ordering::SeqCst);
}

/// Whether the game asked to be started again since the last call.
pub fn take_restart() -> bool {
    RESTART.swap(false, Ordering::SeqCst)
}

/// The app's version, from its Info.plist; never a build that updates itself.
pub fn app_version() -> Option<(String, bool)> {
    horizon_gamecontroller::app_version()
        .filter(|v| !v.is_empty())
        .map(|v| (v, false))
}

/// A short buzz of the phone: the touch controls' feedback. iOS's taps come in three weights, not
/// lengths: the break buzz (25 ms) is the middle one.
pub fn vibrate(millis: u32) {
    match millis {
        0 => {}
        1..=15 => horizon_gamecontroller::haptic_light(),
        16..=30 => horizon_gamecontroller::haptic_medium(),
        _ => horizon_gamecontroller::haptic_rigid(),
    }
}

/// Start or stop listening to the gyroscope; false when the phone has none.
pub fn gyroscope(on: bool) -> bool {
    GYRO_AT.with(|at| at.set(0.0));
    if on {
        horizon_gamecontroller::gyro_start()
    } else {
        horizon_gamecontroller::gyro_stop();
        true
    }
}

/// What the gyroscope turned since the last call (radians about the device's x and y, the axes
/// Android's are) and the interface's rotation (Android's `Display.getRotation`). CoreMotion keeps
/// the latest rate; it turned for the time since the sample used last.
pub fn take_gyro() -> Option<[f32; 3]> {
    let ([x, y, _], time) = horizon_gamecontroller::gyro_sample()?;
    let last = GYRO_AT.with(|at| at.replace(time));
    let seconds = (time - last) as f32;
    // The first sample, or one after a pause, turns nothing.
    if last == 0.0 || !(seconds > 0.0 && seconds < 0.1) {
        return None;
    }
    let rotation = horizon_gamecontroller::interface_rotation() as f32;
    Some([x * seconds, y * seconds, rotation])
}

/// How far the on-screen keyboard covers the window from the bottom, in pixels.
pub fn ime_height() -> u32 {
    horizon_gamecontroller::keyboard_height()
}

/// Registers the platform's hooks and probes, before the game starts.
pub fn install() {
    crate::platform::set(crate::platform::Hooks {
        open_folder,
        pick_file,
        clipboard_get,
        clipboard_set,
        open_link,
        seal,
        open: open_sealed,
        library_dir,
        asset,
        keep_working,
    });
    crate::system_stats::set_probes(crate::system_stats::Probes {
        heap_bytes: horizon_gamecontroller::heap_in_use,
        thermal_status,
        thermal_headroom,
    });
    if let Some(ram) = horizon_gamecontroller::physical_memory() {
        // Every Apple chip iOS 16 runs on clocks its fast cores at 2.39 GHz or more, and every
        // one in a device with 7 GB or more at 3.2 GHz or more: the memory decides the tier.
        let mhz = if ram >= 7 << 30 { 3200 } else { 2390 };
        crate::system_stats::set_device(ram, mhz);
    }
}

/// A file picked, opened in the app or shared to it: into the import inbox as `<name>.part`,
/// renamed once complete (import.rs).
pub fn put_in_inbox(path: &Path) {
    let Some(name) = path.file_name() else {
        return;
    };
    let inbox = crate::import::inbox_dir();
    let part = inbox.join(format!("{}.part", name.to_string_lossy()));
    let done = inbox.join(name);
    let copied = std::fs::create_dir_all(&inbox)
        .and_then(|()| std::fs::copy(path, &part))
        .and_then(|_| std::fs::rename(&part, &done));
    match copied {
        Ok(()) => log::info!("Horizon: {} is in the import inbox", done.display()),
        Err(e) => {
            log::warn!("Horizon: {} did not reach the import inbox: {e}", path.display());
            let _ = std::fs::remove_file(&part);
        }
    }
}

/// Files' view of `dir`: the app's Documents are its "Starling" folder there.
fn open_folder(dir: &Path) -> bool {
    horizon_gamecontroller::open_url(&format!("shareddocuments://{}", url_path(dir)))
}

/// `path` as a URL's path: unreserved bytes and `/` as they are, the rest percent-encoded.
fn url_path(path: &Path) -> String {
    let mut out = String::new();
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn pick_file() -> bool {
    horizon_gamecontroller::pick(
        horizon_gamecontroller::Pick::Files,
        Box::new(|paths| {
            for path in paths {
                put_in_inbox(&path);
            }
        }),
    )
}

fn clipboard_get() -> Option<String> {
    horizon_gamecontroller::clipboard_text().filter(|text| !text.is_empty())
}

fn clipboard_set(text: &str) -> bool {
    horizon_gamecontroller::set_clipboard_text(text)
}

fn open_link(url: &str) -> bool {
    url.starts_with("https://") && horizon_gamecontroller::open_url(url)
}

/// The key the Keychain keeps for this device only, made at the first seal.
fn seal_key() -> Option<[u8; 32]> {
    match horizon_gamecontroller::keychain_get(SECRETS, SEAL_KEY) {
        Ok(Some(key)) if key.len() == 32 => key.try_into().ok(),
        Ok(_) => {
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).ok()?;
            match horizon_gamecontroller::keychain_set(SECRETS, SEAL_KEY, &key) {
                Ok(()) => Some(key),
                Err(status) => {
                    log::warn!("Horizon: the Keychain kept no key (OSStatus {status})");
                    None
                }
            }
        }
        Err(status) => {
            log::warn!("Horizon: the Keychain gave no key (OSStatus {status})");
            None
        }
    }
}

/// `data` sealed with the Keychain's key: the nonce, then ChaCha20-Poly1305's text and tag.
fn seal(data: &[u8]) -> Option<Vec<u8>> {
    use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce, aead::Aead};
    let key = seal_key()?;
    let mut nonce = [0u8; NONCE];
    getrandom::fill(&mut nonce).ok()?;
    let sealed = ChaCha20Poly1305::new(Key::from_slice(&key))
        .encrypt(Nonce::from_slice(&nonce), data)
        .ok()?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Some(out)
}

/// What [`seal`] sealed on this device.
fn open_sealed(sealed: &[u8]) -> Option<Vec<u8>> {
    use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce, aead::Aead};
    let key = seal_key()?;
    let (nonce, text) = sealed.split_at_checked(NONCE)?;
    ChaCha20Poly1305::new(Key::from_slice(&key))
        .decrypt(Nonce::from_slice(nonce), text)
        .ok()
}

/// iOS runs no program the app did not ship with (the preparer's Java launchers).
fn library_dir() -> Option<PathBuf> {
    None
}

/// A file the app carries, in its bundle's `assets` (the APK's `assets/`).
fn asset(name: &str) -> Option<Vec<u8>> {
    if name.contains("..") || name.starts_with('/') {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    std::fs::read(exe.parent()?.join("assets").join(name)).ok()
}

/// Background time while `reason` holds: iOS suspends an app soon after it leaves the screen.
fn keep_working(reason: &str, _line: &str, on: bool) {
    horizon_gamecontroller::background_task(reason, on);
}

/// `NSProcessInfo.thermalState` as Android's AThermalStatus: nominal none, fair light, serious
/// severe, critical critical.
fn thermal_status() -> Option<i32> {
    horizon_gamecontroller::power().map(|(_, state)| match state {
        0 => 0,
        1 => 1,
        2 => 3,
        _ => 4,
    })
}

/// iOS forecasts no throttling.
fn thermal_headroom() -> Option<f32> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn folders_become_url_paths() {
        let path = std::path::Path::new("/var/mobile/Containers/Data/A B/Documents/résumé");
        assert_eq!(
            super::url_path(path),
            "/var/mobile/Containers/Data/A%20B/Documents/r%C3%A9sum%C3%A9"
        );
    }
}
