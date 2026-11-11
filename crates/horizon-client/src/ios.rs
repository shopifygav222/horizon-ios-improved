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
    FAILURE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
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
/// The launcher's performance recordings (Horizon Android's: Settings > Diagnostics), which it
/// registers at the start: the launcher is the app's program and the game a library under it,
/// so the game reaches them through these.
pub struct PerformanceHooks {
    /// A recording runs.
    pub active: fn() -> bool,
    /// A frame's times (frame, CPU, the GPU's stages, the four phases): its row, when kept.
    pub record: fn(f32, f32, &[f32], [f32; 4]) -> Option<serde_json::Value>,
    /// A kept row and what the game was doing then.
    pub submit: fn(serde_json::Value, serde_json::Value),
    /// A slow frame's spans.
    pub slow_frame: fn(f32, &[crate::spans::Span]),
    /// Once per loop iteration (the GPU profile setting starts and stops its recording).
    pub frame: fn(),
}
static PERFORMANCE: std::sync::OnceLock<PerformanceHooks> = std::sync::OnceLock::new();
pub fn set_performance_hooks(hooks: PerformanceHooks) {
    let _ = PERFORMANCE.set(hooks);
}
pub fn performance_active() -> bool {
    PERFORMANCE.get().is_some_and(|hooks| (hooks.active)())
}
pub fn performance_record(
    frame: f32,
    cpu: f32,
    gpu: &[f32],
    phases: [f32; 4],
) -> Option<serde_json::Value> {
    PERFORMANCE
        .get()
        .and_then(|hooks| (hooks.record)(frame, cpu, gpu, phases))
}
pub fn performance_submit(row: serde_json::Value, context: serde_json::Value) {
    if let Some(hooks) = PERFORMANCE.get() {
        (hooks.submit)(row, context);
    }
}
pub fn performance_slow_frame(cpu: f32, spans: &[crate::spans::Span]) {
    if let Some(hooks) = PERFORMANCE.get() {
        (hooks.slow_frame)(cpu, spans);
    }
}
pub fn performance_frame() {
    if let Some(hooks) = PERFORMANCE.get() {
        (hooks.frame)();
    }
}

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
            log::warn!(
                "Horizon: {} did not reach the import inbox: {e}",
                path.display()
            );
            let _ = std::fs::remove_file(&part);
        }
    }
}

/// iOS can't tell whether another app is installed: the setup offers Horizon's worlds once, and
/// a Skip there is final (game/setup.rs).
pub fn horizon_android_installed() -> bool {
    true
}

/// Horizon's own folder picked in Files (its iOS app's, "Horizon"), or any folder of it: its
/// worlds and resource and shader packs into the import inbox, as Starling's HorizonImport.java
/// brings Horizon Android's. False when no picker could open.
pub fn import_horizon_android() -> bool {
    horizon_gamecontroller::pick(
        horizon_gamecontroller::Pick::Folder,
        Box::new(|folders| {
            for folder in folders {
                let brought = bring_horizon(&folder, 0);
                log::info!(
                    "Horizon: {brought} worlds and packs from {} on their way",
                    folder.display()
                );
            }
        }),
    )
}

/// How deep [`bring_horizon`] looks for worlds and packs (Horizon's profiles hold them a few
/// folders down).
const HORIZON_DEPTH: usize = 6;
/// What a world keeps that is the running game's, not the world's.
const LOCK: &str = "session.lock";

/// Each world (a folder with `level.dat`) under `dir`, and each pack in a `resourcepacks` or
/// `shaderpacks` folder, into the inbox: how many went.
fn bring_horizon(dir: &Path, depth: usize) -> usize {
    if dir.join("level.dat").is_file() {
        return usize::from(zip_to_inbox(dir).is_ok());
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let packs = matches!(
        dir.file_name().and_then(|n| n.to_str()),
        Some("resourcepacks" | "shaderpacks")
    );
    let mut brought = 0;
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if packs && kind.is_file() {
            put_in_inbox(&path);
            brought += 1;
        } else if packs && kind.is_dir() && path.join("pack.mcmeta").is_file() {
            brought += usize::from(zip_to_inbox(&path).is_ok());
        } else if kind.is_dir() && depth < HORIZON_DEPTH {
            brought += bring_horizon(&path, depth + 1);
        }
    }
    brought
}

/// A folder (a world, a pack) as `<name>.zip` in the inbox (`.part` until complete), its files
/// under `<name>/`, a world's lock left out.
fn zip_to_inbox(folder: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| std::io::Error::other("a folder without a name"))?;
    let inbox = crate::import::inbox_dir();
    std::fs::create_dir_all(&inbox)?;
    let part = inbox.join(format!("{name}.zip.part"));
    let result = (|| -> std::io::Result<()> {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&part)?);
        // Region files are compressed already: the quickest level loses little.
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(1));
        let mut stack = vec![(folder.to_path_buf(), format!("{name}/"))];
        while let Some((dir, prefix)) = stack.pop() {
            for entry in std::fs::read_dir(&dir)?.flatten() {
                let kind = entry.file_type()?;
                let file = entry.file_name().to_string_lossy().into_owned();
                if kind.is_dir() {
                    stack.push((entry.path(), format!("{prefix}{file}/")));
                } else if kind.is_file() && file != LOCK {
                    zip.start_file(format!("{prefix}{file}"), options)
                        .map_err(std::io::Error::other)?;
                    std::io::copy(&mut std::fs::File::open(entry.path())?, &mut zip)?;
                }
            }
        }
        zip.finish().map_err(std::io::Error::other)?.flush()
    })();
    match result.and_then(|()| std::fs::rename(&part, inbox.join(format!("{name}.zip")))) {
        Ok(()) => {
            log::info!("Horizon: {name} is in the import inbox");
            Ok(())
        }
        Err(e) => {
            log::warn!("Horizon: {name} did not reach the import inbox: {e}");
            let _ = std::fs::remove_file(&part);
            Err(e)
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
