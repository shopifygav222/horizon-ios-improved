//! Android: the catalog, where a player finds and installs mods, resource packs and shader packs
//! without leaving the game. It opens from the Mods, Resource Packs and Shader Packs pages, on their
//! shelf. Searching, checking and installing are the desktop launcher's mod browser (the
//! `starling-catalog` crate); this is its screen. The list's pages and the finger on it are
//! `starling_catalog::list`, where they have tests.
//!
//! Its Installed view lists everything in the shelf's folders, whatever put it there (the
//! catalog, the Files app, an import), with Remove; a native mod compiled into the app (iOS) is
//! listed too, and turns off instead.
use super::Game;
#[cfg(target_os = "android")]
use super::native_builds::{density, wrapped};
use horizon_client::{
    hud::HudBatch,
    ui_theme::{Face, Ui, color},
};
use starling_catalog::{
    Done, Shelf,
    browser::{Kind, Project, installed::Entry},
    list::{self, Ask, Feed, Scroller, Tail},
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use winit::event::{Touch, TouchPhase};

/// iOS has no native builds (it runs no code built elsewhere): their card's scale, here.
#[cfg(target_os = "ios")]
fn density(w: f32, h: f32) -> f32 {
    (w / 1040.0).min(h / 700.0).clamp(1.0, 4.0)
}

/// iOS: `text` in lines of at most `width`, as the native builds' card wraps it.
#[cfg(target_os = "ios")]
fn wrapped(ui: &Ui<'_>, face: Face, size: f32, width: f32, text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let next = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && ui.measure(face, size, &next) > width {
            lines.push(std::mem::replace(&mut line, word.to_owned()));
        } else {
            line = next;
        }
    }
    lines.push(line);
    lines
}

const SHELVES: [(Shelf, &str); 3] = [
    (Shelf::Mods, "Mods"),
    (Shelf::ResourcePacks, "Resource packs"),
    (Shelf::ShaderPacks, "Shader packs"),
];
// In UI units: a landscape phone's short side is 700 of them (`density`), about 0.1 mm each. The
// system's smallest finger target, 48 dp, is about 78 of them.
const MARGIN: f32 = 16.0;
/// What's above and below the list.
const HEAD_H: f32 = 158.0;
const FOOT_H: f32 = 78.0;
/// A row and the gap under it: three and a half rows show on a phone.
const ROW_H: f32 = 132.0;
const ROW_GAP: f32 = 12.0;
/// A row's button.
const ROW_BUTTON_W: f32 = 190.0;
const ROW_BUTTON_H: f32 = 80.0;
/// The line at the end of the list.
const END_H: f32 = 96.0;
const CLEAR_W: f32 = 96.0;
/// Text sizes: about the system's 16 and 14 sp.
const TITLE_TEXT: f32 = 26.0;
const BODY_TEXT: f32 = 22.0;
const BUTTON_TEXT: f32 = 22.0;
/// A line of `BODY_TEXT` (`Ui::text`).
const BODY_LINE: f32 = 32.0;
/// The next page is asked for when the end of the list is this close to what shows.
const NEAR_END: f32 = 3.0 * ROW_H;
/// What the screen leaves free while typing: the system's navigation bar comes back with the
/// keyboard, over the right edge in landscape, and takes the touches there. 48 dp, at most this
/// much of a phone's width.
const NAV_BAR_SHARE: f32 = 0.08;
/// The most characters a search takes.
const QUERY_CHARS: usize = 64;
/// The search runs once the typing has paused this long.
const TYPING_PAUSE: Duration = Duration::from_millis(400);
/// How often the open screen looks again at which files are in the game folder.
const FILE_CHECK: Duration = Duration::from_secs(2);
/// The panel that asks before a file is deleted.
const CONFIRM_W: f32 = 640.0;
const CONFIRM_H: f32 = 270.0;
/// A row's picture, a square at its left.
const PICTURE: f32 = 100.0;
/// A picture's pixels at most, a side: sharp at a phone's density, small in the atlas.
const PICTURE_PX: u32 = 128;
/// Pictures fetched at once, and kept at most (each up to 64 KB).
const PICTURES_AT_ONCE: usize = 4;
const PICTURES_KEPT: usize = 120;

#[derive(Clone, PartialEq)]
enum Act {
    Close,
    Shelf(Shelf),
    Search,
    Clear,
    Everything,
    Retry,
    Install(usize),
    /// Asks to remove the row's project.
    Remove(usize),
    /// Answers the question: delete it, or leave it.
    ConfirmRemove,
    CancelRemove,
    /// Shows what is installed on the shelf, or the catalog again.
    Installed,
    /// Asks to remove the Installed view's row.
    RemoveLocal(usize),
    /// Turns off the Installed view's row: a native mod compiled into the app.
    TurnOff(usize),
}

/// A row's picture: on its way, its pixels, or none to be had (the lettered tile stays).
#[derive(Clone)]
enum Picture {
    Loading,
    Ready(Arc<(u32, u32, Vec<u8>)>),
    Missing,
}

/// Where a row's picture comes from.
enum PictureSource {
    /// A project's icon, by its URL (the browser's image cache keeps it on disk).
    Url(String),
    /// A resource pack's own `pack.png`, in its zip or folder.
    Pack(std::path::PathBuf),
}

/// `rgba` (`w` x `h`) no larger than [`PICTURE_PX`] a side, by nearest pixel (pack icons are
/// pixel art).
fn fit_picture(w: u32, h: u32, rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
    let longest = w.max(h);
    if longest <= PICTURE_PX || w == 0 || h == 0 {
        return (w, h, rgba);
    }
    let (nw, nh) = (
        (w * PICTURE_PX / longest).max(1),
        (h * PICTURE_PX / longest).max(1),
    );
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        for x in 0..nw {
            let at = (((y * h / nh) * w + x * w / nw) * 4) as usize;
            out.extend_from_slice(&rgba[at..at + 4]);
        }
    }
    (nw, nh, out)
}

/// A resource pack's `pack.png`: in its folder, or at the top of its zip.
fn pack_png(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    if path.is_dir() {
        return std::fs::read(path.join("pack.png")).ok();
    }
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut entry = zip.by_name("pack.png").ok()?;
    if entry.size() > 4 << 20 {
        return None;
    }
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// The picture of `source`, read on a worker thread.
fn read_picture(home: &std::path::Path, source: &PictureSource) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h, rgba) = match source {
        PictureSource::Url(url) => {
            let path = starling_catalog::browser::images::fetch(home, url, PICTURE_PX).ok()?;
            horizon_client::hud::decode(&path).ok()?
        }
        PictureSource::Pack(path) => horizon_client::hud::decode_bytes(&pack_png(path)?).ok()?,
    };
    Some(fit_picture(w, h, rgba))
}

/// A row's picture where there is none: a tile with the title's first letter, its colour the
/// title's own.
fn letter_tile(ui: &mut Ui<'_>, rect: [f32; 4], title: &str) {
    const TILES: [u32; 6] = [0x5D9C3F, 0x3B7DD8, 0xC2622D, 0x8E5BC9, 0x2E9C8F, 0xB8456A];
    let pick = title
        .bytes()
        .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(usize::from(b)));
    let tile = color::hex(TILES[pick % TILES.len()], 255);
    ui.rounded(rect[0], rect[1], rect[2], rect[3], 18.0, tile);
    let letter: String = title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect());
    let width = ui.measure(Face::Semibold, 44.0, &letter);
    ui.text(
        Face::Semibold,
        44.0,
        rect[0] + (rect[2] - width) / 2.0,
        rect[1] + (rect[3] - 62.0) / 2.0,
        color::WHITE,
        &letter,
    );
}

/// Who made a project and how many use it: "by Someone · 1.2M downloads · Modrinth".
fn meta_line(project: &Project) -> String {
    let mut parts = Vec::new();
    if !project.author.trim().is_empty() {
        parts.push(format!("by {}", project.author.trim()));
    }
    match project.downloads {
        0 => {}
        n @ 1..1_000 => parts.push(format!("{n} downloads")),
        n @ 1_000..1_000_000 => parts.push(format!("{:.0}K downloads", n as f64 / 1_000.0)),
        n => parts.push(format!("{:.1}M downloads", n as f64 / 1_000_000.0)),
    }
    parts.push(project.source.label().to_owned());
    parts.join(" \u{b7} ")
}

/// Something in a shelf's folders, as the Installed view lists it.
#[derive(Clone, Debug, PartialEq)]
struct Local {
    /// Its path from the game folder (`resourcepacks/Faithful.zip`); empty for a native mod
    /// compiled into the app, which has no file here.
    file: String,
    title: String,
    about: String,
    /// The catalog's record of it, when the catalog installed it: removed through the catalog,
    /// whose list stays true.
    entry: Option<Entry>,
    /// The mod's id, for the Mods page and its list of what is on.
    mod_id: Option<String>,
    /// A native mod: on until turned off, and part of the next build on iOS.
    native: bool,
}

/// The native mods compiled into this app (iOS), by id and version.
#[cfg(target_os = "ios")]
fn built_in_mods() -> Vec<(String, String)> {
    super::native_builds_ios::built_mods()
}
#[cfg(not(target_os = "ios"))]
fn built_in_mods() -> Vec<(String, String)> {
    Vec::new()
}

/// A size in words.
fn size_words(bytes: u64) -> String {
    match bytes {
        0..1_000_000 => format!("{} KB", bytes.div_ceil(1000).max(1)),
        _ => format!("{:.1} MB", bytes as f64 / 1_000_000.0),
    }
}

/// Whether `name` is something the player put in a shelf's folder, not a file of the game's own
/// (hidden ones, a shader pack's saved options, the catalog's bookkeeping).
fn listed_name(name: &str) -> bool {
    !name.starts_with('.') && !name.ends_with(".txt") && !name.ends_with(".partial")
}

/// What the shelf's folders in the game folder `home` hold, by title.
fn local_items(home: &std::path::Path, shelf: Shelf) -> Vec<Local> {
    use horizon_client::mod_list::{DIR, NATIVE_DIR};
    let catalog = starling_catalog::browser::installed::Installed::load(home).entries;
    let folders: &[&str] = match shelf {
        Shelf::Mods => &[DIR, NATIVE_DIR],
        Shelf::ResourcePacks => &["resourcepacks"],
        Shelf::ShaderPacks => &["shaderpacks"],
    };
    let mut items = Vec::new();
    for folder in folders {
        let Ok(dir) = std::fs::read_dir(home.join(folder)) else {
            continue;
        };
        for child in dir.flatten() {
            let name = child.file_name().to_string_lossy().into_owned();
            let Ok(meta) = std::fs::symlink_metadata(child.path()) else {
                continue;
            };
            if !listed_name(&name) || meta.file_type().is_symlink() {
                continue;
            }
            let file = format!("{folder}/{name}");
            let entry = catalog.iter().find(|e| e.file == file).cloned();
            let path = child.path();
            let mut title = entry.as_ref().map_or_else(
                || {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                },
                |e| e.title.clone(),
            );
            let mut mod_id = None;
            let mut native = false;
            if *folder == NATIVE_DIR {
                let Some(bundle) = name
                    .ends_with(".hnmod")
                    .then(|| horizon_mods::index::native::Bundle::read(&path).ok())
                    .flatten()
                else {
                    continue;
                };
                if entry.is_none() {
                    title = format!("{} {}", bundle.manifest.id, bundle.manifest.version);
                }
                mod_id = Some(bundle.manifest.id);
                native = true;
            } else if *folder == DIR {
                match horizon_mods::Package::read(&path) {
                    Ok(package) => {
                        if entry.is_none() {
                            title =
                                format!("{} {}", package.manifest.name, package.manifest.version);
                        }
                        mod_id = Some(package.manifest.id);
                    }
                    Err(_) => mod_id = Some(title.clone()),
                }
            }
            let what = match (meta.is_dir(), native) {
                (_, true) => "Native mod".to_owned(),
                (true, false) => "Folder".to_owned(),
                (false, false) => size_words(meta.len()),
            };
            let from = if entry.is_some() {
                "from the catalog"
            } else {
                "added by you"
            };
            items.push(Local {
                file,
                title,
                about: format!("{what}, {from}."),
                entry,
                mod_id,
                native,
            });
        }
    }
    if shelf == Shelf::Mods {
        for (id, version) in built_in_mods() {
            let listed = items
                .iter()
                .any(|i| i.native && i.mod_id.as_deref() == Some(id.as_str()));
            if !listed {
                items.push(Local {
                    file: String::new(),
                    title: format!("{id} {version}"),
                    about: "Native mod compiled into this app. Turning it off stops it now; \
                            the next build without it takes it out."
                        .to_owned(),
                    entry: None,
                    mod_id: Some(id),
                    native: true,
                });
            }
        }
    }
    items.sort_by_key(|i| i.title.to_lowercase());
    items
}

/// Deletes `file` (a shelf's file or folder, from the game folder `home`): only a plain name
/// under one of the folders the Installed view reads, never through a link.
fn delete_local(home: &std::path::Path, file: &str) -> Result<(), String> {
    use horizon_client::mod_list::{DIR, NATIVE_DIR};
    let (folder, name) = file.split_once('/').ok_or("not a shelf's file")?;
    let known = [DIR, NATIVE_DIR, "resourcepacks", "shaderpacks"];
    if !known.contains(&folder) || name.is_empty() || name.contains(['/', '\\']) || name == ".." {
        return Err(format!("{file} is not a shelf's file"));
    }
    let path = home.join(folder).join(name);
    let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    }
    .map_err(|e| e.to_string())
}

#[derive(Default)]
struct Shared {
    /// The list: the searches' threads add to it.
    feed: Feed,
    /// The rows' pictures by their key (an icon's URL, `pack:<file>`), and how many are being
    /// fetched.
    pictures: HashMap<String, Picture>,
    fetching: usize,
    /// The title of what is being installed, what it says, and how far it is.
    installing: Option<String>,
    line: String,
    fraction: f32,
    finished: Option<(String, Result<Done, String>)>,
}

#[derive(Default)]
pub(super) struct Catalog {
    shown: bool,
    shelf: Shelf,
    query: String,
    /// The search box has the keyboard (text_mirror.rs), and when it last changed.
    typing: bool,
    typed_at: Option<Instant>,
    /// Resource packs: those made for other versions too.
    everything: bool,
    /// How far the list is scrolled, and the finger on the screen.
    list: Scroller,
    /// The projects installed from here whose file is there, by id, and when that was looked at.
    installed: HashSet<String>,
    checked_at: Option<Instant>,
    /// The project the player was asked about removing: nothing else takes a touch meanwhile.
    removing: Option<Project>,
    /// The Installed view instead of the catalog, its rows (the search narrows them), and the
    /// row the player was asked about removing.
    view_installed: bool,
    local: Vec<Local>,
    removing_local: Option<Local>,
    /// What the last install said, and whether it failed.
    notice: Option<(String, bool)>,
    shared: Arc<Mutex<Shared>>,
    /// The buttons as last drawn (UI units), with what each does.
    buttons: RefCell<Vec<([f32; 4], Act)>>,
}

/// `text` on one line that fits `width`, cut with dots if it must be.
fn fitted(ui: &Ui<'_>, face: Face, size: f32, width: f32, text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if ui.measure(face, size, &text) <= width {
        return text;
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cut = format!("{}...", chars.iter().collect::<String>().trim_end());
        if ui.measure(face, size, &cut) <= width {
            return cut;
        }
    }
    String::new()
}

/// `text` in at most `most` lines that fit `width`, the last cut with dots if it must be.
fn fitted_lines(
    ui: &Ui<'_>,
    face: Face,
    size: f32,
    width: f32,
    most: usize,
    text: &str,
) -> Vec<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut lines = wrapped(ui, face, size, width, &text);
    if lines.len() > most {
        let rest = lines.split_off(most - 1).join(" ");
        lines.push(rest);
    }
    lines
        .iter()
        .map(|line| fitted(ui, face, size, width, line))
        .collect()
}

/// `text` in the middle of `rect`.
fn label(ui: &mut Ui<'_>, rect: [f32; 4], ink: u32, text: &str) {
    let width = ui.measure(Face::Semibold, BUTTON_TEXT, text);
    ui.text_line(
        Face::Semibold,
        BUTTON_TEXT,
        rect[3],
        rect[0] + (rect[2] - width) / 2.0,
        rect[1],
        ink,
        text,
    );
}

/// A button's look: filled with the accent when `lit`.
fn plate(ui: &mut Ui<'_>, rect: [f32; 4], text: &str, lit: bool) {
    let (fill, ink) = if lit {
        (color::ACCENT, color::ON_ACCENT)
    } else {
        (color::RAISED, color::TEXT)
    };
    ui.rounded(rect[0], rect[1], rect[2], rect[3], 10.0, fill);
    label(ui, rect, ink, text);
}

/// A button above or below the list.
fn button(
    ui: &mut Ui<'_>,
    rect: [f32; 4],
    text: &str,
    lit: bool,
    act: Act,
    drawn: &mut Vec<([f32; 4], Act)>,
) {
    plate(ui, rect, text, lit);
    drawn.push((rect, act));
}

/// What to do next, after an install.
fn done_notice(done: Done, title: &str) -> String {
    match done {
        // iOS compiles it into the app on GitHub (native_builds_ios.rs), from the Mods page's top.
        Done::NativeMod if cfg!(target_os = "ios") => format!(
            "{title} is in. Close this and press Build on GitHub at the top of the Mods page to \
             play with it."
        ),
        Done::NativeMod => {
            format!("{title} is in. Close this and press Compile on its row to play with it.")
        }
        Done::ContentMod => format!("{title} is in. It loads the next time you start the game."),
        Done::ResourcePack => format!("{title} is in. Turn it on in Options > Resource Packs."),
        Done::ShaderPack => format!("{title} is in. Choose it on the Shader Packs page."),
    }
}

/// The list's height with `rows` rows and its last line.
fn content_h(rows: usize) -> f32 {
    rows as f32 * ROW_H + END_H
}

/// What the last line says while a page is on its way.
fn loading_line(shelf: Shelf, first: bool) -> String {
    let what = match shelf {
        Shelf::Mods => "mods",
        Shelf::ResourcePacks => "resource packs",
        Shelf::ShaderPacks => "shader packs",
    };
    if first {
        format!("Loading {what}...")
    } else {
        format!("Loading more {what}...")
    }
}

/// An installed row's button area: the word Installed over a Remove button.
fn installed_parts(rect: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let state = [rect[0], rect[1] - 14.0, rect[2], 30.0];
    let remove = [rect[0], rect[1] + 20.0, rect[2], rect[3] - 10.0];
    (state, remove)
}

/// The question over everything while the player is asked about deleting a file, and its buttons
/// (they are all that takes a touch then).
fn draw_confirm(ui: &mut Ui<'_>, title: &str, drawn: &mut Vec<([f32; 4], Act)>) {
    drawn.clear();
    let (w, h) = (ui.w, ui.h);
    ui.fill(0.0, 0.0, w, h, color::hex(0x00_00_00, 190));
    let width = CONFIRM_W.min(w - 2.0 * MARGIN);
    let panel = [(w - width) / 2.0, (h - CONFIRM_H) / 2.0, width, CONFIRM_H];
    ui.rounded(panel[0], panel[1], panel[2], panel[3], 14.0, color::BG);
    ui.rounded(panel[0], panel[1], panel[2], panel[3], 14.0, color::RAISED);
    let inner = width - 48.0;
    let head = fitted(
        ui,
        Face::Semibold,
        TITLE_TEXT,
        inner,
        &format!("Remove {title}?"),
    );
    ui.text(
        Face::Semibold,
        TITLE_TEXT,
        panel[0] + 24.0,
        panel[1] + 20.0,
        color::TEXT,
        &head,
    );
    let body = "This deletes its file from your game folder. You can install it again from here.";
    let mut y = panel[1] + 70.0;
    for line in fitted_lines(ui, Face::Regular, BODY_TEXT, inner, 3, body) {
        ui.text(
            Face::Regular,
            BODY_TEXT,
            panel[0] + 24.0,
            y,
            color::MUTED,
            &line,
        );
        y += BODY_LINE;
    }
    let by = panel[1] + CONFIRM_H - ROW_BUTTON_H - 20.0;
    let remove = [
        panel[0] + width - 2.0 * ROW_BUTTON_W - 36.0,
        by,
        ROW_BUTTON_W,
        ROW_BUTTON_H,
    ];
    let cancel = [
        panel[0] + width - ROW_BUTTON_W - 24.0,
        by,
        ROW_BUTTON_W,
        ROW_BUTTON_H,
    ];
    ui.rounded(
        remove[0],
        remove[1],
        remove[2],
        remove[3],
        10.0,
        color::DANGER,
    );
    label(ui, remove, color::TEXT, "Remove");
    drawn.push((remove, Act::ConfirmRemove));
    plate(ui, cancel, "Cancel", false);
    drawn.push((cancel, Act::CancelRemove));
}

/// A search that failed: the log has what it said, the screen one plain sentence.
fn search_failed(raw: &str) -> String {
    log::warn!("Horizon: the catalog's search failed: {raw}");
    list::friendly(raw, "The list didn't load. Try again.")
}

/// An install that failed, the same way.
fn install_failed(title: &str, raw: &str) -> String {
    log::warn!("Horizon: the catalog could not install {title}: {raw}");
    list::friendly(raw, &format!("Couldn't install {title}. Try again."))
}

/// A removal that failed, the same way.
fn remove_failed(title: &str, raw: &str) -> String {
    log::warn!("Horizon: the catalog could not remove {title}: {raw}");
    list::friendly(raw, &format!("Couldn't remove {title}. Try again."))
}

/// The rows that show and the list's last line, under what's above and below them.
/// The square a row's picture takes, at `y`.
fn picture_rect(y: f32) -> [f32; 4] {
    let row = ROW_H - ROW_GAP;
    [MARGIN + 12.0, y + (row - PICTURE) / 2.0, PICTURE, PICTURE]
}

/// Where a row's text starts.
const TEXT_X: f32 = MARGIN + 12.0 + PICTURE + 16.0;

/// A row's picture: its pixels, drawn after the rows (`pictures`), or the lettered tile now.
fn picture(
    ui: &mut Ui<'_>,
    shared: &Shared,
    key: Option<&str>,
    y: f32,
    title: &str,
    pictures: &mut Vec<(String, [f32; 4])>,
) {
    let rect = picture_rect(y);
    match key.and_then(|k| shared.pictures.get(k).map(|p| (k, p))) {
        Some((k, Picture::Ready(_))) => {
            ui.rounded(rect[0], rect[1], rect[2], rect[3], 18.0, color::RAISED);
            pictures.push((k.to_owned(), rect));
        }
        _ => letter_tile(ui, rect, title),
    }
}

fn draw_list(
    ui: &mut Ui<'_>,
    card: &Catalog,
    shared: &Shared,
    edge: f32,
    drawn: &mut Vec<([f32; 4], Act)>,
    pictures: &mut Vec<(String, [f32; 4])>,
) {
    let (top, bottom) = (HEAD_H, ui.h - FOOT_H);
    let row_w = edge - MARGIN;
    let text_w = row_w - ROW_BUTTON_W - 44.0 - (TEXT_X - MARGIN - 14.0);
    let rows = shared.feed.rows();
    let scrolled = card.list.offset;
    for i in list::rows_shown(scrolled, bottom - top, ROW_H, rows.len()) {
        let project = &rows[i];
        let y = top + i as f32 * ROW_H - scrolled;
        let row = ROW_H - ROW_GAP;
        ui.rounded(MARGIN, y, row_w, row, 12.0, color::RAISED);
        let title = if starling_catalog::is_native_mod(project) {
            format!("{} (native mod)", project.title)
        } else {
            project.title.clone()
        };
        picture(ui, shared, project.icon.as_deref(), y, &project.title, pictures);
        let title = fitted(ui, Face::Semibold, TITLE_TEXT, text_w, &title);
        ui.text(
            Face::Semibold,
            TITLE_TEXT,
            TEXT_X,
            y + 8.0,
            color::TEXT,
            &title,
        );
        let meta = fitted(ui, Face::Regular, BODY_TEXT, text_w, &meta_line(project));
        ui.text(Face::Regular, BODY_TEXT, TEXT_X, y + 46.0, color::MUTED, &meta);
        // What it is, or what to know before installing it.
        let (about, ink) = match starling_catalog::caution(project) {
            Some(caution) => (caution, color::ACCENT),
            None => (project.summary.clone(), color::MUTED),
        };
        let about = fitted(ui, Face::Regular, BODY_TEXT, text_w, &about);
        ui.text(Face::Regular, BODY_TEXT, TEXT_X, y + 80.0, ink, &about);
        let rect = [
            MARGIN + row_w - ROW_BUTTON_W - 16.0,
            y + (row - ROW_BUTTON_H) / 2.0,
            ROW_BUTTON_W,
            ROW_BUTTON_H,
        ];
        if shared.installing.as_deref() == Some(project.title.as_str()) {
            label(ui, rect, color::MUTED, "Installing");
        } else if card.installed.contains(&project.id) {
            let (state, remove) = installed_parts(rect);
            label(ui, state, color::MUTED, "Installed");
            ui.rounded(
                remove[0],
                remove[1],
                remove[2],
                remove[3],
                10.0,
                color::HOVER,
            );
            label(ui, remove, color::TEXT, "Remove");
            if let Some(part) = list::shown_part(remove, top, bottom) {
                drawn.push((part, Act::Remove(i)));
            }
        } else {
            plate(ui, rect, "Install", true);
            if let Some(part) = list::shown_part(rect, top, bottom) {
                drawn.push((part, Act::Install(i)));
            }
        }
    }

    let y = top + content_h(rows.len()) - END_H - scrolled;
    if y >= bottom {
        return;
    }
    let (line, ink) = match shared.feed.tail() {
        Tail::Loading => (loading_line(card.shelf, rows.is_empty()), color::MUTED),
        Tail::More => (loading_line(card.shelf, false), color::MUTED),
        Tail::End if rows.is_empty() => ("Nothing found.".to_owned(), color::MUTED),
        Tail::End => ("That's all of them.".to_owned(), color::MUTED),
        Tail::Failed(why) => (why.clone(), color::TEXT),
    };
    let lines = fitted_lines(ui, Face::Regular, BODY_TEXT, row_w - ROW_BUTTON_W - 44.0, 2, &line);
    let mut line_y = y + (END_H - lines.len() as f32 * BODY_LINE) / 2.0;
    for line in lines {
        ui.text(Face::Regular, BODY_TEXT, MARGIN + 14.0, line_y, ink, &line);
        line_y += BODY_LINE;
    }
    if matches!(shared.feed.tail(), Tail::Failed(_)) {
        let rect = [
            MARGIN + row_w - ROW_BUTTON_W - 16.0,
            y + (END_H - ROW_BUTTON_H) / 2.0,
            ROW_BUTTON_W,
            ROW_BUTTON_H,
        ];
        plate(ui, rect, "Try again", false);
        if let Some(part) = list::shown_part(rect, top, bottom) {
            drawn.push((part, Act::Retry));
        }
    }
}

/// The Installed view's rows that match the search.
fn shown_local(card: &Catalog) -> Vec<&Local> {
    let query = card.query.trim().to_lowercase();
    card.local
        .iter()
        .filter(|item| query.is_empty() || item.title.to_lowercase().contains(&query))
        .collect()
}

/// The Installed view: each thing in the shelf's folders, with Remove (or Turn off).
/// A local row's picture key: a resource pack's own `pack.png`.
fn local_picture_key(item: &Local) -> Option<String> {
    item.file
        .starts_with("resourcepacks/")
        .then(|| format!("pack:{}", item.file))
}

fn draw_local_list(
    ui: &mut Ui<'_>,
    card: &Catalog,
    shared: &Shared,
    edge: f32,
    drawn: &mut Vec<([f32; 4], Act)>,
    pictures: &mut Vec<(String, [f32; 4])>,
) {
    let (top, bottom) = (HEAD_H, ui.h - FOOT_H);
    let row_w = edge - MARGIN;
    let text_w = row_w - ROW_BUTTON_W - 44.0 - (TEXT_X - MARGIN - 14.0);
    let rows = shown_local(card);
    let scrolled = card.list.offset;
    for i in list::rows_shown(scrolled, bottom - top, ROW_H, rows.len()) {
        let item = rows[i];
        let y = top + i as f32 * ROW_H - scrolled;
        let row = ROW_H - ROW_GAP;
        ui.rounded(MARGIN, y, row_w, row, 12.0, color::RAISED);
        let key = local_picture_key(item);
        picture(ui, shared, key.as_deref(), y, &item.title, pictures);
        let title = fitted(ui, Face::Semibold, TITLE_TEXT, text_w, &item.title);
        ui.text(
            Face::Semibold,
            TITLE_TEXT,
            TEXT_X,
            y + 8.0,
            color::TEXT,
            &title,
        );
        let mut line_y = y + 46.0;
        for line in fitted_lines(ui, Face::Regular, BODY_TEXT, text_w, 2, &item.about) {
            ui.text(Face::Regular, BODY_TEXT, TEXT_X, line_y, color::MUTED, &line);
            line_y += BODY_LINE;
        }
        let rect = [
            MARGIN + row_w - ROW_BUTTON_W - 16.0,
            y + (row - ROW_BUTTON_H) / 2.0,
            ROW_BUTTON_W,
            ROW_BUTTON_H,
        ];
        let index = card.local.iter().position(|l| l == item).unwrap_or(i);
        let (word, act) = if item.file.is_empty() {
            ("Turn off", Act::TurnOff(index))
        } else {
            ("Remove", Act::RemoveLocal(index))
        };
        plate(ui, rect, word, false);
        if let Some(part) = list::shown_part(rect, top, bottom) {
            drawn.push((part, act));
        }
    }
    let y = top + content_h(rows.len()) - END_H - scrolled;
    if y >= bottom {
        return;
    }
    let line = if !rows.is_empty() {
        "That's everything installed here."
    } else if card.local.is_empty() {
        "Nothing installed here yet."
    } else {
        "Nothing installed matches."
    };
    let line_y = y + (END_H - BODY_LINE) / 2.0;
    ui.text(
        Face::Regular,
        BODY_TEXT,
        MARGIN + 14.0,
        line_y,
        color::MUTED,
        line,
    );
}

/// The title, Close, the three shelves and the search box.
fn draw_head(ui: &mut Ui<'_>, card: &Catalog, edge: f32, drawn: &mut Vec<([f32; 4], Act)>) {
    ui.text_line(
        Face::Semibold,
        TITLE_TEXT,
        62.0,
        MARGIN,
        8.0,
        color::TEXT,
        "Catalog",
    );
    button(
        ui,
        [edge - 130.0, 8.0, 130.0, 62.0],
        "Close",
        false,
        Act::Close,
        drawn,
    );
    let installed_w = ui.measure(Face::Semibold, BUTTON_TEXT, "Installed") + 44.0;
    button(
        ui,
        [edge - 138.0 - installed_w, 8.0, installed_w, 62.0],
        "Installed",
        card.view_installed,
        Act::Installed,
        drawn,
    );

    const SHELF_Y: f32 = 78.0;
    const SHELF_H: f32 = 72.0;
    let mut x = MARGIN;
    for (shelf, name) in SHELVES {
        let width = ui.measure(Face::Semibold, BUTTON_TEXT, name) + 44.0;
        button(
            ui,
            [x, SHELF_Y, width, SHELF_H],
            name,
            card.shelf == shelf,
            Act::Shelf(shelf),
            drawn,
        );
        x += width + 8.0;
    }
    let search = [x + 4.0, SHELF_Y, (edge - x - 4.0).max(120.0), SHELF_H];
    ui.rounded(
        search[0],
        search[1],
        search[2],
        search[3],
        10.0,
        color::RAISED,
    );
    if card.typing {
        ui.border(
            search[0],
            search[1],
            search[2],
            search[3],
            10.0,
            color::ACCENT,
        );
    }
    let hint = if card.view_installed {
        "Search what's installed..."
    } else {
        "Search..."
    };
    let (typed, ink) = if card.query.is_empty() && !card.typing {
        (hint, color::MUTED)
    } else {
        (card.query.as_str(), color::TEXT)
    };
    let typed = fitted(
        ui,
        Face::Regular,
        BODY_TEXT,
        search[2] - 32.0 - CLEAR_W,
        typed,
    );
    ui.text_line(
        Face::Regular,
        BODY_TEXT,
        search[3],
        search[0] + 16.0,
        search[1],
        ink,
        &typed,
    );
    if card.typing {
        let end = search[0] + 16.0 + ui.measure(Face::Regular, BODY_TEXT, &typed);
        ui.fill(end + 1.0, search[1] + 18.0, 2.0, 36.0, color::ACCENT);
    }
    if !card.query.is_empty() {
        let clear = [
            search[0] + search[2] - CLEAR_W,
            search[1],
            CLEAR_W,
            search[3],
        ];
        label(ui, clear, color::MUTED, "Clear");
        drawn.push((clear, Act::Clear));
    }
    drawn.push((search, Act::Search));
}

/// The versions toggle on its shelf, and what an install says.
fn draw_foot(
    ui: &mut Ui<'_>,
    card: &Catalog,
    shared: &Shared,
    edge: f32,
    drawn: &mut Vec<([f32; 4], Act)>,
) {
    let foot = ui.h - FOOT_H + 7.0;
    let mut right = edge;
    if card.shelf == Shelf::ResourcePacks && !card.view_installed {
        let name = if card.everything {
            "Other game versions: On"
        } else {
            "Other game versions: Off"
        };
        let width = ui.measure(Face::Semibold, BUTTON_TEXT, name) + 44.0;
        right -= width;
        button(
            ui,
            [right, foot, width, 64.0],
            name,
            card.everything,
            Act::Everything,
            drawn,
        );
        right -= 12.0;
    }
    let status_w = (right - MARGIN).max(0.0);
    if shared.installing.is_some() {
        let line = fitted(ui, Face::Regular, BODY_TEXT, status_w, &shared.line);
        ui.text(Face::Regular, BODY_TEXT, MARGIN, foot, color::TEXT, &line);
        ui.rounded(MARGIN, foot + 38.0, status_w, 12.0, 6.0, color::RAISED);
        let done = (status_w * shared.fraction.clamp(0.0, 1.0)).max(12.0);
        ui.rounded(MARGIN, foot + 38.0, done, 12.0, 6.0, color::ACCENT);
    } else if let Some((notice, failed)) = &card.notice {
        let ink = if *failed { color::DANGER } else { color::TEXT };
        let mut y = foot;
        for line in fitted_lines(ui, Face::Regular, BODY_TEXT, status_w, 2, notice) {
            ui.text(Face::Regular, BODY_TEXT, MARGIN, y, ink, &line);
            y += BODY_LINE;
        }
    }
}

impl Game<'_> {
    /// A page's Catalog button: the screen, on that page's shelf.
    pub(super) fn open_catalog_on(&mut self, shelf: Shelf) {
        self.catalog.shelf = shelf;
        self.open_catalog();
    }

    /// The screen, on the shelf it was on.
    pub(super) fn open_catalog(&mut self) {
        starling_catalog::set_game_home(Self::game_dir());
        let card = &mut self.catalog;
        card.shown = true;
        card.notice = None;
        card.installed = starling_catalog::installed_projects(&Self::game_dir());
        // A picture that could not be had last time is asked for again.
        card.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pictures
            .retain(|_, p| !matches!(p, Picture::Missing));
        self.catalog_search();
    }

    /// Android's Back while the screen is open: it answers the question about a deletion with no,
    /// or else closes the screen and nothing under it. False when the screen isn't open.
    pub(super) fn catalog_back(&mut self) -> bool {
        let card = &mut self.catalog;
        if !card.shown {
            return false;
        }
        if card.removing.take().is_none() && card.removing_local.take().is_none() {
            card.shown = false;
            card.typing = false;
        }
        true
    }

    /// Which rows say Installed: those whose file is in the game folder right now, and what the
    /// Installed view lists.
    fn refresh_installed(&mut self) {
        let home = Self::game_dir();
        self.catalog.installed = starling_catalog::installed_projects(&home);
        if self.catalog.view_installed {
            self.catalog.local = local_items(&home, self.catalog.shelf);
        }
        self.catalog.checked_at = Some(Instant::now());
    }

    /// Removes what the player agreed to remove from the Installed view: through the catalog when
    /// it installed it, else the file itself. A native mod also turns off, so one compiled into
    /// the app stops too and leaves with its next build.
    fn remove_local(&mut self, item: Local) {
        let busy = self
            .catalog
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .installing
            .is_some();
        if busy {
            self.catalog.notice = Some(("Wait for the install to finish first.".to_owned(), true));
            return;
        }
        let home = Self::game_dir();
        let removed = match &item.entry {
            Some(entry) => {
                starling_catalog::browser::installed::remove(&home, &entry.project).map(|_| ())
            }
            None => delete_local(&home, &item.file),
        };
        if let Err(e) = removed {
            self.catalog.notice = Some((remove_failed(&item.title, &e), true));
            self.refresh_installed();
            return;
        }
        let mut said = format!("{} was removed.", item.title);
        match self.catalog.shelf {
            Shelf::Mods => {
                if item.native
                    && let Some(id) = &item.mod_id
                {
                    self.settings.mods.disabled.insert(id.clone());
                    self.settings.mods.save(&Self::game_dir());
                } else {
                    said.push_str(" It's gone the next time the game starts.");
                }
                self.refresh_mods_page(item.mod_id.as_deref());
            }
            Shelf::ResourcePacks => {
                let infos =
                    super::packs::pack_infos(&self.vanilla_root(), &self.server_packs.active);
                self.ui.options.update_pack_infos(infos);
            }
            Shelf::ShaderPacks => self.after_shader_pack_removed(&item.file),
        }
        self.catalog.notice = Some((said, false));
        self.refresh_installed();
    }

    /// The Installed view's Turn off: a native mod compiled into the app stops now, and the
    /// Mods page offers the build without it.
    fn turn_off_local(&mut self, item: &Local) {
        let Some(id) = &item.mod_id else {
            return;
        };
        self.settings.mods.disabled.insert(id.clone());
        self.settings.mods.save(&Self::game_dir());
        self.refresh_mods_page(None);
        self.catalog.notice = Some((
            format!(
                "{} is off. It stays in the app until a build without it.",
                item.title
            ),
            false,
        ));
        self.refresh_installed();
    }

    /// While the screen is open, looks again now and then: a file the player deleted by hand
    /// (from the phone's file manager) puts its row back to Install.
    fn catalog_check_files(&mut self) {
        let card = &self.catalog;
        if card.shown && card.checked_at.is_none_or(|at| at.elapsed() >= FILE_CHECK) {
            self.refresh_installed();
        }
    }

    /// The Mods page shows what the folder holds now, not what it held at the start. `gone` is a
    /// mod just removed, which is no longer on.
    fn refresh_mods_page(&mut self, gone: Option<&str>) {
        let on = horizon_client::mod_list::ModList::load(&Self::game_dir()).enabled;
        let enabled = &mut self.settings.mods.enabled;
        enabled.extend(on);
        if let Some(id) = gone {
            enabled.remove(id);
        }
        self.start_mods();
    }

    /// A row's Remove: asks first, since a deleted file can't come back.
    fn catalog_ask_remove(&mut self, index: usize) {
        let project = self
            .catalog
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .feed
            .rows()
            .get(index)
            .cloned();
        self.catalog.removing = project;
    }

    /// Deletes what the player agreed to delete, and tidies the pages that listed it.
    fn catalog_remove(&mut self, project: Project) {
        let busy = self
            .catalog
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .installing
            .is_some();
        if busy {
            self.catalog.notice = Some(("Wait for the install to finish first.".to_owned(), true));
            return;
        }
        let title = project.title.clone();
        self.catalog.notice = Some(
            match starling_catalog::remove(&Self::game_dir(), &project) {
                Ok(entry) => {
                    self.after_removal(&entry);
                    (format!("{title} was removed."), false)
                }
                Err(e) => (remove_failed(&title, &e), true),
            },
        );
        self.refresh_installed();
    }

    /// The page that listed a removed file lists what is left. A shader pack that was on goes off
    /// the way the Shader Packs page turns its pack off.
    fn after_removal(&mut self, entry: &Entry) {
        match entry.kind {
            Kind::Shader => self.after_shader_pack_removed(&entry.file),
            Kind::ResourcePack => {
                let infos =
                    super::packs::pack_infos(&self.vanilla_root(), &self.server_packs.active);
                self.ui.options.update_pack_infos(infos);
            }
            _ => {
                let id = starling_catalog::mod_id(entry);
                self.refresh_mods_page(id.as_deref());
            }
        }
    }

    /// `file` is `shaderpacks/<name>`: the list is read again, and if that pack was the one on, none is.
    fn after_shader_pack_removed(&mut self, file: &str) {
        let name = file.rsplit('/').next().unwrap_or(file);
        if self.settings.horizon.shader_pack == name {
            self.settings.horizon.shader_pack.clear();
            self.settings.horizon.shader_profile.clear();
            self.settings.save(&Self::options_path());
        }
        self.settings.shader_packs =
            horizon_client::shader_pack_list::scan(&Self::shader_packs_dir());
    }

    /// Whether the keyboard types in the search box (`text_context`).
    pub(super) fn catalog_typing(&self) -> bool {
        self.catalog.shown && self.catalog.typing
    }

    pub(super) fn catalog_query(&self) -> &str {
        &self.catalog.query
    }

    pub(super) fn catalog_type(&mut self, text: &str) {
        let card = &mut self.catalog;
        let room = QUERY_CHARS.saturating_sub(card.query.chars().count());
        card.query
            .extend(text.chars().filter(|c| !c.is_control()).take(room));
        card.typed_at = Some(Instant::now());
    }

    pub(super) fn catalog_backspace(&mut self) {
        let card = &mut self.catalog;
        card.query.pop();
        card.typed_at = Some(Instant::now());
    }

    /// The keyboard's action key: the search now, and the keyboard away.
    pub(super) fn catalog_submit(&mut self) {
        let card = &mut self.catalog;
        card.typing = false;
        card.typed_at = None;
        self.catalog_search();
    }

    /// A new list for the shelf and search as they are: back to the top, its first page asked for.
    /// The Installed view's is read from the folders, and the search narrows it here.
    fn catalog_search(&mut self) {
        self.refresh_installed();
        self.catalog.list.reset();
        if self.catalog.view_installed {
            return;
        }
        let ask = self
            .catalog
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .feed
            .restart();
        self.catalog_ask(ask);
    }

    /// Asks for a page on its own thread; its answer adds to the list when it comes.
    fn catalog_ask(&self, ask: Ask) {
        let card = &self.catalog;
        let shared = card.shared.clone();
        let worker = shared.clone();
        let (shelf, query, everything) = (card.shelf, card.query.clone(), card.everything);
        let started = std::thread::Builder::new()
            .name("catalog search".into())
            .spawn(move || {
                let found =
                    starling_catalog::search(shelf, &query, everything, ask.offset, list::PAGE)
                        .map_err(|e| search_failed(&e));
                worker
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .feed
                    .answer(ask, found);
            });
        if let Err(e) = started {
            shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .feed
                .answer(
                    ask,
                    Err(search_failed(&format!("The search didn't start: {e}"))),
                );
        }
    }

    /// How tall the part of the list that shows is, and the whole list.
    fn catalog_list_size(&self) -> (f32, f32) {
        let [w, h] = self.touch_size();
        let view = h as f32 / density(w as f32, h as f32) - HEAD_H - FOOT_H;
        let rows = if self.catalog.view_installed {
            shown_local(&self.catalog).len()
        } else {
            self.catalog
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .feed
                .rows()
                .len()
        };
        (view, content_h(rows))
    }

    /// Asks for the pictures of the rows that show and the next few, a few at a time, each on its
    /// own thread; once many are kept, the ones far from the rows that show go.
    fn catalog_pictures(&mut self) {
        let (view, _) = self.catalog_list_size();
        let home = Self::game_dir();
        let card = &self.catalog;
        let ahead = view + 3.0 * ROW_H;
        let wanted: Vec<(String, PictureSource)> = if card.view_installed {
            let rows = shown_local(card);
            list::rows_shown(card.list.offset, ahead, ROW_H, rows.len())
                .filter_map(|i| {
                    let item = rows[i];
                    let key = local_picture_key(item)?;
                    Some((key, PictureSource::Pack(home.join(&item.file))))
                })
                .collect()
        } else {
            let shared = card.shared.lock().unwrap_or_else(PoisonError::into_inner);
            let rows = shared.feed.rows();
            list::rows_shown(card.list.offset, ahead, ROW_H, rows.len())
                .filter_map(|i| {
                    let url = rows[i].icon.clone()?;
                    Some((url.clone(), PictureSource::Url(url)))
                })
                .collect()
        };
        let mut shared = card.shared.lock().unwrap_or_else(PoisonError::into_inner);
        if shared.pictures.len() > PICTURES_KEPT {
            let keep: HashSet<&str> = wanted.iter().map(|(k, _)| k.as_str()).collect();
            shared
                .pictures
                .retain(|k, p| keep.contains(k.as_str()) || matches!(p, Picture::Loading));
        }
        for (key, source) in wanted {
            if shared.fetching >= PICTURES_AT_ONCE {
                break;
            }
            if shared.pictures.contains_key(&key) {
                continue;
            }
            shared.pictures.insert(key.clone(), Picture::Loading);
            shared.fetching += 1;
            let worker = card.shared.clone();
            let home = home.clone();
            let named = key.clone();
            let started = std::thread::Builder::new()
                .name("catalog picture".into())
                .spawn(move || {
                    let got = read_picture(&home, &source);
                    let mut s = worker.lock().unwrap_or_else(PoisonError::into_inner);
                    s.fetching = s.fetching.saturating_sub(1);
                    let picture = got.map_or(Picture::Missing, |p| Picture::Ready(Arc::new(p)));
                    s.pictures.insert(named, picture);
                });
            if started.is_err() {
                shared.fetching = shared.fetching.saturating_sub(1);
                shared.pictures.insert(key, Picture::Missing);
            }
        }
    }

    /// While the screen is open: the list inside its ends, and its next page once the end is near.
    fn catalog_keep_up(&mut self) {
        let (view, content) = self.catalog_list_size();
        let card = &mut self.catalog;
        card.list.clamp(content - view);
        if card.view_installed || !list::near_end(card.list.offset, view, content, NEAR_END) {
            return;
        }
        let ask = card
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .feed
            .next();
        if let Some(ask) = ask {
            self.catalog_ask(ask);
        }
    }

    /// What came since the last frame: a pause in the typing, the keyboard put away, an install
    /// that ended (it goes on with the screen closed).
    pub(super) fn poll_catalog(&mut self) {
        if self.catalog.typing && self.keyboard_put_away("catalog") {
            self.catalog.typing = false;
        }
        if self
            .catalog
            .typed_at
            .is_some_and(|at| at.elapsed() >= TYPING_PAUSE)
        {
            self.catalog.typed_at = None;
            self.catalog.notice = None;
            self.catalog_search();
        }
        if self.catalog.shown {
            self.catalog_keep_up();
            self.catalog_pictures();
        }
        self.catalog_check_files();
        let finished = self
            .catalog
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finished
            .take();
        let Some((title, result)) = finished else {
            return;
        };
        self.catalog.notice = Some(match result {
            Ok(done) => {
                match done {
                    Done::NativeMod => self.list_new_native_mods(),
                    Done::ShaderPack => {
                        self.settings.shader_packs =
                            horizon_client::shader_pack_list::scan(&Self::shader_packs_dir());
                    }
                    // The Resource Packs page lists what was read when Options opened, and its
                    // folder watcher only sees what changes with the page open. A selection
                    // made on the page and not yet applied stays.
                    Done::ResourcePack => {
                        let infos = super::packs::pack_infos(
                            &self.vanilla_root(),
                            &self.server_packs.active,
                        );
                        self.ui.options.update_pack_infos(infos);
                    }
                    // The Mods page lists what was found when the game started.
                    Done::ContentMod => self.refresh_mods_page(None),
                }
                self.refresh_installed();
                (done_notice(done, &title), false)
            }
            Err(e) => (install_failed(&title, &e), true),
        });
    }

    fn catalog_install(&mut self, index: usize) {
        let card = &mut self.catalog;
        let shared = card.shared.clone();
        let worker = shared.clone();
        let project = {
            let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
            if s.installing.is_some() {
                return;
            }
            let Some(project) = s.feed.rows().get(index).cloned() else {
                return;
            };
            s.installing = Some(project.title.clone());
            s.line = project.title.clone();
            s.fraction = 0.0;
            project
        };
        card.notice = None;
        let everything = card.everything;
        let home = Self::game_dir();
        let title = project.title.clone();
        let started = std::thread::Builder::new()
            .name("catalog install".into())
            .spawn(move || {
                let progress = |line: &str, fraction: f32| {
                    let mut s = worker.lock().unwrap_or_else(PoisonError::into_inner);
                    line.clone_into(&mut s.line);
                    s.fraction = fraction;
                };
                // However it ends, the screen stops saying Installing: nothing else can install
                // while it does.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    starling_catalog::install(&project, everything, &home, &progress)
                }))
                .unwrap_or_else(|_| Err(format!("Installing {} stopped.", project.title)));
                let mut s = worker.lock().unwrap_or_else(PoisonError::into_inner);
                s.installing = None;
                s.finished = Some((project.title, result));
            });
        if let Err(e) = started {
            let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
            s.installing = None;
            let why = format!("Installing {title} didn't start: {e}");
            s.finished = Some((title, Err(why)));
        }
    }

    /// The screen, over everything else while it is open.
    pub(super) fn draw_catalog(&self, batch: &mut HudBatch) {
        let card = &self.catalog;
        if !card.shown {
            return;
        }
        let shared = card.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let mut drawn = Vec::new();
        let mut pictures = Vec::new();
        let (w, h, edge) = {
            let mut ui = Ui::new(&self.gui.atlas, batch, (-1.0, -1.0));
            let (w, h) = (ui.w, ui.h);
            let edge = w - MARGIN - if card.typing { w * NAV_BAR_SHARE } else { 0.0 };
            ui.fill(0.0, 0.0, w, h, color::BG);
            if card.view_installed {
                draw_local_list(&mut ui, card, &shared, edge, &mut drawn, &mut pictures);
            } else {
                draw_list(&mut ui, card, &shared, edge, &mut drawn, &mut pictures);
            }
            ui.finish();
            (w, h, edge)
        };
        // The rows' pictures, in the list's band: Horizon Android's toolkit draws images, in the
        // same units and the same atlas space as this screen's.
        {
            let atlas = &self.gui.atlas;
            let mut art =
                horizon_client::android_control_theme::Ui::new(atlas, batch, (-1.0, -1.0));
            art.set_clip(Some([0.0, HEAD_H, w, (h - HEAD_H - FOOT_H).max(0.0)]));
            for (key, [x, y, pw, ph]) in &pictures {
                let Some(Picture::Ready(pixels)) = shared.pictures.get(key) else {
                    continue;
                };
                let (iw, ih, rgba) = &**pixels;
                // Fitted in its square, its shape kept.
                let scale = (pw / *iw as f32).min(ph / *ih as f32);
                let (dw, dh) = (*iw as f32 * scale, *ih as f32 * scale);
                art.image(
                    key,
                    *iw,
                    *ih,
                    rgba,
                    x + (pw - dw) / 2.0,
                    y + (ph - dh) / 2.0,
                    dw,
                    dh,
                );
            }
            art.finish();
        }
        let mut ui = Ui::new(&self.gui.atlas, batch, (-1.0, -1.0));
        // The rows go under these two.
        ui.fill(0.0, 0.0, w, HEAD_H, color::BG);
        ui.fill(0.0, h - FOOT_H, w, FOOT_H, color::BG);
        draw_head(&mut ui, card, edge, &mut drawn);
        draw_foot(&mut ui, card, &shared, edge, &mut drawn);
        if let Some(project) = &card.removing {
            draw_confirm(&mut ui, &project.title, &mut drawn);
        } else if let Some(item) = &card.removing_local {
            draw_confirm(&mut ui, &item.title, &mut drawn);
        }
        ui.finish();
        *card.buttons.borrow_mut() = drawn;
    }

    /// While the screen is open it takes every touch. A finger that moves scrolls the list; one
    /// that lifts where it came down acts on the button there.
    pub(super) fn catalog_touch(&mut self, touch: &Touch) -> bool {
        if !self.catalog.shown {
            return false;
        }
        let [w, h] = self.touch_size();
        let d = density(w as f32, h as f32);
        let at = [touch.location.x as f32 / d, touch.location.y as f32 / d];
        let bottom = h as f32 / d - FOOT_H;
        let (view, content) = self.catalog_list_size();
        let most = content - view;
        let card = &mut self.catalog;
        let tapped = match touch.phase {
            TouchPhase::Started => {
                let on_list = at[1] >= HEAD_H && at[1] < bottom;
                card.list.press(touch.id, at, on_list);
                None
            }
            TouchPhase::Moved => {
                card.list.moved(touch.id, at, most);
                None
            }
            TouchPhase::Ended => card.list.lift(touch.id, at, most),
            TouchPhase::Cancelled => {
                card.list.cancel(touch.id);
                None
            }
        };
        let Some(at) = tapped else {
            return true;
        };
        let act = card
            .buttons
            .borrow()
            .iter()
            .find(|(r, _)| {
                at[0] >= r[0] && at[0] <= r[0] + r[2] && at[1] >= r[1] && at[1] <= r[1] + r[3]
            })
            .map(|(_, act)| act.clone());
        // A tap anywhere else leaves the search box; what was typed still runs.
        if !matches!(act, Some(Act::Search | Act::Clear)) {
            card.typing = false;
        }
        let Some(act) = act else {
            return true;
        };
        match act {
            Act::Close => card.shown = false,
            Act::Shelf(shelf) => {
                card.shelf = shelf;
                card.typed_at = None;
                card.notice = None;
                self.catalog_search();
            }
            Act::Search => card.typing = true,
            Act::Clear => {
                card.query.clear();
                card.typed_at = None;
                self.catalog_search();
            }
            Act::Everything => {
                card.everything = !card.everything;
                self.catalog_search();
            }
            Act::Retry => {
                let ask = card
                    .shared
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .feed
                    .retry();
                if let Some(ask) = ask {
                    self.catalog_ask(ask);
                }
            }
            Act::Install(index) => self.catalog_install(index),
            Act::Remove(index) => self.catalog_ask_remove(index),
            Act::ConfirmRemove => {
                if let Some(project) = card.removing.take() {
                    self.catalog_remove(project);
                } else if let Some(item) = card.removing_local.take() {
                    self.remove_local(item);
                }
            }
            Act::CancelRemove => {
                card.removing = None;
                card.removing_local = None;
            }
            Act::Installed => {
                card.view_installed = !card.view_installed;
                card.notice = None;
                self.catalog_search();
            }
            Act::RemoveLocal(index) => card.removing_local = card.local.get(index).cloned(),
            Act::TurnOff(index) => {
                if let Some(item) = card.local.get(index).cloned() {
                    self.turn_off_local(&item);
                }
            }
        }
        true
    }
}
