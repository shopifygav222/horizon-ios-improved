//! The shared worlds on this device, as the title's Shared Worlds screen lists them
//! (`title/shared.rs`): which worlds this player is a member of, the invite each one was added
//! with, and what their storage says about them now (who hosts, how many are playing).
//!
//! The invites hold their bucket's key, so they live in the platform's sealed store (the
//! Keychain on iOS), one secret each; the list of worlds beside it holds only ids, names and
//! folders. Opening a world to play is the game's (`game/shared_worlds.rs`): this module hands
//! it a [`Context`] for the world.
//!
//! Every network call runs on a thread of its own; the screen reads [`entries`] and [`task`]
//! as it draws.
use horizon_shared_worlds::{
    codec::{hex, random},
    invite::WorldInvite,
    lease::{Member, State as LeaseState},
    manifest::WORK,
    s3::{Bucket, S3Store},
    session::{self, Context, Marker},
    store::Store,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// How often the list reads its worlds' leases while it is shown.
const REFRESH: Duration = Duration::from_secs(10);
const LIST_FILE: &str = "shared-worlds.json";
const DEVICE_FILE: &str = "shared-worlds-device.txt";
const BUCKET_SECRET: &str = "sharedworldbucket";

fn secret_key(world: &str) -> String {
    format!("sharedworld{world}")
}

fn tr(key: &str, english: &str) -> String {
    crate::lang::get_or(key, english)
}

/// What the storage says about one world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not read yet.
    Unknown,
    Known {
        playing: usize,
        host: Option<String>,
        /// Open to join now.
        open: bool,
        tunnel: bool,
    },
    /// Its storage couldn't be read; the message says why.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// Its folder under `saves`.
    pub folder: String,
    pub status: Status,
}

/// Making or adding a world, in the background.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Task {
    #[default]
    Idle,
    Working {
        what: String,
        done: u64,
        total: u64,
    },
    Failed(String),
    Done(String),
}

#[derive(Serialize, Deserialize)]
struct Listed {
    id: String,
    name: String,
    folder: String,
}

#[derive(Default)]
struct State {
    loaded: bool,
    entries: Vec<Entry>,
    invites: std::collections::HashMap<String, WorldInvite>,
    task: Task,
    cancel: Option<Arc<AtomicBool>>,
    refreshing: bool,
    refreshed: Option<Instant>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = state();
    f(guard.get_or_insert_with(State::default))
}

fn game_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

/// The folder worlds are saved in.
pub fn saves() -> PathBuf {
    game_dir().join("saves")
}

/// This install's own name, so the same player on two devices is two hosts.
pub fn device() -> String {
    let path = game_dir().join(DEVICE_FILE);
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return id.to_owned();
        }
    }
    let id = random::<16>().map(|b| hex(&b)).unwrap_or_else(|_| "0".repeat(32));
    let _ = horizon_shared_worlds::scan::write_atomic(&path, id.as_bytes());
    id
}

/// The player as a shared world knows them: a Microsoft account, which proves who they are to
/// the other players' games. An offline or Ely.by player can't take part.
pub fn me() -> Result<Member, String> {
    let Some(session) = crate::auth::current() else {
        return Err(tr(
            "horizon.shared.needs_account",
            "Shared worlds need a Microsoft account: sign in first.",
        ));
    };
    if session.is_ely() {
        return Err(tr(
            "horizon.shared.needs_microsoft",
            "Shared worlds need a Microsoft account, not an Ely.by one.",
        ));
    }
    Ok(Member {
        uuid: format!("{:032x}", session.uuid),
        name: session.name.clone(),
        reachable: false,
    })
}

fn save_list(state: &State) -> Result<(), String> {
    let listed: Vec<Listed> = state
        .entries
        .iter()
        .map(|e| Listed {
            id: e.id.clone(),
            name: e.name.clone(),
            folder: e.folder.clone(),
        })
        .collect();
    let data = serde_json::to_vec_pretty(&listed).map_err(|e| e.to_string())?;
    horizon_shared_worlds::scan::write_atomic(&game_dir().join(LIST_FILE), &data).map_err(|e| e.to_string())
}

/// Reads the list and its invites once.
pub fn load() {
    with(|state| {
        if state.loaded {
            return;
        }
        state.loaded = true;
        let listed: Vec<Listed> = std::fs::read(game_dir().join(LIST_FILE))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        let secrets = crate::accounts::store();
        for world in listed {
            match secrets.get(&secret_key(&world.id)) {
                Ok(Some(text)) => match WorldInvite::decode(&text) {
                    Ok(invite) => {
                        state.invites.insert(world.id.clone(), invite);
                    }
                    Err(e) => log::warn!("Shared worlds: the invite of {} doesn't read: {e}", world.name),
                },
                Ok(None) => log::warn!("Shared worlds: this device no longer holds the invite of {}", world.name),
                Err(e) => log::warn!("Shared worlds: the invite of {} wasn't read: {e}", world.name),
            }
            state.entries.push(Entry {
                id: world.id,
                name: world.name,
                folder: world.folder,
                status: Status::Unknown,
            });
        }
    });
}

pub fn entries() -> Vec<Entry> {
    load();
    with(|state| state.entries.clone())
}

pub fn task() -> Task {
    with(|state| state.task.clone())
}

/// Clears a finished task's message.
pub fn dismiss() {
    with(|state| {
        if !matches!(state.task, Task::Working { .. }) {
            state.task = Task::Idle;
        }
    });
}

/// Calls off the task under way, at its next step.
pub fn cancel() {
    with(|state| {
        if let Some(cancel) = &state.cancel {
            cancel.store(true, Ordering::SeqCst);
        }
    });
}

/// The world's invite, to send to friends.
pub fn invite_text(id: &str) -> Option<String> {
    load();
    with(|state| state.invites.get(id).and_then(|i| i.encode().ok()))
}

fn store_for(invite: &WorldInvite) -> Result<Arc<dyn Store>, String> {
    Ok(Arc::new(S3Store::new(&invite.bucket).map_err(|e| e.to_string())?))
}

/// What the game needs to open world `id`: its storage, its folder, and this player.
pub fn context(id: &str) -> Result<Context, String> {
    load();
    let me = me()?;
    let (invite, folder) = with(|state| {
        let invite = state.invites.get(id).cloned();
        let folder = state.entries.iter().find(|e| e.id == id).map(|e| e.folder.clone());
        (invite, folder)
    });
    let invite = invite.ok_or_else(|| {
        tr(
            "horizon.shared.no_invite",
            "This device no longer has this world's invite: add it again with the invite a friend sends.",
        )
    })?;
    let folder = folder.ok_or("no such shared world")?;
    Ok(Context::new(store_for(&invite)?, &invite, saves().join(folder), me, device()))
}

/// Reads every world's lease again, in the background, at most every [`REFRESH`] unless `now`.
pub fn refresh(now: bool) {
    load();
    let worlds = with(|state| {
        let due = state.refreshed.is_none_or(|at| at.elapsed() >= REFRESH);
        if state.refreshing || !(now || due) {
            return None;
        }
        state.refreshing = true;
        Some(state.invites.clone())
    });
    let Some(worlds) = worlds else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("shared-worlds-list".into())
        .spawn(move || {
            for (id, invite) in worlds {
                let status = match store_for(&invite).and_then(|store| {
                    horizon_shared_worlds::lease::Leases::new(store, &format!("{}lease", invite.prefix()))
                        .read()
                        .map_err(|e| e.to_string())
                }) {
                    Ok(Some(seen)) => {
                        let lease = seen.lease;
                        Status::Known {
                            playing: lease.playing(),
                            host: lease
                                .host
                                .filter(|_| matches!(lease.state, LeaseState::Hosting | LeaseState::Starting))
                                .map(|h| h.name),
                            open: lease.state == LeaseState::Hosting && lease.code.is_some(),
                            tunnel: lease.tunnel,
                        }
                    }
                    Ok(None) => Status::Failed(tr(
                        "horizon.shared.gone",
                        "This world is no longer in its storage.",
                    )),
                    Err(e) => Status::Failed(e),
                };
                with(|state| {
                    if let Some(entry) = state.entries.iter_mut().find(|e| e.id == id) {
                        entry.status = status;
                    }
                });
            }
            with(|state| {
                state.refreshing = false;
                state.refreshed = Some(Instant::now());
            });
        });
    if spawned.is_err() {
        with(|state| state.refreshing = false);
    }
}

/// A folder name under `saves` for a world called `name`, not in use yet.
fn new_folder(name: &str) -> String {
    let base: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>()
        .trim()
        .chars()
        .take(40)
        .collect();
    let base = if base.is_empty() { "Shared World".to_owned() } else { base };
    let saves = saves();
    let mut folder = base.clone();
    let mut n = 2;
    while saves.join(&folder).exists() {
        folder = format!("{base} ({n})");
        n += 1;
    }
    folder
}

fn remember(state: &mut State, invite: WorldInvite, folder: String) -> Result<(), String> {
    let id = invite.world.clone();
    let text = invite.encode().map_err(|e| e.to_string())?;
    crate::accounts::store().set(&secret_key(&id), &text)?;
    state.entries.retain(|e| e.id != id);
    state.entries.push(Entry {
        id: id.clone(),
        name: invite.name.clone(),
        folder,
        status: Status::Unknown,
    });
    state.invites.insert(id, invite);
    save_list(state)
}

/// Adds the world a friend's invite names. Nothing is downloaded until it is played.
pub fn add(text: &str) -> Result<String, String> {
    load();
    let invite = WorldInvite::decode(text).map_err(|e| e.to_string())?;
    S3Store::new(&invite.bucket).map_err(|e| e.to_string())?;
    let name = invite.name.clone();
    with(|state| {
        if state.entries.iter().any(|e| e.id == invite.world) {
            return Err(tr("horizon.shared.already_added", "This world is already in your list."));
        }
        let folder = new_folder(&name);
        remember(state, invite, folder)
    })?;
    refresh(true);
    Ok(name)
}

/// The bucket used last to make a world, so the next one doesn't need it typed again.
pub fn last_bucket() -> Option<Bucket> {
    let text = crate::accounts::store().get(BUCKET_SECRET).ok()??;
    serde_json::from_str(&text).ok()
}

/// Makes the closed singleplayer world in `saves/<folder>` a shared world kept in `bucket`, in
/// the background: the bucket is checked, the world uploaded, and the world moves from the
/// Singleplayer list to the Shared Worlds list.
pub fn create(folder: String, name: String, bucket: Bucket) {
    load();
    let busy = with(|state| {
        if matches!(state.task, Task::Working { .. }) {
            return true;
        }
        state.task = Task::Working {
            what: tr("horizon.shared.checking_storage", "Checking the storage..."),
            done: 0,
            total: 0,
        };
        false
    });
    if busy {
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    with(|state| state.cancel = Some(cancel.clone()));
    let spawned = std::thread::Builder::new()
        .name("shared-worlds-create".into())
        .spawn(move || {
            let made = (|| -> Result<String, String> {
                let me = me()?;
                let path = saves().join(&folder);
                if Marker::read(&path).is_some() {
                    return Err(tr("horizon.shared.already_shared", "This world is already shared."));
                }
                let invite = WorldInvite {
                    bucket: bucket.clone(),
                    world: WorldInvite::new_id().map_err(|e| e.to_string())?,
                    name: name.clone(),
                };
                invite.encode().map_err(|e| e.to_string())?;
                let ctx = Context::new(store_for(&invite)?, &invite, path, me, device());
                let uploading = tr("horizon.shared.uploading", "Uploading the world...");
                let mut progress = |done: u64, total: u64| {
                    with(|state| {
                        state.task = Task::Working {
                            what: uploading.clone(),
                            done,
                            total,
                        }
                    });
                    !cancel.load(Ordering::SeqCst)
                };
                session::create(&ctx, &name, &mut progress).map_err(|e| e.to_string())?;
                let _ = serde_json::to_string(&bucket)
                    .map_err(|e| e.to_string())
                    .and_then(|text| crate::accounts::store().set(BUCKET_SECRET, &text));
                with(|state| remember(state, invite.clone(), folder.clone()))?;
                invite.encode().map_err(|e| e.to_string())
            })();
            with(|state| {
                state.cancel = None;
                state.task = match made {
                    Ok(_) => Task::Done(tr(
                        "horizon.shared.created",
                        "The world is shared. Copy its invite and send it to your friends.",
                    )),
                    Err(e) => Task::Failed(e),
                };
            });
            refresh(true);
        });
    if spawned.is_err() {
        with(|state| state.task = Task::Failed("the game couldn't start the upload".into()));
    }
}

/// Takes the world off this device's list: it stays in its storage for the others, and this
/// device's copy becomes a normal singleplayer world (nothing is deleted).
pub fn remove(id: &str) -> Result<(), String> {
    load();
    with(|state| {
        let Some(at) = state.entries.iter().position(|e| e.id == id) else {
            return Ok(());
        };
        let entry = state.entries.remove(at);
        state.invites.remove(id);
        crate::accounts::store().delete(&secret_key(id))?;
        let work = saves().join(&entry.folder).join(WORK);
        if work.exists() {
            let _ = std::fs::remove_dir_all(&work);
        }
        save_list(state)
    })
}

/// Shows `task` on the Shared Worlds screen: the game's own work for a world (getting it ready,
/// uploading it, waiting for a new host).
pub fn set_task(task: Task) {
    with(|state| {
        if task == Task::Idle {
            state.cancel = None;
        }
        state.task = task;
    });
}

/// A new cancel switch for the game's task: the screen's Cancel turns it on.
pub fn new_cancel() -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    with(|state| state.cancel = Some(cancel.clone()));
    cancel
}

/// Makes the player with `uuid` the world's own player before the world opens (level.dat's
/// `singleplayer_uuid`), so the host loads their own saved state, never the last host's. A
/// player kept in level.dat itself (a world vanilla saved) goes to its own file first.
pub fn set_world_player(world: &Path, uuid: u128) -> Result<(), String> {
    use crate::{
        lan::playerdata,
        save::{put, read_dat, uuid_from_tag, uuid_tag, write_dat},
    };
    use horizon_world::nbt::Tag;
    let path = world.join("level.dat");
    let mut root = read_dat(&path).ok_or("the world's level.dat doesn't read")?;
    let Some(data) = root.get_mut("Data") else {
        return Err("the world's level.dat holds no world".into());
    };
    let legacy = match data {
        Tag::Compound(map) => map.remove(&horizon_world::nbt::JavaString::from("Player")),
        _ => None,
    };
    if let Some(player) = legacy {
        let owner = uuid_from_tag(player.get("UUID")).or_else(|| uuid_from_tag(data.get("singleplayer_uuid")));
        if let Some(owner) = owner {
            let file = playerdata::path(world, playerdata::ints_uuid(owner));
            if !file.exists() {
                write_dat(&file, player).map_err(|e| e.to_string())?;
            }
        }
    }
    put(data, "singleplayer_uuid", uuid_tag(playerdata::uuid_ints(uuid)));
    write_dat(&path, root).map_err(|e| e.to_string())
}

/// Whether `world` (a folder under `saves`) is a shared world's copy: the Singleplayer list
/// leaves it out, since it opens only through the Shared Worlds list.
pub fn is_shared(world: &Path) -> bool {
    world.join(WORK).join("world.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_keep_to_safe_characters() {
        let name = new_folder("The: Island/../x");
        assert!(!name.contains(['/', ':', '.']), "{name}");
        assert!(new_folder("   ").starts_with("Shared World"));
    }
}
