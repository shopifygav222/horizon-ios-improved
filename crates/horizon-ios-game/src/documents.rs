//! Imports through Files' picker (UIDocumentPickerViewController), bounded background copies and
//! explicit collision choices, as Horizon Android's Storage Access Framework imports; exports
//! through Files' save sheet (PNGs to Photos, as Android's go to Pictures). The Documents folder
//! is also shared with the Files app (UIFileSharingEnabled) for copying by hand.
use horizon_client::{
    Result,
    content_files::{self, Kind},
    hud::rgba,
    ui_theme::{Face, Ui, color},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct Request {
    kind: Kind,
    appearance: Option<crate::skins::library::Kind>,
    target_dir: Option<PathBuf>,
    home: PathBuf,
    cache: PathBuf,
}
struct Conflict {
    kind: Kind,
    staged: PathBuf,
    target: PathBuf,
}
#[derive(Clone)]
pub struct Notice {
    pub text: String,
    pub changed: bool,
    pub kind: Option<Kind>,
}
struct State {
    request: Option<Request>,
    working: bool,
    conflict: Option<Conflict>,
    /// Files picked together that wait for a conflict's choice: imported after it.
    queue: Option<(Request, Vec<PathBuf>)>,
    notice: Option<Notice>,
    hits: Vec<([f32; 4], Choice)>,
    toast: Option<(String, std::time::Instant)>,
    cursor: (f32, f32),
}
#[derive(Clone, Copy)]
enum Choice {
    KeepBoth,
    Replace,
    Cancel,
}
static STATE: Mutex<State> = Mutex::new(State {
    request: None,
    working: false,
    conflict: None,
    queue: None,
    notice: None,
    hits: Vec::new(),
    toast: None,
    cursor: (0.0, 0.0),
});
static CANCEL: AtomicBool = AtomicBool::new(false);

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}
pub fn busy() -> bool {
    let s = state();
    s.request.is_some() || s.working || s.conflict.is_some()
}
fn conflict() -> bool {
    state().conflict.is_some()
}
fn notice(text: impl Into<String>, changed: bool) {
    let text = text.into();
    let mut s = state();
    s.toast = Some((text.clone(), std::time::Instant::now()));
    s.notice = Some(Notice {
        text,
        changed,
        kind: None,
    });
}
fn imported(text: impl Into<String>, kind: Kind) {
    let text = text.into();
    let mut s = state();
    s.toast = Some((text.clone(), std::time::Instant::now()));
    s.notice = Some(Notice {
        text,
        changed: true,
        kind: Some(kind),
    });
}
pub fn poll() -> Option<Notice> {
    state().notice.take()
}

/// The private cache folder an import stages in.
fn cache(name: &str) -> std::result::Result<PathBuf, String> {
    Ok(crate::platform::cache_dir()
        .ok_or("Could not access the import cache")?
        .join(name))
}

/// Files' picker for `request`; what is picked is imported on a thread of its own.
fn open_picker(request: Request, prompt: &str) -> std::result::Result<(), String> {
    if busy() {
        return Err("Finish or cancel the current import first".into());
    }
    CANCEL.store(false, Ordering::Relaxed);
    crate::platform::hide();
    state().request = Some(request);
    if !horizon_gamecontroller::pick(horizon_gamecontroller::Pick::Files, Box::new(picked)) {
        state().request = None;
        notice("Files' picker is unavailable", false);
        return Err("Files' picker is unavailable".into());
    }
    notice(prompt, false);
    Ok(())
}

pub fn begin(kind: Kind, home: PathBuf) -> std::result::Result<(), String> {
    let request = Request {
        kind,
        appearance: None,
        target_dir: None,
        home,
        cache: cache("content-imports")?,
    };
    open_picker(request, &format!("Choose a {} file", kind.label()))
}
pub fn begin_appearance(
    home: PathBuf,
    kind: crate::skins::library::Kind,
) -> std::result::Result<(), String> {
    let request = Request {
        kind: Kind::Packs,
        appearance: Some(kind),
        target_dir: None,
        home,
        cache: cache("appearance-imports")?,
    };
    open_picker(request, "Choose a skin or cape PNG")
}

/// Files' picker, to import files straight into `target_dir` (the file manager).
pub fn begin_into_dir(target_dir: PathBuf) -> std::result::Result<(), String> {
    let request = Request {
        kind: Kind::Packs,
        appearance: None,
        target_dir: Some(target_dir),
        home: PathBuf::new(),
        cache: cache("file-manager-imports")?,
    };
    open_picker(request, "Choose a file to import into this folder")
}

/// Exports a local file: a PNG to Photos, anything else through Files' save sheet, to a folder
/// the player chooses.
pub fn export_file(
    _home: &Path,
    _profile: u32,
    file_path: &Path,
) -> std::result::Result<String, String> {
    let meta = std::fs::symlink_metadata(file_path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("Only regular files can be exported.".into());
    }
    let file_name = file_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid filename")?;
    if file_name.to_ascii_lowercase().ends_with(".png") {
        crate::gallery::save(file_path)?;
        return Ok(format!("Saving '{file_name}' to Photos..."));
    }
    if horizon_gamecontroller::export(&[file_path.to_owned()]) {
        Ok(format!("Choose where Files saves '{file_name}'."))
    } else {
        Err(format!(
            "Files' save sheet is unavailable. '{file_name}' is in the Starling folder in the \
             Files app."
        ))
    }
}

/// What Files' picker handed over (on the picker's thread): each file imported in turn. The
/// copies iOS made for this app are removed once read.
fn picked(paths: Vec<PathBuf>) {
    // Taken first: the state's lock is free again before anything below takes it.
    let request = state().request.take();
    let Some(request) = request else {
        notice("Import interrupted. Choose the file again.", false);
        remove_copies(&paths);
        return;
    };
    if paths.is_empty() {
        notice("Import cancelled", false);
        return;
    }
    state().working = true;
    import_all(request, paths);
}

/// Imports `paths` for `request` in order until one meets a file of the same name: the rest wait
/// for the player's choice.
fn import_all(request: Request, mut paths: Vec<PathBuf>) {
    paths.reverse();
    while let Some(path) = paths.pop() {
        if CANCEL.load(Ordering::Relaxed) {
            remove_copies(&paths);
            remove_copies(std::slice::from_ref(&path));
            break;
        }
        let result = import_one(&request, &path);
        remove_copies(std::slice::from_ref(&path));
        if let Err(error) = result {
            notice(format!("Import failed: {error}"), false);
        }
        if conflict() {
            paths.reverse();
            state().queue = (!paths.is_empty()).then(|| (request.clone(), paths));
            break;
        }
    }
    state().working = false;
}

fn import_one(request: &Request, path: &Path) -> Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("The picked file has no name")?;
    let input = std::fs::File::open(path)?;
    stage(request.clone(), input, name)
}

/// The copies iOS made in this app's temporary folder for a pick (its Inbox): read once.
fn remove_copies(paths: &[PathBuf]) {
    let temp = std::env::temp_dir();
    for path in paths {
        if path.starts_with(&temp) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// After a conflict's choice: the files picked with it.
fn continue_queue() {
    let queue = state().queue.take();
    let Some((request, paths)) = queue else {
        return;
    };
    if CANCEL.load(Ordering::Relaxed) {
        remove_copies(&paths);
        return;
    }
    state().working = true;
    let spawned = std::thread::Builder::new()
        .name("Content import".into())
        .spawn(move || import_all(request, paths));
    if let Err(error) = spawned {
        state().working = false;
        notice(format!("Import failed: {error}"), false);
    }
}

fn stage(request: Request, mut input: std::fs::File, name: String) -> Result<()> {
    if let Some(target_dir) = request.target_dir {
        crate::document_store::validate_entry_name(&name)?;
        std::fs::create_dir_all(&target_dir)?;
        if std::fs::symlink_metadata(&target_dir)?
            .file_type()
            .is_symlink()
        {
            return Err("Refusing a redirected destination folder".into());
        }
        std::fs::create_dir_all(&request.cache)?;
        if std::fs::symlink_metadata(&request.cache)?
            .file_type()
            .is_symlink()
        {
            return Err("Refusing a redirected import cache".into());
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let staged = request.cache.join(format!(".import-fm-{stamp}.part"));
        let res = (|| -> Result<()> {
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)?;
            content_files::copy_bounded(&mut input, &mut output, |bytes| {
                notice(format!("Importing {name}: {} KiB", bytes / 1024), false);
                !CANCEL.load(Ordering::Relaxed)
            })?;
            output.sync_all()?;
            drop(output);
            if CANCEL.load(Ordering::Relaxed) {
                return Err("Import cancelled".into());
            }
            let dest = crate::document_store::unique_destination_path(&target_dir, &name)?;
            let final_name = dest
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if std::fs::rename(&staged, &dest).is_err() {
                std::fs::copy(&staged, &dest)?;
                let _ = std::fs::remove_file(&staged);
            }
            notice(
                format!("Imported '{final_name}' into current folder."),
                true,
            );
            Ok(())
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&staged);
        }
        return res;
    }
    if let Some(kind) = request.appearance {
        use std::io::Read;
        if !name.to_ascii_lowercase().ends_with(".png") {
            return Err("Choose a PNG skin or cape".into());
        }
        let mut bytes = Vec::new();
        input
            .take(crate::skins::MAX_IMPORT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if CANCEL.load(Ordering::Relaxed) {
            return Err("Import cancelled".into());
        }
        let mut library = crate::skins::library::Library::load(&request.home)?;
        library.import(&request.home, &name, kind, &bytes)?;
        notice(
            "PNG added to the local gallery. Select it to apply locally.",
            false,
        );
        return Ok(());
    }
    if !content_files::plain_name(&name) || !request.kind.accepts(&name) {
        return Err("Selected file has the wrong extension for this folder".into());
    }
    let dir = request.home.join(request.kind.folder());
    std::fs::create_dir_all(&dir)?;
    if std::fs::symlink_metadata(&dir)?.file_type().is_symlink() {
        return Err("Refusing a redirected content directory".into());
    }
    std::fs::create_dir_all(&request.cache)?;
    if std::fs::symlink_metadata(&request.cache)?
        .file_type()
        .is_symlink()
    {
        return Err("Refusing a redirected import cache".into());
    }
    for entry in std::fs::read_dir(&request.cache)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".import-") && name.ends_with(".part") && entry.file_type()?.is_file() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let staged = request.cache.join(format!(".import-{stamp}.part"));
    let result = (|| -> Result<()> {
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        content_files::copy_bounded(&mut input, &mut output, |bytes| {
            notice(
                format!("Importing {}: {} KiB", request.kind.label(), bytes / 1024),
                false,
            );
            !CANCEL.load(Ordering::Relaxed)
        })?;
        output.sync_all()?;
        drop(output);
        notice("Checking archive...", false);
        let native_shader = content_files::validate(&staged, request.kind, &name, || {
            CANCEL.load(Ordering::Relaxed)
        })?;
        let name = if request.kind == Kind::Mods {
            format!("{}.hmod", horizon_mods::Package::read(&staged)?.manifest.id)
        } else if native_shader || name.to_ascii_lowercase().ends_with(".hshader") {
            let pack = horizon_shader_api::Pack::read(&staged)?;
            horizon_shader_api::check(&pack).map_err(|e| format!("Invalid shader pack: {e:?}"))?;
            let mut path = PathBuf::from(name);
            path.set_extension("hshader");
            path.to_string_lossy().into_owned()
        } else {
            name
        };
        if !content_files::plain_name(&name) {
            return Err("Package has an invalid ID or filename".into());
        }
        if CANCEL.load(Ordering::Relaxed) {
            return Err("Import cancelled".into());
        }
        let target = dir.join(name);
        if target.exists() {
            state().conflict = Some(Conflict {
                kind: request.kind,
                staged: staged.clone(),
                target,
            });
            notice(
                "File already exists: choose Keep both, Replace or Cancel",
                false,
            );
        } else {
            std::fs::rename(&staged, &target)?;
            imported(
                format!(
                    "Imported {} into {}. Enable it in game settings.",
                    target.file_name().unwrap_or_default().to_string_lossy(),
                    request.kind.label()
                ),
                request.kind,
            );
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result
}
fn resolve(choice: Choice) {
    let c = state().conflict.take();
    let Some(mut c) = c else {
        return;
    };
    if matches!(choice, Choice::Cancel) {
        let _ = std::fs::remove_file(c.staged);
        notice("Import cancelled", false);
        continue_queue();
        return;
    }
    let result = (|| -> Result<()> {
        if matches!(choice, Choice::Replace)
            && std::fs::symlink_metadata(&c.target)
                .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err("Cannot replace a folder or symbolic link".into());
        }
        if matches!(choice, Choice::KeepBoth) {
            if c.kind == Kind::Mods {
                return Err(
                    "A mod ID cannot be installed twice; choose Replace to update it".into(),
                );
            }
            let mut stem = c
                .target
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            while stem.len() > 150 {
                stem.pop();
            }
            let ext = c
                .target
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let dir = c
                .target
                .parent()
                .ok_or("Invalid content folder")?
                .to_owned();
            for i in 1..=10000 {
                let candidate = dir.join(format!("{stem} ({i}).{ext}"));
                if !candidate.exists() {
                    c.target = candidate;
                    break;
                }
                if i == 10000 {
                    return Err("Too many filename conflicts".into());
                }
            }
        }
        std::fs::rename(&c.staged, &c.target)?;
        Ok(())
    })();
    match result {
        Ok(()) => imported(
            "Import completed. Enable it in Installed or game settings.",
            c.kind,
        ),
        Err(e) => {
            let _ = std::fs::remove_file(c.staged);
            notice(format!("Import failed: {e}"), false);
        }
    }
    continue_queue();
}
fn cancel() {
    CANCEL.store(true, Ordering::Relaxed);
    state().request = None;
    if conflict() {
        resolve(Choice::Cancel);
    } else {
        notice("Import cancelled", false);
    }
}
fn tap(x: f32, y: f32) -> bool {
    if !busy() {
        return false;
    }
    let choice = state()
        .hits
        .iter()
        .find(|(r, _)| x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3])
        .map(|(_, c)| *c);
    if let Some(choice) = choice {
        if matches!(choice, Choice::Cancel) {
            cancel();
        } else {
            resolve(choice);
        }
    }
    true
}
/// Launcher overlays must use its bundled font, including before game fonts exist.
pub fn draw_ui(ui: &mut Ui<'_>) {
    let mut s = state();
    s.hits.clear();
    if s.conflict.is_none() && !s.working {
        // The launcher already shows the result in its status line. Keep only
        // actionable overlays (progress/cancel and a name-conflict choice).
        return;
    }
    let (name, choices) = if let Some(c) = &s.conflict {
        (
            c.target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            if c.kind == Kind::Mods {
                vec![("Replace", Choice::Replace), ("Cancel", Choice::Cancel)]
            } else {
                vec![
                    ("Keep both", Choice::KeepBoth),
                    ("Replace", Choice::Replace),
                    ("Cancel", Choice::Cancel),
                ]
            },
        )
    } else {
        (
            s.toast.as_ref().map(|v| v.0.clone()).unwrap_or_default(),
            vec![("Cancel", Choice::Cancel)],
        )
    };
    let w = 440.0f32.min(ui.w - 24.0);
    let vertical = w < 320.0;
    let height = if vertical {
        92.0 + choices.len() as f32 * 44.0
    } else {
        144.0
    };
    let x = (ui.w - w) / 2.0;
    let y = ((ui.h - height) / 2.0).max(0.0);
    ui.rounded(0.0, 0.0, ui.w, ui.h, 0.0, rgba(0, 0, 0, 180));
    ui.rounded(x, y, w, height, 2.0, color::BG);
    ui.text(
        Face::Medium,
        20.0,
        x + 12.0,
        y + 12.0,
        color::TEXT,
        if s.conflict.is_some() {
            "File exists"
        } else {
            "Importing file"
        },
    );
    let name = ui.fit_text(Face::Regular, 16.0, &name, w - 24.0);
    ui.text(Face::Regular, 16.0, x + 12.0, y + 46.0, color::MUTED, &name);
    let width = (w - 24.0) / choices.len() as f32;
    for (i, (label, choice)) in choices.into_iter().enumerate() {
        let rect = if vertical {
            [x + 12.0, y + 86.0 + i as f32 * 44.0, w - 24.0, 38.0]
        } else {
            [x + 12.0 + i as f32 * width, y + 92.0, width - 4.0, 38.0]
        };
        ui.rounded(rect[0], rect[1], rect[2], rect[3], 2.0, color::RAISED);
        ui.text(
            Face::Medium,
            16.0,
            rect[0] + 8.0,
            rect[1] + 7.0,
            color::TEXT,
            label,
        );
        s.hits.push((
            [
                rect[0] * ui.density,
                rect[1] * ui.density,
                rect[2] * ui.density,
                rect[3] * ui.density,
            ],
            choice,
        ));
    }
}
pub fn handle_event(event: &winit::event::WindowEvent) -> bool {
    use winit::{
        event::{ElementState, MouseButton, TouchPhase, WindowEvent},
        keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    };
    if !busy() {
        return false;
    }
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            state().cursor = (position.x as f32, position.y as f32);
            true
        }
        WindowEvent::MouseInput {
            button: MouseButton::Left,
            state: pressed,
            ..
        } => {
            if *pressed == ElementState::Released {
                let (x, y) = state().cursor;
                tap(x, y);
            }
            true
        }
        WindowEvent::Touch(t) => {
            if t.phase == TouchPhase::Ended {
                tap(t.location.x as f32, t.location.y as f32);
            }
            true
        }
        WindowEvent::KeyboardInput { event, .. } => {
            if event.state == ElementState::Pressed {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    || event.logical_key == Key::Named(NamedKey::BrowserBack)
                {
                    cancel();
                } else if event.physical_key == PhysicalKey::Code(KeyCode::Enter) && conflict() {
                    let native = state()
                        .conflict
                        .as_ref()
                        .is_some_and(|c| c.kind == Kind::Mods);
                    resolve(if native {
                        Choice::Cancel
                    } else {
                        Choice::KeepBoth
                    });
                }
            }
            true
        }
        _ => false,
    }
}
