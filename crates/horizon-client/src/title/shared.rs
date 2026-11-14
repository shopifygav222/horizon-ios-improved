//! Shared Worlds on the title screen: the worlds this player shares with friends, how many are
//! in each now, and starting or joining one; adding a world from a friend's invite; and sharing
//! one of this device's worlds (`crate::shared_worlds` does the work).
use super::{centered, wrapped};
use crate::{
    clipboard::{Clipboard, FieldId, SystemClipboard},
    hud::{HudAtlas, HudBatch, rgba},
    shared_worlds::{self, Entry, Status, Task},
};
use horizon_shared_worlds::s3::Bucket;
use std::time::Instant;

fn tr(key: &str, english: &str) -> String {
    crate::lang::get_or(&format!("horizon.shared.{key}"), english)
}
fn grey() -> u32 {
    rgba(170, 170, 170, 255)
}

/// Where friends read how to make the storage a shared world lives in.
pub const SETUP_URL: &str =
    "https://github.com/shopifygav222/horizon-pocket-edition-ios#shared-worlds";

const PLAY: u8 = 200;
const COPY: u8 = 201;
const ADD: u8 = 202;
const REMOVE: u8 = 203;
const BACK: u8 = 204;
const PREVIOUS: u8 = 205;
const NEXT: u8 = 206;
const SHARE: u8 = 207;
const WORLD_BACK: u8 = 209;
const ROW: u8 = 210;
const ADD_CONFIRM: u8 = 220;
const ADD_CANCEL: u8 = 221;
const PASTE: u8 = 222;
const PICK_ROW: u8 = 230;
const PICK_PREVIOUS: u8 = 236;
const PICK_NEXT: u8 = 237;
const PICK_CANCEL: u8 = 239;
const STORAGE_SHARE: u8 = 240;
const STORAGE_CANCEL: u8 = 241;
const STORAGE_HELP: u8 = 242;
const TASK_CANCEL: u8 = 250;
const TASK_OK: u8 = 251;
const REMOVE_CONFIRM: u8 = 260;
const REMOVE_CANCEL: u8 = 261;

const INVITE_FIELD: FieldId = ("shared.invite", 0);
const STORAGE_FIELDS: [(&str, &str, usize); 4] = [
    ("storage_address", "Storage address (https://...)", 256),
    ("bucket", "Bucket name", 63),
    ("access_key", "Access key ID", 128),
    ("secret_key", "Secret access key", 256),
];

#[derive(Clone, Debug, Default, PartialEq)]
enum Page {
    #[default]
    List,
    World(String),
    Remove(String),
    Add,
    Pick,
    Storage {
        folder: String,
        name: String,
    },
}

/// What a click asks of the title.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// Back to the main menu.
    Close,
    /// Start or join the world with this id.
    Play(String),
}

#[derive(Clone, Default)]
pub struct Screen {
    page: Page,
    offset: usize,
    invite: String,
    /// The storage fields, in [`STORAGE_FIELDS`] order.
    storage: [String; 4],
    focus: usize,
    /// One line under the page's buttons: what the last action did.
    notice: Option<String>,
    copied: Option<Instant>,
    /// This device's singleplayer worlds, (folder, name), for sharing one.
    local: Vec<(String, String)>,
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedWorldsScreen")
            .field("page", &self.page)
            .field("storage", &"<hidden>")
            .finish()
    }
}

fn rows(h: f32) -> usize {
    (((h - 120.0) / 24.0).floor() as usize).clamp(1, 6)
}

/// A row's text: the world's name and who is in it.
fn summary(entry: &Entry) -> String {
    format!("{}  -  {}", entry.name, status_line(&entry.status))
}

fn status_line(status: &Status) -> String {
    match status {
        Status::Unknown => tr("reading", "Reading..."),
        Status::Failed(_) => tr("unreachable", "Storage unreachable"),
        Status::Known { playing: 0, .. } => tr("nobody", "Nobody playing"),
        Status::Known {
            playing,
            host,
            open,
            ..
        } => {
            let host = host.clone().unwrap_or_default();
            if *open {
                tr("playing_host", "%d playing, hosted by %s")
                    .replacen("%d", &playing.to_string(), 1)
                    .replacen("%s", &host, 1)
            } else {
                tr("starting", "%s is opening it").replacen("%s", &host, 1)
            }
        }
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.0)
}

impl Screen {
    pub fn new() -> Self {
        shared_worlds::refresh(true);
        Self::default()
    }

    fn entry(&self, id: &str) -> Option<Entry> {
        shared_worlds::entries().into_iter().find(|e| e.id == id)
    }

    fn storage_page(&self) -> bool {
        matches!(self.page, Page::Storage { .. })
    }

    /// The focused text field, for typing, the caret, copy and paste.
    pub(super) fn field(&mut self) -> Option<(FieldId, &mut String, usize)> {
        if task_shown() {
            return None;
        }
        match self.page {
            Page::Add => Some((INVITE_FIELD, &mut self.invite, 4096)),
            Page::Storage { .. } => {
                let i = self.focus.min(3);
                Some((
                    ("shared.storage", i),
                    &mut self.storage[i],
                    STORAGE_FIELDS[i].2,
                ))
            }
            _ => None,
        }
    }

    pub(super) fn field_rect(&self, w: f32) -> Option<[f32; 4]> {
        let x = (w / 2.0).floor() - 150.0;
        match self.page {
            Page::Add => Some([x, 60.0, 300.0, 20.0]),
            Page::Storage { .. } => Some([x, 50.0 + 36.0 * self.focus.min(3) as f32, 300.0, 20.0]),
            _ => None,
        }
    }

    /// The secret access key is never shown.
    pub(super) fn password_masked(&self) -> bool {
        self.storage_page() && self.focus == 3
    }

    pub(super) fn focus_at(&mut self, (x, y): (f32, f32), w: f32) {
        if !self.storage_page() || (x - w / 2.0).abs() > 150.0 {
            return;
        }
        for i in 0..4 {
            let top = 50.0 + 36.0 * i as f32;
            if (top..top + 20.0).contains(&y) {
                self.focus = i;
            }
        }
    }

    pub(super) fn tab(&mut self, reverse: bool) -> bool {
        if !self.storage_page() {
            return false;
        }
        self.focus = (self.focus + if reverse { 3 } else { 1 }) % 4;
        true
    }

    pub(super) fn submit(&mut self, h: f32) -> bool {
        match self.page {
            Page::Add if !self.invite.trim().is_empty() => {
                self.click(ADD_CONFIRM, h, std::path::Path::new(""));
                true
            }
            Page::Storage { .. } if self.storage_ready() => {
                self.click(STORAGE_SHARE, h, std::path::Path::new(""));
                true
            }
            Page::Storage { .. } => self.tab(false),
            _ => false,
        }
    }

    /// Escape: back one page; false on the list, which leaves the screen.
    pub(super) fn back(&mut self) -> bool {
        if matches!(shared_worlds::task(), Task::Working { .. }) {
            shared_worlds::cancel();
            return true;
        }
        if task_shown() {
            shared_worlds::dismiss();
            return true;
        }
        if self.page == Page::List {
            return false;
        }
        self.leave_page();
        true
    }

    fn leave_page(&mut self) {
        self.page = match std::mem::take(&mut self.page) {
            Page::Remove(id) => Page::World(id),
            Page::Storage { .. } => Page::Pick,
            _ => Page::List,
        };
        self.notice = None;
        self.invite.clear();
        if !self.storage_page() {
            for field in &mut self.storage {
                field.clear();
            }
        }
    }

    fn storage_ready(&self) -> bool {
        self.storage.iter().all(|f| !f.trim().is_empty())
    }

    fn bucket(&self) -> Bucket {
        Bucket {
            endpoint: self.storage[0].trim().to_owned(),
            bucket: self.storage[1].trim().to_owned(),
            region: "auto".into(),
            access_key: self.storage[2].trim().to_owned(),
            secret_key: self.storage[3].trim().to_owned(),
        }
    }

    pub(super) fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut crate::ui::Caret,
        started: Instant,
    ) {
        let (w, h) = (batch.width.ceil(), batch.height.ceil());
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        centered(atlas, batch, &tr("title", "Shared Worlds"), 20.0, u32::MAX);
        let width = (w - 50.0).min(320.0);
        let lines = |batch: &mut HudBatch, text: &str, mut y: f32, color: u32| {
            for line in wrapped(atlas, text, width) {
                centered(atlas, batch, &line, y, color);
                y += 10.0;
            }
            y
        };
        match shared_worlds::task() {
            Task::Working { what, done, total } => {
                let y = lines(batch, &what, 60.0, u32::MAX);
                let detail = if total > 0 {
                    format!(
                        "{} / {} MB ({}%)",
                        megabytes(done),
                        megabytes(total),
                        done * 100 / total.max(1)
                    )
                } else {
                    ".".repeat((started.elapsed().as_millis() / 400 % 4) as usize + 1)
                };
                centered(atlas, batch, &detail, y + 6.0, grey());
                return;
            }
            Task::Failed(message) | Task::Done(message) => {
                lines(batch, &message, 60.0, u32::MAX);
                return;
            }
            Task::Idle => {}
        }
        match &self.page {
            Page::List => {
                let entries = shared_worlds::entries();
                let message = if entries.is_empty() {
                    tr(
                        "empty",
                        "No shared worlds yet. Share one of your worlds, or add one with the invite a friend sent you.",
                    )
                } else {
                    tr(
                        "intro",
                        "Whoever opens a world while nobody is playing hosts it.",
                    )
                };
                lines(batch, &message, 36.0, grey());
            }
            Page::World(id) => {
                let Some(entry) = self.entry(id) else {
                    return;
                };
                let mut y = lines(batch, &entry.name, 40.0, u32::MAX) + 4.0;
                y = lines(batch, &status_line(&entry.status), y, grey());
                match &entry.status {
                    Status::Failed(e) => {
                        lines(batch, e, y + 4.0, grey());
                    }
                    Status::Known { tunnel: true, .. } => {
                        lines(
                            batch,
                            &tr(
                                "tunnel_note",
                                "Friends join this host through a free public tunnel (bore.pub): it may be slower.",
                            ),
                            y + 4.0,
                            grey(),
                        );
                    }
                    _ => {}
                }
            }
            Page::Remove(id) => {
                let name = self.entry(id).map(|e| e.name).unwrap_or_default();
                lines(
                    batch,
                    &tr(
                        "remove_confirm",
                        "Take %s off this device? It stays in its storage for your friends, and your copy becomes a normal singleplayer world.",
                    )
                    .replacen("%s", &name, 1),
                    50.0,
                    u32::MAX,
                );
            }
            Page::Add => {
                centered(
                    atlas,
                    batch,
                    &tr("paste_invite", "Paste the world invite a friend sent you"),
                    46.0,
                    grey(),
                );
                let rect = [(w / 2.0).floor() - 150.0, 60.0, 300.0, 20.0];
                caret.draw(
                    batch,
                    atlas,
                    (INVITE_FIELD, &self.invite, 4096),
                    rect,
                    true,
                    started,
                );
            }
            Page::Pick => {
                let message = if self.local.is_empty() {
                    tr("no_local", "You have no singleplayer world to share.")
                } else {
                    tr(
                        "pick",
                        "Which world? It moves to this list, and stays on this device.",
                    )
                };
                lines(batch, &message, 36.0, grey());
            }
            Page::Storage { name, .. } => {
                for (i, (key, english, max)) in STORAGE_FIELDS.iter().enumerate() {
                    let y = 50.0 + 36.0 * i as f32;
                    centered(atlas, batch, &tr(key, english), y - 11.0, grey());
                    let field = (("shared.storage", i), self.storage[i].as_str(), *max);
                    let rect = [(w / 2.0).floor() - 150.0, y, 300.0, 20.0];
                    if i == 3 {
                        caret.draw_password(batch, atlas, field, rect, self.focus == i, started);
                    } else {
                        caret.draw(batch, atlas, field, rect, self.focus == i, started);
                    }
                }
                lines(
                    batch,
                    &tr("sharing", "Sharing %s").replacen("%s", name, 1),
                    50.0 + 36.0 * 4.0 - 6.0,
                    grey(),
                );
            }
        }
        if let Some(notice) = &self.notice {
            lines(batch, notice, h - 64.0, u32::MAX);
        }
    }

    pub(super) fn buttons(&self, w: f32, h: f32) -> Vec<(f32, f32, f32, String, bool, u8)> {
        let cx = (w / 2.0).floor();
        let mut out = Vec::new();
        let mut push = |x: f32, y: f32, bw: f32, label: String, enabled: bool, id: u8| {
            out.push((x, y, bw, label, enabled, id));
        };
        match shared_worlds::task() {
            Task::Working { .. } => {
                push(
                    cx - 100.0,
                    h - 27.0,
                    200.0,
                    tr("cancel", "Cancel"),
                    true,
                    TASK_CANCEL,
                );
                return out;
            }
            Task::Failed(_) | Task::Done(_) => {
                push(cx - 100.0, h - 27.0, 200.0, "OK".into(), true, TASK_OK);
                return out;
            }
            Task::Idle => {}
        }
        match &self.page {
            Page::List => {
                let entries = shared_worlds::entries();
                let n = rows(h);
                let offset = self.offset.min(entries.len().saturating_sub(1) / n * n);
                for (i, entry) in entries.iter().skip(offset).take(n).enumerate() {
                    push(
                        cx - 150.0,
                        56.0 + 24.0 * i as f32,
                        300.0,
                        summary(entry),
                        true,
                        ROW + i as u8,
                    );
                }
                if entries.len() > n {
                    push(cx - 176.0, 56.0, 20.0, "<".into(), offset > 0, PREVIOUS);
                    push(
                        cx + 156.0,
                        56.0,
                        20.0,
                        ">".into(),
                        offset + n < entries.len(),
                        NEXT,
                    );
                }
                push(
                    cx - 154.0,
                    h - 52.0,
                    150.0,
                    tr("share", "Share a World..."),
                    true,
                    SHARE,
                );
                push(
                    cx + 4.0,
                    h - 52.0,
                    150.0,
                    tr("add", "Add with Invite..."),
                    true,
                    ADD,
                );
                push(cx - 100.0, h - 27.0, 200.0, tr("back", "Back"), true, BACK);
            }
            Page::World(id) => {
                let status = self.entry(id).map(|e| e.status);
                let (label, enabled) = match &status {
                    Some(Status::Known { playing: 0, .. }) => (tr("start", "Start"), true),
                    Some(Status::Known {
                        open: true, host, ..
                    }) => (
                        tr("join", "Join %s").replacen("%s", host.as_deref().unwrap_or(""), 1),
                        true,
                    ),
                    Some(Status::Known { .. }) => (tr("wait", "Opening..."), false),
                    Some(Status::Failed(_)) => (tr("play", "Play"), false),
                    _ => (tr("play", "Play"), false),
                };
                push(cx - 100.0, 110.0, 200.0, label, enabled, PLAY);
                let copied = self.copied.is_some_and(|t| t.elapsed().as_secs() < 2);
                let copy = if copied {
                    tr("copied", "Copied")
                } else {
                    tr("copy_invite", "Copy World Invite")
                };
                push(cx - 100.0, 134.0, 200.0, copy, true, COPY);
                push(
                    cx - 100.0,
                    158.0,
                    200.0,
                    tr("remove", "Remove from This Device"),
                    true,
                    REMOVE,
                );
                push(
                    cx - 100.0,
                    h - 27.0,
                    200.0,
                    tr("back", "Back"),
                    true,
                    WORLD_BACK,
                );
            }
            Page::Remove(_) => {
                push(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("remove_yes", "Remove"),
                    true,
                    REMOVE_CONFIRM,
                );
                push(
                    cx + 4.0,
                    h - 27.0,
                    150.0,
                    tr("cancel", "Cancel"),
                    true,
                    REMOVE_CANCEL,
                );
            }
            Page::Add => {
                push(cx - 100.0, 86.0, 200.0, tr("paste", "Paste"), true, PASTE);
                let ready = !self.invite.trim().is_empty();
                push(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("add_world", "Add World"),
                    ready,
                    ADD_CONFIRM,
                );
                push(
                    cx + 4.0,
                    h - 27.0,
                    150.0,
                    tr("cancel", "Cancel"),
                    true,
                    ADD_CANCEL,
                );
            }
            Page::Pick => {
                let n = rows(h);
                let offset = self.offset.min(self.local.len().saturating_sub(1) / n * n);
                for (i, (_, name)) in self.local.iter().skip(offset).take(n).enumerate() {
                    push(
                        cx - 150.0,
                        56.0 + 24.0 * i as f32,
                        300.0,
                        name.clone(),
                        true,
                        PICK_ROW + i as u8,
                    );
                }
                if self.local.len() > n {
                    push(
                        cx - 176.0,
                        56.0,
                        20.0,
                        "<".into(),
                        offset > 0,
                        PICK_PREVIOUS,
                    );
                    push(
                        cx + 156.0,
                        56.0,
                        20.0,
                        ">".into(),
                        offset + n < self.local.len(),
                        PICK_NEXT,
                    );
                }
                push(
                    cx - 100.0,
                    h - 27.0,
                    200.0,
                    tr("cancel", "Cancel"),
                    true,
                    PICK_CANCEL,
                );
            }
            Page::Storage { .. } => {
                push(
                    cx - 154.0,
                    h - 52.0,
                    150.0,
                    tr("paste", "Paste"),
                    true,
                    PASTE,
                );
                push(
                    cx + 4.0,
                    h - 52.0,
                    150.0,
                    tr("setup_help", "How to Set Up..."),
                    true,
                    STORAGE_HELP,
                );
                let ready = self.storage_ready();
                push(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("share_world", "Share World"),
                    ready,
                    STORAGE_SHARE,
                );
                push(
                    cx + 4.0,
                    h - 27.0,
                    150.0,
                    tr("cancel", "Cancel"),
                    true,
                    STORAGE_CANCEL,
                );
            }
        }
        out
    }

    /// The world list's rows on the current page, for a row button's id.
    fn row(&self, id: u8, first: u8, h: f32, length: usize) -> Option<usize> {
        let n = rows(h);
        let offset = self.offset.min(length.saturating_sub(1) / n * n);
        let index = offset + usize::from(id - first);
        (index < length).then_some(index)
    }

    pub(super) fn click(&mut self, id: u8, h: f32, saves: &std::path::Path) -> Outcome {
        match id {
            TASK_CANCEL => shared_worlds::cancel(),
            TASK_OK => {
                if matches!(shared_worlds::task(), Task::Done(_)) {
                    self.page = Page::List;
                    self.offset = 0;
                    for field in &mut self.storage {
                        field.clear();
                    }
                }
                shared_worlds::dismiss();
            }
            BACK => return Outcome::Close,
            PREVIOUS | PICK_PREVIOUS => self.offset = self.offset.saturating_sub(rows(h)),
            NEXT | PICK_NEXT => self.offset += rows(h),
            ROW..=215 => {
                let entries = shared_worlds::entries();
                if let Some(i) = self.row(id, ROW, h, entries.len()) {
                    self.page = Page::World(entries[i].id.clone());
                    self.notice = None;
                    shared_worlds::refresh(true);
                }
            }
            PLAY => {
                if let Page::World(world) = &self.page {
                    return Outcome::Play(world.clone());
                }
            }
            COPY => {
                if let Page::World(world) = &self.page
                    && let Some(text) = shared_worlds::invite_text(world)
                {
                    SystemClipboard::default().set(&text);
                    self.copied = Some(Instant::now());
                    self.notice = Some(tr(
                        "invite_secret",
                        "Send it only to friends: whoever has it can play and change this world.",
                    ));
                }
            }
            REMOVE => {
                if let Page::World(world) = &self.page {
                    self.page = Page::Remove(world.clone());
                }
            }
            REMOVE_CONFIRM => {
                if let Page::Remove(world) = &self.page {
                    let world = world.clone();
                    self.page = Page::List;
                    self.notice = shared_worlds::remove(&world).err();
                }
            }
            REMOVE_CANCEL | WORLD_BACK | ADD_CANCEL | PICK_CANCEL | STORAGE_CANCEL => {
                self.leave_page()
            }
            ADD => {
                self.page = Page::Add;
                self.invite.clear();
                self.notice = None;
            }
            PASTE => {
                let text = SystemClipboard::default().get();
                let text = text.trim().to_owned();
                if let Some((_, field, max)) = self.field() {
                    *field = text.chars().take(max).collect();
                }
            }
            ADD_CONFIRM => match shared_worlds::add(&self.invite) {
                Ok(name) => {
                    self.invite.clear();
                    self.page = Page::List;
                    self.notice = Some(tr("added", "Added %s.").replacen("%s", &name, 1));
                }
                Err(e) => self.notice = Some(e),
            },
            SHARE => {
                self.local = crate::title::list_worlds(saves)
                    .into_iter()
                    .filter(|w| !w.corrupted && !w.locked && w.incompatible.is_none())
                    .map(|w| (w.folder, w.name))
                    .collect();
                self.offset = 0;
                self.notice = None;
                self.page = Page::Pick;
            }
            PICK_ROW..=235 => {
                if let Some(i) = self.row(id, PICK_ROW, h, self.local.len()) {
                    let (folder, name) = self.local[i].clone();
                    if let Some(bucket) = shared_worlds::last_bucket() {
                        self.storage = [
                            bucket.endpoint,
                            bucket.bucket,
                            bucket.access_key,
                            bucket.secret_key,
                        ];
                    }
                    self.focus = self.storage.iter().position(|f| f.is_empty()).unwrap_or(0);
                    self.page = Page::Storage { folder, name };
                }
            }
            STORAGE_HELP => {
                let _ = crate::platform::open_link(SETUP_URL);
            }
            STORAGE_SHARE => {
                if let Page::Storage { folder, name } = &self.page
                    && self.storage_ready()
                {
                    shared_worlds::create(folder.clone(), name.clone(), self.bucket());
                }
            }
            _ => {}
        }
        Outcome::None
    }
}

/// A task's progress or result covers the page.
fn task_shown() -> bool {
    !matches!(shared_worlds::task(), Task::Idle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_fit_the_screen() {
        assert_eq!(rows(262.0), 5);
        assert_eq!(rows(140.0), 1);
        assert_eq!(rows(2000.0), 6);
        // Six rows at most: their ids stay below the next range.
        assert!(ROW + 6 <= ADD_CONFIRM && PICK_ROW + 6 <= PICK_PREVIOUS);
    }

    #[test]
    fn a_status_reads_as_who_is_there() {
        assert_eq!(
            status_line(&Status::Known {
                playing: 0,
                host: None,
                open: false,
                tunnel: false
            }),
            "Nobody playing"
        );
        assert_eq!(
            status_line(&Status::Known {
                playing: 3,
                host: Some("Alex".into()),
                open: true,
                tunnel: false
            }),
            "3 playing, hosted by Alex"
        );
        assert_eq!(
            status_line(&Status::Known {
                playing: 1,
                host: Some("Alex".into()),
                open: false,
                tunnel: false
            }),
            "Alex is opening it"
        );
    }

    #[test]
    fn the_secret_key_is_masked_and_the_fields_cycle() {
        let mut screen = Screen {
            page: Page::Storage {
                folder: "f".into(),
                name: "n".into(),
            },
            ..Screen::default()
        };
        assert!(!screen.password_masked());
        for _ in 0..3 {
            assert!(screen.tab(false));
        }
        assert!(screen.password_masked());
        assert!(screen.tab(false));
        assert_eq!(screen.focus, 0);
        assert!(screen.tab(true));
        assert_eq!(screen.focus, 3);
        assert!(!format!("{screen:?}").contains("secret"));
    }
}
