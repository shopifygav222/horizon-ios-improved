//! Starling's account screen (`Mode::Account`): signing in with a Microsoft account by its
//! device code, signing out, and fetching the game's official files as an owner of the game
//! (`crate::accounts`). Starling's own screen, in vanilla's look: no vanilla screen does this.
use super::{centered, wrapped};
use crate::accounts::{self, Files, State};
use crate::hud::{HudAtlas, HudBatch, rgba};

const SIGN_IN: u8 = 60;
const CANCEL: u8 = 61;
const OPEN_LINK: u8 = 62;
const COPY_CODE: u8 = 63;
const DOWNLOAD: u8 = 64;
const SIGN_OUT: u8 = 65;
const STOP_DOWNLOAD: u8 = 66;
const DONE: u8 = 69;

fn gray() -> u32 {
    rgba(170, 170, 170, 255)
}
fn red() -> u32 {
    rgba(255, 85, 85, 255)
}
fn yellow() -> u32 {
    rgba(255, 255, 85, 255)
}

/// The title's button for this screen: who plays, at a glance. On iOS the launcher's account
/// plays (the button opens the launcher's accounts).
pub(super) fn main_label() -> String {
    if cfg!(target_os = "ios") {
        return match crate::auth::current() {
            Some(session) => format!("Account: {}", session.name),
            None => "Accounts".into(),
        };
    }
    match accounts::state() {
        State::SignedIn(account) => format!("Account: {}", account.name),
        State::Resuming => "Account: signing in...".into(),
        _ => "Microsoft Account".into(),
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}

fn files_line(files: &Files) -> (String, u32) {
    match files {
        Files::Missing => ("Game files: not fetched".into(), gray()),
        Files::Checking => ("Game files: checking the account...".into(), gray()),
        Files::Downloading(p) if p.bytes_total == 0 => {
            ("Game files: listing what to fetch...".into(), gray())
        }
        Files::Downloading(p) => (
            format!(
                "Game files: {} of {} files, {} of {}",
                p.files,
                p.files_total,
                mib(p.bytes),
                mib(p.bytes_total)
            ),
            u32::MAX,
        ),
        Files::Ready => ("Game files: fetched and checked".into(), gray()),
        Files::Failed(e) => (format!("Game files: {e}"), red()),
    }
}

/// The screen's lines: text and colour.
fn lines(state: &State) -> Vec<(String, u32)> {
    match state {
        State::Offline => vec![(
            "Sign in with the Microsoft account that owns Minecraft: Java Edition to play on online-mode servers and to fetch the game's files from Mojang. The sign-in stays on this device.".into(),
            gray(),
        )],
        State::Resuming | State::Starting => vec![("Signing in...".into(), gray())],
        State::Code { code, link } => vec![
            (format!("Open {link} and enter this code:"), u32::MAX),
            (String::new(), 0),
            (code.clone(), yellow()),
            (String::new(), 0),
            (
                "Starling carries on by itself once you have signed in there.".into(),
                gray(),
            ),
        ],
        State::SignedIn(account) => {
            vec![
                (format!("Signed in as {}", account.name), u32::MAX),
                (String::new(), 0),
                files_line(&accounts::files()),
            ]
        }
        State::Failed(e) => vec![(e.clone(), red())],
    }
}

pub(super) fn draw(atlas: &HudAtlas, batch: &mut HudBatch) {
    let (w, h) = (batch.width.ceil(), batch.height.ceil());
    crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
    centered(atlas, batch, "Microsoft Account", 20.0, u32::MAX);
    let mut y = 50.0;
    for (text, color) in lines(&accounts::state()) {
        if text.is_empty() {
            y += 9.0;
            continue;
        }
        for line in wrapped(atlas, &text, (w - 50.0).min(320.0)) {
            centered(atlas, batch, &line, y, color);
            y += 9.0;
        }
    }
}

/// `(x, y, width, label, enabled, id)`: two rows at the bottom.
pub(super) fn buttons(w: f32, h: f32) -> Vec<(f32, f32, f32, String, bool, u8)> {
    let cx = (w / 2.0).floor();
    let (top, bottom) = (h - 52.0, h - 28.0);
    let wide = |y, label: &str, enabled, id| (cx - 100.0, y, 200.0, label.to_owned(), enabled, id);
    let left = |y, label: &str, enabled, id| (cx - 100.0, y, 98.0, label.to_owned(), enabled, id);
    let right = |y, label: &str, enabled, id| (cx + 2.0, y, 98.0, label.to_owned(), enabled, id);
    match accounts::state() {
        State::Offline | State::Failed(_) => vec![
            wide(top, "Sign in", true, SIGN_IN),
            wide(bottom, "Done", true, DONE),
        ],
        State::Resuming => vec![wide(bottom, "Done", true, DONE)],
        State::Starting => vec![
            wide(top, "Cancel", true, CANCEL),
            wide(bottom, "Done", true, DONE),
        ],
        State::Code { .. } => vec![
            left(top, "Open link", true, OPEN_LINK),
            right(top, "Copy code", true, COPY_CODE),
            left(bottom, "Cancel", true, CANCEL),
            right(bottom, "Done", true, DONE),
        ],
        State::SignedIn(_) => {
            let first = match accounts::files() {
                Files::Checking | Files::Downloading(_) => {
                    wide(top, "Stop download", true, STOP_DOWNLOAD)
                }
                Files::Ready => wide(top, "Check game files", true, DOWNLOAD),
                Files::Missing | Files::Failed(_) => wide(top, "Fetch game files", true, DOWNLOAD),
            };
            vec![
                first,
                left(bottom, "Sign out", true, SIGN_OUT),
                right(bottom, "Done", true, DONE),
            ]
        }
    }
}

/// A click on button `id`; true to go back to the title.
pub(super) fn click(id: u8) -> bool {
    match id {
        SIGN_IN => accounts::sign_in(),
        CANCEL => accounts::cancel(),
        OPEN_LINK | COPY_CODE => {
            if let State::Code { code, link } = accounts::state() {
                if id == OPEN_LINK {
                    // Without a browser to open (a desktop), the link is on the screen.
                    let _ = crate::platform::open_link(&link);
                } else {
                    let mut clipboard = crate::clipboard::SystemClipboard::default();
                    crate::clipboard::Clipboard::set(&mut clipboard, &code);
                }
            }
        }
        DOWNLOAD => accounts::download_files(),
        STOP_DOWNLOAD => accounts::cancel_download(),
        SIGN_OUT => {
            if let Err(e) = accounts::sign_out() {
                log::warn!("Horizon: signing out failed: {e}");
            }
        }
        DONE => return true,
        _ => {}
    }
    false
}
