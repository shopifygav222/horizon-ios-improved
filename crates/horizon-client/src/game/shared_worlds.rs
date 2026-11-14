//! Shared worlds in play: taking a world to host and opening it, keeping its copy in the storage
//! up to date while it runs, handing it on when the host leaves or gives it away, and following
//! the host as a guest, to the next host when it changes (`horizon-shared-worlds` does the
//! storage's part; `shared_worlds.rs` in the library keeps the list).
//!
//! The window's game holds the session ([`Shared`]); the world's server thread holds the
//! hosting handle for its checkpoints ([`Game::shared_checkpoint`]) and its freeze.
use super::Game;
use horizon_client::{
    Result,
    hud::HudBatch,
    lang,
    shared_screen::{self, Action, View},
    shared_worlds::{self as list, Task},
    text::Text,
};
use horizon_shared_worlds::{
    Error,
    lease::{Claim, Member},
    session::{self, Context, Hosting, Next, Start, Watcher, next_step},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TryRecvError, channel},
    },
    time::{Duration, Instant},
};

/// The engine's own messages between a shared world's host and its guests (`horizon:mod_net`).
pub(super) const CHANNEL: &str = "shared_worlds:ctl";
const REQUEST: &[u8] = b"request";
const DENY: &[u8] = b"deny";
const REACHABLE: &[u8] = b"reach:1";
const UNREACHABLE: &[u8] = b"reach:0";
/// How long a world may take to open after its copy is ready.
const OPEN_LIMIT: Duration = Duration::from_secs(180);
/// Messages from guests, from the server thread to the window.
pub(super) type Inbox = Arc<Mutex<Vec<(u128, Vec<u8>)>>>;

fn tr(key: &str, english: &str) -> String {
    lang::get_or(&format!("horizon.shared.{key}"), english)
}

fn uuid_text(uuid: u128) -> String {
    format!("{uuid:032x}")
}

/// A shared world this game is playing, hosting, or waiting on.
pub(super) struct Shared {
    name: String,
    ctx: Arc<Context>,
    phase: Phase,
    /// Copy Code was pressed.
    copied: Option<Instant>,
}

enum Phase {
    /// A thread takes the world and brings its newest copy onto this device.
    Preparing(Receiver<std::result::Result<Hosting, Error>>),
    Hosting(Box<Host>),
    /// The host closed the world; its last copy goes up.
    Leaving(Box<Leaving>),
    /// Someone else hosts it: joining, playing, or waiting for the next host.
    Watching(Box<Guest>),
}

struct Host {
    hosting: Arc<Hosting>,
    since: Instant,
    opened: bool,
    code: Option<String>,
    inbox: Inbox,
    reachable: HashMap<u128, bool>,
    /// Players who asked to host, (UUID, name).
    requests: Vec<(u128, String)>,
    /// Given the world with Give Host: hosts next.
    successor: Option<String>,
    players: Vec<Member>,
    listed: Option<Instant>,
    frozen_told: bool,
    /// Another game took the world: it closes without uploading.
    fenced: bool,
    /// Sessions through the public tunnel the player was told about.
    tunneled_told: u32,
}

struct Leaving {
    hosting: Arc<Hosting>,
    successor: Option<String>,
    rejoin: bool,
    done: Receiver<std::result::Result<(), Error>>,
    /// After a failed upload: when it is tried again, and the Cancel that keeps the world on
    /// this device instead.
    retry: Option<(Instant, Arc<AtomicBool>)>,
}

struct Guest {
    watcher: Watcher,
    /// The code this game connected with.
    joined: Option<String>,
    /// The connection to the host dropped (not left on purpose).
    lost: bool,
    next_try: Instant,
    asked: bool,
    reach: Option<Receiver<bool>>,
    reach_sent: bool,
    cancel: Arc<AtomicBool>,
    /// The newest copy this device holds or is fetching, and the fetch under way.
    fetched: Option<horizon_shared_worlds::manifest::GenId>,
    prefetch: Option<Receiver<()>>,
    /// The last join's path isn't known yet: the player is told once it is, if it is the
    /// public tunnel.
    tunnel_note: bool,
}

fn watch(ctx: &Arc<Context>) -> Result<Box<Guest>> {
    Ok(Box::new(Guest {
        watcher: Watcher::start(ctx.clone()).map_err(|e| e.to_string())?,
        joined: None,
        lost: false,
        next_try: Instant::now(),
        asked: false,
        reach: None,
        reach_sent: false,
        cancel: list::new_cancel(),
        fetched: None,
        prefetch: None,
        tunnel_note: false,
    }))
}

/// The public tunnel a host takes when nothing else reaches it: bore's free server, or
/// `HORIZON_SHARED_TUNNEL` (a host name; `none` turns it off).
fn public_tunnel() -> Option<horizon_friends::host::Tunnel> {
    let server = std::env::var("HORIZON_SHARED_TUNNEL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| horizon_shared_worlds::bore::DEFAULT_SERVER.to_owned());
    if server == "none" {
        return None;
    }
    Some(Arc::new(move |port: u16| {
        let local = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        match horizon_shared_worlds::bore::Tunnel::open(&server, local) {
            Ok(tunnel) => {
                log::info!(
                    "Shared worlds: friends reach this host through {server} at {}",
                    tunnel.address()
                );
                let address = tunnel.address();
                Some((address, Box::new(tunnel) as Box<dyn std::any::Any + Send>))
            }
            Err(e) => {
                log::warn!("Shared worlds: no public tunnel: {e}");
                None
            }
        }
    }))
}

fn working(what: String) {
    list::set_task(Task::Working {
        what,
        done: 0,
        total: 0,
    });
}

/// Whether friends on other networks can reach this device: the same test the host's
/// sharing makes, then put back as it was. Run on its own thread.
fn probe_reach() -> Receiver<bool> {
    let (tx, rx) = channel();
    let _ = std::thread::Builder::new()
        .name("shared-worlds-reach".into())
        .spawn(move || {
            let options = horizon_friends::reach::Options {
                map_port: true,
                stun: horizon_friends::stun::servers_from_env(),
                loopback: false,
            };
            // Never the port a host on the same network has its router open for: a port of its
            // own, given back at once.
            let offset = horizon_shared_worlds::codec::random::<1>().map_or(0, |b| u16::from(b[0]));
            let port = horizon_friends::host::DEFAULT_PORT + 1 + offset;
            let mut reach = horizon_friends::reach::Reach::discover(port, &options);
            let reachable = matches!(
                reach.verdict(),
                horizon_friends::reach::Verdict::Mapped(_)
                    | horizon_friends::reach::Verdict::Public
            );
            reach.release();
            let _ = tx.send(reachable);
        });
    rx
}

impl Game<'_> {
    /// How friends reach this host, for the Friends screen: nothing when every friend can
    /// connect directly.
    fn shared_route_line(&self) -> Option<String> {
        self.friends_tunnel()?;
        let mut line = match self.friends_verdict() {
            Some(horizon_friends::reach::Verdict::LocalOnly { cgnat: true }) => tr(
                "why_shared_address",
                "Friends on other networks may play through bore.pub (slower): your connection shares its address (mobile data, or a second router).",
            ),
            _ => tr(
                "why_router",
                "Friends on other networks may play through bore.pub (slower): your router didn't open a port. Turning on UPnP in its settings fixes that.",
            ),
        };
        let (direct, tunneled) = self.friends_paths();
        if direct + tunneled > 0 {
            line.push(' ');
            line.push_str(
                &tr("paths", "Direct: %d, bore.pub: %d.")
                    .replacen("%d", &direct.to_string(), 1)
                    .replacen("%d", &tunneled.to_string(), 1),
            );
        }
        Some(line)
    }

    /// The Shared Worlds screen's Start or Join: watch the world's lease, which says what to do.
    pub(super) fn shared_play(&mut self, world: &str) -> Result<()> {
        if self.shared.is_some() {
            return Ok(());
        }
        let ctx = match list::context(world) {
            Ok(ctx) => Arc::new(ctx),
            Err(e) => {
                list::set_task(Task::Failed(e));
                return Ok(());
            }
        };
        let name = list::entries()
            .into_iter()
            .find(|e| e.id == world)
            .map_or_else(|| world.to_owned(), |e| e.name);
        working(tr("connecting", "Reading the world's storage..."));
        let guest = watch(&ctx)?;
        self.shared = Some(Shared {
            name,
            ctx,
            copied: None,
            phase: Phase::Watching(guest),
        });
        Ok(())
    }

    /// The session's work for this frame.
    pub(super) fn tick_shared(&mut self) {
        let Some(mut shared) = self.shared.take() else {
            return;
        };
        let keep = match self.step_shared(&mut shared) {
            Ok(keep) => keep,
            Err(e) => {
                log::warn!("Shared worlds: {e}");
                list::set_task(Task::Failed(e.to_string()));
                false
            }
        };
        if keep {
            self.shared = Some(shared);
        }
    }

    /// One step; false ends the session.
    fn step_shared(&mut self, shared: &mut Shared) -> Result<bool> {
        match &mut shared.phase {
            Phase::Preparing(done) => match done.try_recv() {
                Err(TryRecvError::Empty) => Ok(true),
                Err(TryRecvError::Disconnected) => Err("getting the world ready stopped".into()),
                Ok(Ok(hosting)) => {
                    let hosting = Arc::new(hosting);
                    self.shared_open(shared, hosting)
                }
                Ok(Err(Error::Refused | Error::Busy)) => {
                    // Another player took it first (follow them), or this device's copy was
                    // still being brought up to date (look again).
                    shared.phase = Phase::Watching(watch(&shared.ctx)?);
                    Ok(true)
                }
                Ok(Err(Error::Cancelled)) => {
                    list::set_task(Task::Idle);
                    Ok(false)
                }
                Ok(Err(e)) => Err(e.to_string().into()),
            },
            Phase::Hosting(_) => self.shared_host_step(shared),
            Phase::Leaving(_) => self.shared_leave_step(shared),
            Phase::Watching(_) => self.shared_guest_step(shared),
        }
    }

    /// Takes the world to host, on a thread: the lease, then its newest copy onto this device.
    fn shared_prepare(&mut self, shared: &mut Shared, start: Start) {
        let (tx, rx) = channel();
        let ctx = shared.ctx.clone();
        let cancel = list::new_cancel();
        let name = shared.name.clone();
        let getting = tr("getting_ready", "Getting the world ready...");
        working(getting.clone());
        let spawned = std::thread::Builder::new()
            .name("shared-worlds-host".into())
            .spawn(move || {
                let folder = ctx.folder().to_path_buf();
                let recover_into = move || -> PathBuf {
                    let saves = folder.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                    let base = format!("{name} (my own progress)");
                    let mut path = saves.join(&base);
                    let mut n = 2;
                    while path.exists() {
                        path = saves.join(format!("{base} {n}"));
                        n += 1;
                    }
                    path
                };
                let mut progress = |done: u64, total: u64| {
                    list::set_task(Task::Working {
                        what: getting.clone(),
                        done,
                        total,
                    });
                    !cancel.load(Ordering::SeqCst)
                };
                let _ = tx.send(session::host(ctx, start, &recover_into, &mut progress));
            });
        match spawned {
            Ok(_) => shared.phase = Phase::Preparing(rx),
            Err(e) => list::set_task(Task::Failed(e.to_string())),
        }
    }

    /// The copy is ready: the host's own player, then the world opens.
    fn shared_open(&mut self, shared: &mut Shared, hosting: Arc<Hosting>) -> Result<bool> {
        if let Some(kept) = &hosting.recovered {
            // Progress of this device's that never reached the storage, kept as its own world.
            if let Some(folder) = kept.file_name().and_then(|n| n.to_str()) {
                let saves = kept.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                let label = format!("{} (my own progress)", shared.name);
                let _ = horizon_client::title::world_management::rename(&saves, folder, &label);
            }
        }
        let uuid = u128::from_str_radix(&shared.ctx.me.uuid, 16).unwrap_or(0);
        let folder = shared
            .ctx
            .folder()
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .ok_or("the world's folder has no name")?;
        let opened: Result<()> = match list::set_world_player(shared.ctx.folder(), uuid) {
            Ok(()) => self.open_world(&folder, None, None, false, None),
            Err(e) => Err(e.into()),
        };
        if let Err(e) = opened {
            let _ = hosting.release();
            return Err(e);
        }
        list::set_task(Task::Idle);
        shared.phase = Phase::Hosting(Box::new(Host {
            hosting,
            since: Instant::now(),
            opened: false,
            code: None,
            inbox: Arc::default(),
            reachable: HashMap::new(),
            requests: Vec::new(),
            successor: None,
            players: Vec::new(),
            listed: None,
            frozen_told: false,
            fenced: false,
            tunneled_told: 0,
        }));
        Ok(true)
    }

    fn shared_host_step(&mut self, shared: &mut Shared) -> Result<bool> {
        let Phase::Hosting(host) = &mut shared.phase else {
            return Ok(true);
        };
        let world_running = self.integrated.is_some();
        let world_starting = self.integrated_starting.is_some() || self.pending_entry.is_some();
        if !host.opened {
            if world_running && self.title.is_none() {
                host.opened = true;
                if let Some(server) = &self.integrated {
                    let (hosting, inbox) = (host.hosting.clone(), host.inbox.clone());
                    server.execute(move |g| {
                        g.shared_host = Some(hosting);
                        g.shared_inbox = Some(inbox);
                    });
                }
                if let Err(e) = self.friends_start_with(public_tunnel()) {
                    self.system_message(Text::literal(
                        tr("share_failed", "Friends can't join yet: %s").replacen(
                            "%s",
                            &e.to_string(),
                            1,
                        ),
                    ));
                }
            } else if !world_running && !world_starting && host.since.elapsed() > OPEN_LIMIT {
                let _ = host.hosting.release();
                return Err(tr("didnt_open", "The world didn't open.").into());
            }
            return Ok(true);
        }
        if !world_running && !world_starting {
            // The world closed and is saved: its last copy goes up, unless the world was taken.
            if host.fenced || host.hosting.fenced() {
                host.hosting.abandon();
                list::set_task(Task::Failed(tr(
                    "taken",
                    "Another player took over this world while your game couldn't reach its storage. What you did since its last upload is kept on this device.",
                )));
                return Ok(false);
            }
            let me = shared.ctx.me.uuid.clone();
            let successor = host
                .successor
                .clone()
                .or_else(|| session::successor(&host.players, &me));
            let rejoin = host.successor.is_some();
            let hosting = host.hosting.clone();
            shared.phase = Phase::Leaving(Box::new(self.shared_upload(hosting, successor, rejoin)));
            return Ok(true);
        }
        if host.hosting.fenced() && !host.fenced {
            host.fenced = true;
            self.system_message(Text::literal(tr(
                "taken_now",
                "Another player took over this world: it closes. Your progress since its last upload stays on this device.",
            )));
            if let Err(e) = self.leave_server(None) {
                log::warn!("Shared worlds: the world didn't close: {e}");
            }
            return Ok(true);
        }
        let frozen = host.hosting.must_freeze();
        if frozen && !host.frozen_told {
            host.frozen_told = true;
            self.system_message(Text::literal(tr(
                "frozen",
                "Your game can't reach the world's storage: the world waits until it can.",
            )));
        } else if !frozen && host.frozen_told {
            host.frozen_told = false;
            self.system_message(Text::literal(tr(
                "thawed",
                "The world's storage answers again.",
            )));
        }
        // The invite code that joins this game, once it is known. The world runs on its server
        // thread, so nothing else reads the sharing's news while no screen shows it.
        self.tick_friends();
        let view = self.friends_view();
        if let Some(code) = view.code
            && host.code.as_ref() != Some(&code)
        {
            let tunnel = self.friends_tunnel().map(|a| a.to_string());
            host.hosting.open(&code, tunnel.as_deref());
            host.code = Some(code);
        }
        // A friend whose game couldn't reach this one directly plays through the tunnel.
        let (_, tunneled) = self.friends_paths();
        if tunneled > host.tunneled_told {
            host.tunneled_told = tunneled;
            self.system_message(Text::literal(tr(
                "tunnel_host",
                "A friend couldn't connect to your game directly, so they play through bore.pub, a free public tunnel: it can be slower. The world stays encrypted between you and them.",
            )));
        }
        // What the guests said.
        let messages = std::mem::take(&mut *host.inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for (player, data) in messages {
            let name = self
                .remote_player_list()
                .into_iter()
                .find(|(uuid, _)| *uuid == player)
                .map(|(_, n)| n)
                .unwrap_or_default();
            match data.as_slice() {
                REQUEST if !host.requests.iter().any(|(u, _)| *u == player) => {
                    host.requests.push((player, name.clone()));
                    self.system_message(Text::literal(
                        tr(
                            "asks",
                            "%s asks to host this world: Game Menu, Friends button, to answer.",
                        )
                        .replacen("%s", &name, 1),
                    ));
                }
                REACHABLE => {
                    host.reachable.insert(player, true);
                }
                UNREACHABLE => {
                    host.reachable.insert(player, false);
                }
                _ => {}
            }
        }
        // Who is here, for the lease (the world list's count, and the next host).
        if host
            .listed
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
        {
            host.listed = Some(Instant::now());
            let me = &shared.ctx.me;
            let mut players = vec![Member {
                reachable: view.reachable,
                ..me.clone()
            }];
            for (uuid, name) in self.remote_player_list() {
                let text = uuid_text(uuid);
                if text != me.uuid {
                    players.push(Member {
                        uuid: text,
                        name,
                        reachable: host.reachable.get(&uuid).copied().unwrap_or(false),
                    });
                }
            }
            host.requests
                .retain(|(uuid, _)| players.iter().any(|p| p.uuid == uuid_text(*uuid)));
            host.hosting.set_players(players.clone());
            host.players = players;
        }
        if let Some(Err(e)) = host.hosting.poll()
            && !matches!(e, Error::Fenced)
        {
            log::warn!("Shared worlds: a checkpoint didn't upload, it goes with the next one: {e}");
        }
        Ok(true)
    }

    /// The last copy's upload, on a thread.
    fn shared_upload(
        &mut self,
        hosting: Arc<Hosting>,
        successor: Option<String>,
        rejoin: bool,
    ) -> Leaving {
        let (tx, rx) = channel();
        let uploading = tr("uploading_last", "Uploading the world for your friends...");
        working(uploading.clone());
        // The upload's progress shows on Shared Worlds, where the closed world leaves the title.
        if let Some(title) = self.title.as_mut() {
            title.open_shared();
        }
        let thread_hosting = hosting.clone();
        let thread_successor = successor.clone();
        horizon_client::platform::keep_working("shared-world", &uploading, true);
        let _ = std::thread::Builder::new()
            .name("shared-worlds-leave".into())
            .spawn(move || {
                let mut progress = |done: u64, total: u64| {
                    list::set_task(Task::Working {
                        what: uploading.clone(),
                        done,
                        total,
                    });
                    true
                };
                let result = thread_hosting
                    .leave(thread_successor.as_deref(), &mut progress)
                    .map(|_| ());
                horizon_client::platform::keep_working("shared-world", "", false);
                let _ = tx.send(result);
            });
        Leaving {
            hosting,
            successor,
            rejoin,
            done: rx,
            retry: None,
        }
    }

    fn shared_leave_step(&mut self, shared: &mut Shared) -> Result<bool> {
        let Phase::Leaving(leaving) = &mut shared.phase else {
            return Ok(true);
        };
        if let Some((at, cancel)) = &leaving.retry {
            if cancel.load(Ordering::SeqCst) {
                // The world stays this device's: its next start here carries on from it.
                leaving.hosting.abandon();
                list::set_task(Task::Done(tr(
                    "kept_here",
                    "The world stays on this device. Open it again to upload it for your friends.",
                )));
                return Ok(false);
            }
            if Instant::now() >= *at {
                let next = self.shared_upload(
                    leaving.hosting.clone(),
                    leaving.successor.clone(),
                    leaving.rejoin,
                );
                **leaving = next;
            }
            return Ok(true);
        }
        match leaving.done.try_recv() {
            Err(TryRecvError::Empty) => Ok(true),
            Err(TryRecvError::Disconnected) => Err("the upload stopped".into()),
            Ok(Ok(())) if leaving.rejoin => {
                working(tr(
                    "waiting_new_host",
                    "Waiting for the new host to open the world...",
                ));
                shared.phase = Phase::Watching(watch(&shared.ctx)?);
                Ok(true)
            }
            Ok(Ok(())) => {
                list::set_task(Task::Done(tr(
                    "uploaded",
                    "The world is saved for your friends.",
                )));
                Ok(false)
            }
            Ok(Err(Error::Fenced)) => {
                list::set_task(Task::Failed(tr(
                    "taken",
                    "Another player took over this world while your game couldn't reach its storage. What you did since its last upload is kept on this device.",
                )));
                Ok(false)
            }
            Ok(Err(e)) => {
                log::warn!("Shared worlds: the last copy didn't upload: {e}");
                let cancel = list::new_cancel();
                list::set_task(Task::Working {
                    what: tr(
                        "upload_retry",
                        "Couldn't upload the world (%s). Trying again... Cancel keeps it on this device until you open it again.",
                    )
                    .replacen("%s", &e.to_string(), 1),
                    done: 0,
                    total: 0,
                });
                leaving.retry = Some((Instant::now() + Duration::from_secs(10), cancel));
                Ok(true)
            }
        }
    }

    fn shared_guest_step(&mut self, shared: &mut Shared) -> Result<bool> {
        let Phase::Watching(guest) = &mut shared.phase else {
            return Ok(true);
        };
        let connected = self.remote.is_some() && self.integrated.is_none();
        if !connected && guest.cancel.load(Ordering::SeqCst) {
            list::set_task(Task::Idle);
            return Ok(false);
        }
        // Left the world on purpose: the session ends with it.
        if guest.joined.is_some() && !connected && !guest.lost && self.title.is_some() {
            list::set_task(Task::Idle);
            return Ok(false);
        }
        if connected {
            self.shared_report_reach(guest);
            let path = horizon_client::net::last_friend_path();
            if guest.tunnel_note
                && self.title.is_none()
                && path != horizon_client::net::FriendPath::Unknown
            {
                guest.tunnel_note = false;
                if path == horizon_client::net::FriendPath::Tunnel {
                    self.system_message(Text::literal(tr(
                        "tunnel_guest",
                        "Your game couldn't connect to the host's directly, so you play through bore.pub, a free public tunnel: it can be slower. The world stays encrypted between you and the host.",
                    )));
                }
            }
        }
        let Some((read, quiet)) = guest.watcher.latest() else {
            return Ok(true);
        };
        let seen = match read {
            Ok(Some(seen)) => seen,
            Ok(None) => return Err(tr("gone", "This world is no longer in its storage.").into()),
            Err(e) => {
                if !connected {
                    working(
                        tr(
                            "storage_retry",
                            "Can't reach the world's storage (%s). Trying again...",
                        )
                        .replacen("%s", &e.to_string(), 1),
                    );
                }
                return Ok(true);
            }
        };
        // While another game hosts, this device brings its copy up to each new one in the
        // background, so that taking the world later has almost nothing to download.
        if guest
            .prefetch
            .as_ref()
            .is_some_and(|rx| !matches!(rx.try_recv(), Err(TryRecvError::Empty)))
        {
            guest.prefetch = None;
        }
        if guest.prefetch.is_none()
            && seen.lease.generation.is_some()
            && guest.fetched != seen.lease.generation
            && seen
                .lease
                .host
                .as_ref()
                .is_some_and(|h| h.uuid != shared.ctx.me.uuid)
        {
            guest.fetched = seen.lease.generation;
            let (tx, rx) = channel();
            let ctx = shared.ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("shared-worlds-prefetch".into())
                .spawn(move || {
                    if let Err(e) = session::prefetch(&ctx, &mut |_, _| true) {
                        log::info!("Shared worlds: the copy wasn't brought up to date in the background: {e}");
                    }
                    let _ = tx.send(());
                });
            if spawned.is_ok() {
                guest.prefetch = Some(rx);
            }
        }
        let me = shared.ctx.me.uuid.clone();
        match next_step(Some(&seen.lease), quiet, &me, &shared.ctx.timing) {
            Next::Join { code, tunnel } => {
                if connected && guest.joined.as_ref() == Some(&code) {
                    return Ok(true);
                }
                if connected {
                    // A new host: leave the old one first.
                    self.leave_server(None)?;
                }
                if Instant::now() >= guest.next_try {
                    guest.next_try = Instant::now() + Duration::from_secs(15);
                    guest.joined = Some(code.clone());
                    guest.lost = false;
                    guest.asked = false;
                    guest.reach_sent = false;
                    guest.tunnel_note = true;
                    list::set_task(Task::Idle);
                    let name = shared.ctx.me.name.clone();
                    // The host's public tunnel, marked: dialled last, and left for a direct
                    // connection when the two games find one.
                    let tunnels: Vec<std::net::SocketAddr> =
                        tunnel.and_then(|t| t.parse().ok()).into_iter().collect();
                    self.connect_server_with_options(
                        &format!(
                            "{}{}",
                            horizon_friends::ADDRESS_PREFIX,
                            horizon_friends::with_tunnels(&code, &tunnels)
                        ),
                        &name,
                        None,
                        false,
                    )?;
                }
                Ok(true)
            }
            Next::Host(claim) => {
                if guest.prefetch.is_some() {
                    // The copy is still being brought up to date: hosting starts from it after.
                    return Ok(true);
                }
                if connected {
                    self.leave_server(None)?;
                }
                if let Some(title) = self.title.as_mut() {
                    title.open_shared();
                }
                let start = match claim {
                    Claim::Free => Start::Free,
                    Claim::Takeover => Start::Takeover(seen),
                };
                self.shared_prepare(shared, start);
                Ok(true)
            }
            Next::Wait => {
                if !connected {
                    let host = seen.lease.host.map(|h| h.name).unwrap_or_default();
                    working(
                        tr("waiting_host", "Waiting for %s to open the world...")
                            .replacen("%s", &host, 1),
                    );
                }
                Ok(true)
            }
            Next::Gone => Err(tr("gone", "This world is no longer in its storage.").into()),
        }
    }

    /// A guest tells the host once whether friends could reach this device, for choosing the
    /// next host.
    fn shared_report_reach(&mut self, guest: &mut Guest) {
        if guest.reach_sent {
            return;
        }
        let rx = guest.reach.get_or_insert_with(probe_reach);
        if let Ok(reachable) = rx.try_recv() {
            let message = if reachable { REACHABLE } else { UNREACHABLE };
            if self.native_net_send_to_server(CHANNEL, message).is_ok() {
                guest.reach_sent = true;
            } else {
                // Not joined yet: try again next frame with the same answer.
                guest.reach = Some({
                    let (tx, rx) = channel();
                    let _ = tx.send(reachable);
                    rx
                });
            }
        }
    }

    /// The connection to a shared world's host dropped. True when the session follows the
    /// world to its next host (the title shows that), instead of the Disconnected screen.
    pub(super) fn shared_connection_lost(&mut self) -> bool {
        let Some(Shared {
            phase: Phase::Watching(guest),
            ..
        }) = self.shared.as_mut()
        else {
            return false;
        };
        if guest.joined.is_none() {
            return false;
        }
        guest.lost = true;
        guest.next_try = Instant::now() + Duration::from_secs(3);
        working(tr(
            "host_changing",
            "The host left. Waiting for the next host...",
        ));
        true
    }

    /// After [`Self::shared_connection_lost`] took the player back to the title.
    pub(super) fn shared_back_to_list(&mut self) {
        if let Some(title) = self.title.as_mut() {
            title.open_shared();
        }
    }

    /// A message on [`CHANNEL`] from the host, on a guest.
    pub(super) fn shared_from_host(&mut self, data: &[u8]) {
        if data == DENY {
            self.system_message(Text::literal(tr(
                "denied",
                "The host wants to keep hosting for now.",
            )));
        }
    }

    /// The pause menu's Friends button in a shared world.
    pub(super) fn shared_active(&self) -> bool {
        self.shared.is_some()
    }

    fn shared_view(&self) -> View {
        let Some(shared) = &self.shared else {
            return View::default();
        };
        let mut view = View {
            world: shared.name.clone(),
            copied: shared
                .copied
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2)),
            ..View::default()
        };
        match &shared.phase {
            Phase::Hosting(host) => {
                view.code = host.code.clone();
                let others: Vec<&Member> = host
                    .players
                    .iter()
                    .filter(|p| p.uuid != shared.ctx.me.uuid)
                    .collect();
                let here = if others.is_empty() {
                    tr("alone", "Nobody else is here yet.")
                } else {
                    tr("here", "Here: %s").replacen(
                        "%s",
                        &others
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        1,
                    )
                };
                view.lines
                    .push(format!("{} {here}", tr("you_host", "You host this world.")));
                if host.code.is_none() {
                    view.lines.push(tr(
                        "not_open_yet",
                        "Friends can join once the invite code is ready.",
                    ));
                } else if let Some(line) = self.shared_route_line() {
                    view.lines.push(line);
                }
                if host.hosting.must_freeze() {
                    view.lines.push(tr(
                        "frozen",
                        "Your game can't reach the world's storage: the world waits until it can.",
                    ));
                }
                for (uuid, name) in &host.requests {
                    view.buttons.push((
                        tr("let_host", "Let %s Host").replacen("%s", name, 1),
                        Action::Accept(uuid_text(*uuid)),
                        true,
                    ));
                    view.buttons.push((
                        tr("say_no", "Keep Hosting (tell %s no)").replacen("%s", name, 1),
                        Action::Deny(uuid_text(*uuid)),
                        true,
                    ));
                }
                // Behind the tunnel, a player whose game takes direct connections hosts faster.
                let tunneled = self.friends_tunnel().is_some();
                for player in others {
                    if host
                        .requests
                        .iter()
                        .any(|(u, _)| uuid_text(*u) == player.uuid)
                    {
                        continue;
                    }
                    let label = if tunneled && player.reachable {
                        tr("give_host_direct", "Give Host to %s (faster)")
                    } else {
                        tr("give_host", "Give Host to %s")
                    };
                    view.buttons.push((
                        label.replacen("%s", &player.name, 1),
                        Action::GiveHost(player.uuid.clone()),
                        true,
                    ));
                }
            }
            Phase::Watching(guest) => {
                let host = guest
                    .watcher
                    .latest()
                    .and_then(|(read, _)| read.ok().flatten())
                    .and_then(|seen| seen.lease.host.map(|h| h.name))
                    .unwrap_or_default();
                view.lines
                    .push(tr("hosted_by", "Hosted by %s.").replacen("%s", &host, 1));
                let connected = self.remote.is_some() && self.integrated.is_none();
                if connected {
                    view.code = guest.joined.clone();
                    let path = horizon_client::net::last_friend_path();
                    if path == horizon_client::net::FriendPath::Tunnel {
                        view.lines.push(tr(
                            "via_tunnel",
                            "You play through bore.pub: your game couldn't connect to the host's directly.",
                        ));
                    } else if path == horizon_client::net::FriendPath::Direct {
                        view.lines
                            .push(tr("via_direct", "Connected to the host directly."));
                    }
                }
                let label = if guest.asked {
                    tr("asked", "Asked to Host")
                } else {
                    tr("ask_host", "Ask to Host")
                };
                view.buttons.push((label, Action::AskHost, !guest.asked));
            }
            Phase::Preparing(_) | Phase::Leaving(_) => {}
        }
        view
    }

    pub(super) fn draw_shared_screen(&mut self, batch: &mut HudBatch) {
        let view = self.shared_view();
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new("shared_world", shared_screen::TITLE)
        });
        shared_screen::draw(batch, &self.gui.atlas, self.cursor, &view);
    }

    pub(super) fn shared_screen_click(&mut self) {
        let view = self.shared_view();
        let Some(action) =
            shared_screen::click(&self.gui.atlas, self.gui_size(), self.cursor, &view)
        else {
            return;
        };
        self.audio.play_button_click();
        match action {
            Action::Done => self.screen = super::Screen::Paused,
            Action::CopyCode => {
                if let Some(code) = view.code {
                    self.copy_text(&code);
                    if let Some(shared) = self.shared.as_mut() {
                        shared.copied = Some(Instant::now());
                    }
                }
            }
            Action::AskHost => {
                if self.native_net_send_to_server(CHANNEL, REQUEST).is_ok()
                    && let Some(Shared {
                        phase: Phase::Watching(guest),
                        ..
                    }) = self.shared.as_mut()
                {
                    guest.asked = true;
                }
            }
            Action::Deny(uuid) => {
                let Ok(player) = u128::from_str_radix(&uuid, 16) else {
                    return;
                };
                if let Some(Shared {
                    phase: Phase::Hosting(host),
                    ..
                }) = self.shared.as_mut()
                {
                    host.requests.retain(|(u, _)| *u != player);
                }
                if let Some(server) = &self.integrated {
                    server.execute(move |g| {
                        let _ = g.native_net_send_to_player(player, CHANNEL, DENY);
                    });
                }
            }
            Action::GiveHost(uuid) | Action::Accept(uuid) => {
                let name = self.shared.as_ref().and_then(|s| match &s.phase {
                    Phase::Hosting(host) => host
                        .players
                        .iter()
                        .find(|p| p.uuid == uuid)
                        .map(|p| p.name.clone()),
                    _ => None,
                });
                if let Some(Shared {
                    phase: Phase::Hosting(host),
                    ..
                }) = self.shared.as_mut()
                {
                    host.successor = Some(uuid);
                }
                self.system_message(Text::literal(
                    tr("handing", "Handing the world to %s...").replacen(
                        "%s",
                        &name.unwrap_or_default(),
                        1,
                    ),
                ));
                // The world closes and saves; its last copy then goes up named for them.
                if let Err(e) = self.leave_server(None) {
                    log::warn!("Shared worlds: the world didn't close: {e}");
                }
            }
        }
    }

    /// The app leaves the screen while this game hosts: a checkpoint now, so that what was just
    /// done is in the storage if the system stops the game before it comes back.
    pub(super) fn shared_checkpoint_for_background(&mut self) -> bool {
        let Some(Shared {
            phase: Phase::Hosting(host),
            ..
        }) = &self.shared
        else {
            return false;
        };
        let (Some(server), true) = (&self.integrated, host.opened) else {
            return false;
        };
        let hosting = host.hosting.clone();
        horizon_client::platform::keep_working(
            "shared-world",
            &tr("uploading", "Uploading the world..."),
            true,
        );
        server.execute(move |g| {
            if let Err(e) = g.shared_checkpoint(&hosting) {
                log::warn!("Shared worlds: the checkpoint for the background failed: {e}");
            }
            // Let the system go on once the upload is over (or after a minute at most).
            let _ = std::thread::Builder::new()
                .name("shared-worlds-background".into())
                .spawn(move || {
                    let started = Instant::now();
                    while hosting.uploading() && started.elapsed() < Duration::from_secs(60) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    horizon_client::platform::keep_working("shared-world", "", false);
                });
        });
        true
    }

    /// The server thread: a checkpoint in place of an autosave. Everything is saved and written
    /// first; the changed files are then copied aside while nothing writes to them, and upload
    /// behind the game. False when it was put off (an upload still runs, or a write came in
    /// between): the caller tries again soon.
    pub(super) fn shared_checkpoint(&mut self, hosting: &Hosting) -> Result<bool> {
        if hosting.fenced() {
            return Ok(true);
        }
        if hosting.uploading() {
            self.save_game_with(false)?;
            return Ok(false);
        }
        let started = Instant::now();
        self.save_game()?;
        let saved = started.elapsed();
        use horizon_client::save::writes;
        let stamp = writes::stamp();
        let taken = writes::idle(stamp)
            && match hosting.capture(&|| writes::stamp() == stamp) {
                Ok(taken) => taken,
                Err(e) => {
                    log::warn!("Shared worlds: the checkpoint wasn't taken: {e}");
                    false
                }
            };
        log::info!(
            "Shared worlds: checkpoint {} (saving {} ms, copying {} ms)",
            if taken { "taken" } else { "put off" },
            saved.as_millis(),
            started.elapsed().saturating_sub(saved).as_millis()
        );
        Ok(taken)
    }

    /// The server thread: whether the world must wait for its storage (or was taken).
    pub(super) fn shared_frozen(&self) -> bool {
        self.shared_host.as_ref().is_some_and(|h| h.must_freeze())
    }

    /// The server thread: a guest's message on [`CHANNEL`], for the window.
    pub(super) fn shared_from_guest(&mut self, player: u128, data: &[u8]) {
        if let Some(inbox) = &self.shared_inbox {
            let mut inbox = inbox.lock().unwrap_or_else(|e| e.into_inner());
            if inbox.len() < 64 {
                inbox.push((player, data.to_vec()));
            }
        }
    }

    /// `horizon-ctl shared ...`, for the end-to-end checks.
    pub(super) fn shared_control(&mut self, words: &[&str]) -> Result<Option<serde_json::Value>> {
        use serde_json::json;
        if words.first() != Some(&"shared") {
            return Ok(None);
        }
        match words.get(1).copied() {
            Some("list") => {
                let worlds: Vec<serde_json::Value> = list::entries()
                    .into_iter()
                    .map(|e| {
                        json!({
                            "id": e.id,
                            "name": e.name,
                            "folder": e.folder,
                            "status": format!("{:?}", e.status),
                        })
                    })
                    .collect();
                Ok(Some(json!({ "worlds": worlds })))
            }
            Some("add") => {
                let text = words.get(2).ok_or("shared add INVITE")?;
                let name = list::add(text)?;
                Ok(Some(json!({ "ok": true, "name": name })))
            }
            Some("create") => {
                // shared create FOLDER NAME ENDPOINT BUCKET KEY SECRET
                let [folder, name, endpoint, bucket, key, secret] = [2, 3, 4, 5, 6, 7]
                    .map(|i| words.get(i).copied().unwrap_or_default().to_owned());
                list::create(
                    folder,
                    name,
                    horizon_shared_worlds::s3::Bucket {
                        endpoint,
                        bucket,
                        region: "us-east-1".into(),
                        access_key: key,
                        secret_key: secret,
                    },
                );
                Ok(Some(json!({ "ok": true })))
            }
            Some("invite") => {
                let id = words.get(2).ok_or("shared invite ID")?;
                Ok(Some(json!({ "invite": list::invite_text(id) })))
            }
            Some("play") => {
                let id = words.get(2).ok_or("shared play ID")?;
                self.shared_play(id)?;
                Ok(Some(json!({ "ok": true })))
            }
            Some("give") => {
                let uuid = words.get(2).ok_or("shared give UUID")?.to_string();
                if let Some(Shared {
                    phase: Phase::Hosting(host),
                    ..
                }) = self.shared.as_mut()
                {
                    host.successor = Some(uuid);
                }
                self.leave_server(None)?;
                Ok(Some(json!({ "ok": true })))
            }
            Some("ask") => {
                self.native_net_send_to_server(CHANNEL, REQUEST)?;
                Ok(Some(json!({ "ok": true })))
            }
            Some("status") => {
                let task = format!("{:?}", list::task());
                let state = self.shared.as_ref().map(|s| match &s.phase {
                    Phase::Preparing(_) => json!({ "phase": "preparing" }),
                    Phase::Hosting(host) => json!({
                        "phase": "hosting",
                        "opened": host.opened,
                        "code": host.code,
                        "players": host.players.iter().map(|p| &p.name).collect::<Vec<_>>(),
                        "requests": host.requests.iter().map(|(_, n)| n).collect::<Vec<_>>(),
                        "committed": host.hosting.committed().map(|g| format!("{g:?}")),
                        "uploading": host.hosting.uploading(),
                        "frozen": host.hosting.must_freeze(),
                        "epoch": host.hosting.epoch(),
                    }),
                    Phase::Leaving(_) => json!({ "phase": "leaving" }),
                    Phase::Watching(guest) => json!({
                        "phase": "watching",
                        "joined": guest.joined,
                        "lost": guest.lost,
                    }),
                });
                Ok(Some(json!({ "task": task, "session": state })))
            }
            Some("checkpoint") => {
                let Some(Shared {
                    phase: Phase::Hosting(host),
                    ..
                }) = &self.shared
                else {
                    return Err("not hosting a shared world".into());
                };
                let hosting = host.hosting.clone();
                let server = self.integrated.as_ref().ok_or("the world isn't open")?;
                let taken = server
                    .call(move |g| g.shared_checkpoint(&hosting).map_err(|e| e.to_string()))
                    .ok_or(super::integrated::NO_ANSWER)??;
                Ok(Some(json!({ "taken": taken })))
            }
            _ => Err("shared list|add INVITE|create ...|invite ID|play ID|give UUID|ask|status|checkpoint".into()),
        }
    }
}
