//! The lease: the one record that says who hosts a shared world, who is playing and which copy
//! of the save is the newest.
//!
//! Every change is a conditional write on the version that was read, so of two games that try
//! to take the world at once the storage lets exactly one through, and a host that was replaced
//! can't write again: its next write is refused and it learns it is [`Error::Fenced`].
//!
//! Nothing here compares clocks between devices. A host that stops is noticed by its heartbeat
//! counter standing still on the watcher's own timer ([`Watch`], [`DEAD`]), and a host that
//! can't reach the storage freezes its world well before that ([`Pulse`], [`SELF_FENCE`]), so by
//! the time anyone may take over, the old host has already stopped changing the world.
use crate::{
    Error, Result,
    codec::{hex, random},
    manifest::GenId,
    store::{Condition, Etag, Store},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// The record's format. A lease written by a newer format is never touched.
pub const VERSION: u32 = 1;
/// How often a host rewrites the lease.
pub const HEARTBEAT: Duration = Duration::from_secs(15);
/// A host that hasn't managed to write the lease for this long freezes its world.
pub const SELF_FENCE: Duration = Duration::from_secs(45);
/// A lease whose heartbeat hasn't moved for this long may be taken over.
pub const DEAD: Duration = Duration::from_secs(90);
/// Players waiting to take over go one at a time, this far apart, in the order of the lease's
/// player list.
pub const TURN: Duration = Duration::from_secs(3);

// The margin the scheme rests on, checked when the crate is built: a host that can't write
// the lease has frozen, with two more heartbeats to spare, before anyone may take over.
const _: () = assert!(SELF_FENCE.as_secs() + 2 * HEARTBEAT.as_secs() < DEAD.as_secs());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// A host has the world and is getting it ready: nobody can join yet.
    Starting,
    /// The world is open: `code` joins it.
    Hosting,
    /// The host is leaving and has published its last copy; `successor` takes the world next.
    Handoff,
    /// Nobody hosts it. Whoever opens it next does.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The player's account UUID, as 32 hex digits.
    pub uuid: String,
    pub name: String,
    /// Friends on other networks can reach this player's game: a good next host.
    #[serde(default)]
    pub reachable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub version: u32,
    /// Counts hosts: each game that takes the world raises it by one.
    pub epoch: u64,
    pub state: State,
    pub host: Option<Member>,
    /// The host's device, so the same player on another device is another host.
    pub device: String,
    /// The player (UUID) who takes the world after a handoff; None leaves it to whoever comes.
    pub successor: Option<String>,
    /// Raised with every write: a heartbeat that stands still is a host that stopped.
    pub beat: u64,
    /// Random with every write, so a writer can tell its own write from someone else's.
    pub token: String,
    /// The invite code that joins the host, while hosting.
    pub code: Option<String>,
    /// The code goes through the public tunnel.
    pub tunnel: bool,
    /// Who is in the world, the host first.
    pub players: Vec<Member>,
    /// The newest copy of the save. A copy counts only once it is named here.
    pub generation: Option<GenId>,
}

impl Lease {
    /// How many are playing, as the world list shows it.
    pub fn playing(&self) -> usize {
        match self.state {
            State::Hosting | State::Starting => self.players.len().max(1),
            State::Handoff | State::Closed => 0,
        }
    }
}

/// A lease as the storage had it, with the tag a change must name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub lease: Lease,
    pub etag: Etag,
}

/// Why a game asks for the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// Nobody has it: it is closed, was never opened, or is handed to this player.
    Free,
    /// Its host stopped: the caller watched the heartbeat stand still for [`DEAD`].
    Takeover,
}

/// A world's lease in its storage.
#[derive(Clone)]
pub struct Leases {
    store: Arc<dyn Store>,
    key: String,
}

fn token() -> Result<String> {
    Ok(hex(&random::<12>()?))
}

impl Leases {
    /// `key` is the lease's full key in the bucket (`w/<world>/lease`).
    pub fn new(store: Arc<dyn Store>, key: &str) -> Self {
        Self {
            store,
            key: key.to_owned(),
        }
    }

    pub fn read(&self) -> Result<Option<Seen>> {
        let Some(object) = self.store.get(&self.key)? else {
            return Ok(None);
        };
        let lease: Lease = serde_json::from_slice(&object.data)?;
        if lease.version > VERSION {
            return Err(Error::Format(
                "this world was last hosted by a newer version of the game: update to play it"
                    .into(),
            ));
        }
        Ok(Some(Seen {
            lease,
            etag: object.etag,
        }))
    }

    /// Takes the world for `me`. `seen` is the lease as last read (None: there is none yet).
    /// [`Error::Refused`] when the lease doesn't allow it or changed since: read it again.
    pub fn claim(
        &self,
        seen: Option<&Seen>,
        me: &Member,
        device: &str,
        why: Claim,
    ) -> Result<Held> {
        let (epoch, generation, condition) = match seen {
            None => (1, None, Condition::Absent),
            Some(seen) => {
                let lease = &seen.lease;
                let free = match lease.state {
                    State::Closed => true,
                    State::Handoff => lease.successor.as_ref().is_none_or(|s| *s == me.uuid),
                    State::Starting | State::Hosting => false,
                };
                if !(free || why == Claim::Takeover) {
                    return Err(Error::Refused);
                }
                (
                    lease.epoch + 1,
                    lease.generation,
                    Condition::Unchanged(seen.etag.clone()),
                )
            }
        };
        let lease = Lease {
            version: VERSION,
            epoch,
            state: State::Starting,
            host: Some(me.clone()),
            device: device.to_owned(),
            successor: None,
            beat: 0,
            token: token()?,
            code: None,
            tunnel: false,
            players: vec![me.clone()],
            generation,
        };
        let data = serde_json::to_vec(&lease)?;
        match self.store.put(&self.key, &data, &condition) {
            Ok(etag) => Ok(Held::new(self.clone(), Seen { lease, etag })),
            // The write may have landed although its answer was lost.
            Err(error) => match self.read() {
                Ok(Some(found)) if found.lease.token == lease.token => {
                    Ok(Held::new(self.clone(), found))
                }
                _ => Err(error),
            },
        }
    }

    /// The same game picking its own lease up again after a restart, without a new epoch: the
    /// lease still names this player on this device, and nobody took it meanwhile.
    pub fn resume(&self, seen: &Seen, me: &Member, device: &str) -> Result<Held> {
        let lease = &seen.lease;
        let mine = matches!(lease.state, State::Starting | State::Hosting)
            && lease.host.as_ref().is_some_and(|h| h.uuid == me.uuid)
            && lease.device == device;
        if !mine {
            return Err(Error::Refused);
        }
        let mut held = Held::new(self.clone(), seen.clone());
        held.update(|lease| {
            lease.state = State::Starting;
            lease.code = None;
            lease.tunnel = false;
            lease.players = vec![me.clone()];
        })?;
        Ok(held)
    }
}

/// The lease while this game holds it. Every method is one conditional write; after
/// [`Error::Fenced`] the world is someone else's and this value is of no further use.
pub struct Held {
    leases: Leases,
    seen: Seen,
    /// Tokens of writes whose answer never came: any of them may still land.
    unanswered: Vec<String>,
}

impl Held {
    fn new(leases: Leases, seen: Seen) -> Self {
        Self {
            leases,
            seen,
            unanswered: Vec::new(),
        }
    }

    pub fn lease(&self) -> &Lease {
        &self.seen.lease
    }

    /// Changes the lease and raises its heartbeat.
    /// - [`Error::Fenced`]: another game took the world. Stop.
    /// - [`Error::Store`]: the storage wasn't reached, or the answer was lost. The lease may or
    ///   may not hold the change; calling again is safe.
    pub fn update(&mut self, change: impl Fn(&mut Lease)) -> Result<()> {
        // Twice at most: the second time on top of an earlier write of ours that landed late.
        for _ in 0..2 {
            let mut next = self.seen.lease.clone();
            change(&mut next);
            next.beat += 1;
            next.token = token()?;
            let data = serde_json::to_vec(&next)?;
            let condition = Condition::Unchanged(self.seen.etag.clone());
            let error = match self.leases.store.put(&self.leases.key, &data, &condition) {
                Ok(etag) => {
                    self.seen = Seen { lease: next, etag };
                    self.unanswered.clear();
                    return Ok(());
                }
                Err(error @ (Error::Refused | Error::Store(_))) => error,
                Err(other) => return Err(other),
            };
            self.unanswered.push(next.token.clone());
            let found = match self.leases.read() {
                Ok(found) => found,
                // Not known: the lease may be ours still. The caller's own timer decides.
                Err(_) => return Err(Error::Store("the lease couldn't be read back".into())),
            };
            match found {
                Some(found) if found.lease.token == next.token => {
                    // This very write landed; only its answer was lost.
                    self.seen = found;
                    self.unanswered.clear();
                    return Ok(());
                }
                Some(found) if self.unanswered.contains(&found.lease.token) => {
                    // An earlier write of ours landed after we gave up on it: go on from it.
                    self.seen = found;
                    self.unanswered.clear();
                }
                Some(found) if found.etag == self.seen.etag => {
                    // Still the version we hold: the write didn't land, and nobody took over.
                    return Err(match error {
                        Error::Refused => Error::Store("the storage is busy with the lease".into()),
                        other => other,
                    });
                }
                _ => return Err(Error::Fenced),
            }
        }
        Err(Error::Store(
            "the lease kept changing under its own writes".into(),
        ))
    }

    /// The host is alive, with these players (the host first).
    pub fn heartbeat(&mut self, players: &[Member]) -> Result<()> {
        self.update(|lease| lease.players = players.to_vec())
    }

    /// The world is open and `code` joins it.
    pub fn open(&mut self, code: &str, tunnel: bool) -> Result<()> {
        self.update(|lease| {
            lease.state = State::Hosting;
            lease.code = Some(code.to_owned());
            lease.tunnel = tunnel;
        })
    }

    /// Makes `generation` the world's newest copy. Its files must already be in the storage.
    pub fn commit(&mut self, generation: GenId) -> Result<()> {
        self.update(|lease| lease.generation = Some(generation))
    }

    /// The host leaves, having published `generation`; `successor` (a UUID) hosts next.
    ///
    /// The players stay listed through a handoff: if the successor never comes, the others take
    /// their turns in that order ([`turn`]). Nobody is playing either way ([`Lease::playing`]).
    pub fn hand_off(&mut self, generation: GenId, successor: Option<&str>) -> Result<()> {
        self.update(|lease| {
            lease.state = if successor.is_some() {
                State::Handoff
            } else {
                State::Closed
            };
            lease.generation = Some(generation);
            lease.successor = successor.map(str::to_owned);
            lease.code = None;
            lease.tunnel = false;
            if successor.is_none() {
                lease.players.clear();
            }
        })
    }

    /// Gives the world back without a new copy: it couldn't be made ready, or was never opened.
    pub fn release(&mut self) -> Result<()> {
        self.update(|lease| {
            lease.state = State::Closed;
            lease.successor = None;
            lease.code = None;
            lease.tunnel = false;
            lease.players.clear();
        })
    }
}

/// The intervals the lease's rules run on. [`Timing::REAL`] is the game's; tests run the same
/// rules faster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub heartbeat: Duration,
    pub self_fence: Duration,
    pub dead: Duration,
    pub turn: Duration,
    /// How long a handoff waits for its successor before the others may take the world.
    pub handoff_wait: Duration,
    /// How often a watcher reads the lease.
    pub poll: Duration,
}

impl Timing {
    pub const REAL: Self = Self {
        heartbeat: HEARTBEAT,
        self_fence: SELF_FENCE,
        dead: DEAD,
        turn: TURN,
        handoff_wait: Duration::from_secs(30),
        poll: Duration::from_secs(5),
    };

    /// Whether the rules hold together: a host that can't write freezes, with two heartbeats
    /// to spare, before anyone may take over, and watchers look often enough to see it.
    pub fn consistent(&self) -> bool {
        self.self_fence + self.heartbeat * 2 < self.dead
            && self.poll < self.heartbeat
            && self.handoff_wait < self.dead
    }

    /// Whether a lease quiet for `quiet` may be taken, after `base`, by the watcher whose turn
    /// is `turn`.
    pub fn after(&self, base: Duration, quiet: Duration, turn: usize) -> bool {
        let turn = u32::try_from(turn).unwrap_or(u32::MAX);
        quiet >= base.saturating_add(self.turn.saturating_mul(turn))
    }
}

/// A watcher's view of a lease it doesn't hold: how long its heartbeat has stood still, on the
/// watcher's own timer.
#[derive(Debug)]
pub struct Watch {
    mark: Option<(u64, u64, String)>,
    since: Instant,
}

impl Watch {
    pub fn new(now: Instant) -> Self {
        Self {
            mark: None,
            since: now,
        }
    }

    /// Notes the lease as read at `now`, and returns how long it has been the same write.
    pub fn observe(&mut self, lease: &Lease, now: Instant) -> Duration {
        let mark = (lease.epoch, lease.beat, lease.token.clone());
        if self.mark.as_ref() != Some(&mark) {
            self.mark = Some(mark);
            self.since = now;
        }
        now.saturating_duration_since(self.since)
    }

    /// Whether a lease quiet for `quiet` may be taken by the watcher whose place in the lease's
    /// player list (the host aside) is `turn`: the first at [`DEAD`], each next one [`TURN`] later.
    pub fn may_take(quiet: Duration, turn: usize) -> bool {
        Timing::REAL.after(DEAD, quiet, turn)
    }
}

/// A watcher's place in line to take a world whose host stopped: its position in the lease's
/// player list with the host left out. Someone not in the list goes after everyone in it.
pub fn turn(lease: &Lease, me: &str) -> usize {
    let host = lease.host.as_ref().map(|h| h.uuid.as_str());
    let waiting: Vec<&str> = lease
        .players
        .iter()
        .map(|p| p.uuid.as_str())
        .filter(|uuid| Some(*uuid) != host)
        .collect();
    waiting
        .iter()
        .position(|uuid| *uuid == me)
        .unwrap_or(waiting.len())
}

/// The host's own timer: when did a write of the lease last succeed. Give it the time the
/// write was *sent*, so the count starts no later than the storage's does.
#[derive(Debug)]
pub struct Pulse {
    last: Instant,
}

impl Pulse {
    pub fn new(sent: Instant) -> Self {
        Self { last: sent }
    }

    pub fn succeeded(&mut self, sent: Instant) {
        self.last = self.last.max(sent);
    }

    /// The host must stop changing the world until a write succeeds again.
    pub fn must_freeze(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last) >= SELF_FENCE
    }

    /// Time for the next heartbeat.
    pub fn due(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last) >= HEARTBEAT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use std::sync::atomic::Ordering;

    const KEY: &str = "w/test/lease";

    fn member(n: u32) -> Member {
        Member {
            uuid: format!("{n:032x}"),
            name: format!("Player{n}"),
            reachable: false,
        }
    }

    fn generation(epoch: u64, seq: u32) -> GenId {
        GenId {
            epoch,
            seq,
            day: 20261113,
        }
    }

    #[test]
    fn the_first_to_open_a_world_hosts_it() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        assert!(leases.read().unwrap().is_none());
        let mut held = leases
            .claim(None, &member(1), "phone", Claim::Free)
            .unwrap();
        assert_eq!(
            (held.lease().epoch, held.lease().state),
            (1, State::Starting)
        );
        // A second game that also saw no lease is refused.
        assert!(matches!(
            leases.claim(None, &member(2), "pc", Claim::Free),
            Err(Error::Refused)
        ));
        held.open("CODE-1", false).unwrap();
        held.heartbeat(&[member(1), member(2)]).unwrap();
        let seen = leases.read().unwrap().unwrap();
        assert_eq!(seen.lease.state, State::Hosting);
        assert_eq!(seen.lease.code.as_deref(), Some("CODE-1"));
        assert_eq!(seen.lease.playing(), 2);
        assert_eq!(seen.lease.beat, 2);
        // While it is hosted nobody else may claim it as free.
        assert!(matches!(
            leases.claim(Some(&seen), &member(2), "pc", Claim::Free),
            Err(Error::Refused)
        ));
    }

    #[test]
    fn a_closed_world_goes_to_one_of_two_who_open_it_together() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        let mut first = leases.claim(None, &member(1), "a", Claim::Free).unwrap();
        first.commit(generation(1, 1)).unwrap();
        first.hand_off(generation(1, 2), None).unwrap();
        let seen = leases.read().unwrap().unwrap();
        assert_eq!((seen.lease.state, seen.lease.playing()), (State::Closed, 0));
        // Both read the same closed lease.
        let second = leases
            .claim(Some(&seen), &member(2), "b", Claim::Free)
            .unwrap();
        assert!(matches!(
            leases.claim(Some(&seen), &member(3), "c", Claim::Free),
            Err(Error::Refused)
        ));
        assert_eq!(second.lease().epoch, 2);
        assert_eq!(
            second.lease().generation,
            Some(generation(1, 2)),
            "the newest copy carries over"
        );
    }

    #[test]
    fn a_handoff_is_for_its_successor() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        let mut host = leases.claim(None, &member(1), "a", Claim::Free).unwrap();
        host.open("C", false).unwrap();
        host.hand_off(generation(1, 1), Some(&member(2).uuid))
            .unwrap();
        let seen = leases.read().unwrap().unwrap();
        assert_eq!(seen.lease.state, State::Handoff);
        assert!(matches!(
            leases.claim(Some(&seen), &member(3), "c", Claim::Free),
            Err(Error::Refused)
        ));
        let next = leases
            .claim(Some(&seen), &member(2), "b", Claim::Free)
            .unwrap();
        assert_eq!(next.lease().epoch, 2);
        // The old host can't write any more.
        assert!(matches!(host.heartbeat(&[member(1)]), Err(Error::Fenced)));
    }

    #[test]
    fn a_host_that_was_replaced_is_fenced() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        let mut old = leases.claim(None, &member(1), "a", Claim::Free).unwrap();
        old.open("C", false).unwrap();
        let seen = leases.read().unwrap().unwrap();
        let mut new = leases
            .claim(Some(&seen), &member(2), "b", Claim::Takeover)
            .unwrap();
        assert!(matches!(old.commit(generation(1, 9)), Err(Error::Fenced)));
        assert!(matches!(old.heartbeat(&[member(1)]), Err(Error::Fenced)));
        new.commit(generation(2, 1)).unwrap();
        let lease = leases.read().unwrap().unwrap().lease;
        assert_eq!(
            lease.generation,
            Some(generation(2, 1)),
            "the old host's copy never counts"
        );
        assert_eq!(lease.host, Some(member(2)));
    }

    #[test]
    fn a_write_whose_answer_was_lost_still_counts() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        store.fail_after_write.store(1, Ordering::SeqCst);
        let mut held = leases.claim(None, &member(1), "a", Claim::Free).unwrap();
        assert_eq!(held.lease().epoch, 1);
        store.fail_after_write.store(1, Ordering::SeqCst);
        held.commit(generation(1, 1)).unwrap();
        held.heartbeat(&[member(1)]).unwrap();
        let lease = leases.read().unwrap().unwrap().lease;
        assert_eq!((lease.beat, lease.generation), (2, Some(generation(1, 1))));
    }

    #[test]
    fn an_unreachable_storage_is_not_a_lost_world() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        let mut held = leases.claim(None, &member(1), "a", Claim::Free).unwrap();
        // The write fails, and so does reading it back.
        store.fail_before.store(2, Ordering::SeqCst);
        assert!(matches!(held.heartbeat(&[member(1)]), Err(Error::Store(_))));
        // The write fails; the lease reads back unchanged.
        store.fail_before.store(1, Ordering::SeqCst);
        assert!(matches!(held.heartbeat(&[member(1)]), Err(Error::Store(_))));
        // And then it works again, from where it was.
        held.heartbeat(&[member(1)]).unwrap();
        assert_eq!(leases.read().unwrap().unwrap().lease.beat, 1);
    }

    #[test]
    fn a_game_picks_its_own_lease_up_after_a_restart() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        let mut held = leases
            .claim(None, &member(1), "phone", Claim::Free)
            .unwrap();
        held.open("C", true).unwrap();
        drop(held);
        let seen = leases.read().unwrap().unwrap();
        assert!(matches!(
            leases.resume(&seen, &member(1), "pc"),
            Err(Error::Refused)
        ));
        assert!(matches!(
            leases.resume(&seen, &member(2), "phone"),
            Err(Error::Refused)
        ));
        let again = leases.resume(&seen, &member(1), "phone").unwrap();
        assert_eq!(
            (again.lease().epoch, again.lease().state),
            (1, State::Starting)
        );
        assert_eq!(again.lease().code, None);
        // The lease read before the restart is stale now.
        assert!(matches!(
            leases.resume(&seen, &member(1), "phone"),
            Err(Error::Fenced)
        ));
    }

    #[test]
    fn a_lease_from_a_newer_format_is_left_alone() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        let held = leases.claim(None, &member(1), "a", Claim::Free).unwrap();
        let mut lease = held.lease().clone();
        lease.version = VERSION + 1;
        store
            .put(
                KEY,
                &serde_json::to_vec(&lease).unwrap(),
                &Condition::Always,
            )
            .unwrap();
        assert!(matches!(leases.read(), Err(Error::Format(_))));
        store.put(KEY, b"not json", &Condition::Always).unwrap();
        assert!(matches!(leases.read(), Err(Error::Format(_))));
    }

    #[test]
    fn a_heartbeat_that_stands_still_is_a_host_that_stopped() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        let mut held = leases.claim(None, &member(1), "a", Claim::Free).unwrap();
        held.heartbeat(&[member(1), member(2), member(3)]).unwrap();
        let start = Instant::now();
        let mut watch = Watch::new(start);
        let lease = leases.read().unwrap().unwrap().lease;
        assert_eq!(watch.observe(&lease, start), Duration::ZERO);
        assert_eq!(
            watch.observe(&lease, start + Duration::from_secs(60)),
            Duration::from_secs(60)
        );
        // The host writes again: the count starts over.
        held.heartbeat(&[member(1), member(2), member(3)]).unwrap();
        let lease = leases.read().unwrap().unwrap().lease;
        assert_eq!(
            watch.observe(&lease, start + Duration::from_secs(70)),
            Duration::ZERO
        );
        let quiet = watch.observe(&lease, start + Duration::from_secs(70) + DEAD);
        assert_eq!(quiet, DEAD);
        // The players take their turns, the host aside.
        assert_eq!(turn(&lease, &member(2).uuid), 0);
        assert_eq!(turn(&lease, &member(3).uuid), 1);
        assert_eq!(turn(&lease, &member(9).uuid), 2);
        assert!(Watch::may_take(quiet, 0));
        assert!(!Watch::may_take(quiet, 1));
        assert!(Watch::may_take(quiet + TURN, 1));
        assert!(!Watch::may_take(DEAD - Duration::from_millis(1), 0));
    }

    #[test]
    fn the_real_timing_holds_together() {
        assert!(Timing::REAL.consistent());
        let mut fast = Timing::REAL;
        fast.self_fence = fast.dead;
        assert!(!fast.consistent());
    }

    #[test]
    fn a_handoff_keeps_the_players_in_line_and_a_release_frees_the_world() {
        let store = Arc::new(MemoryStore::new());
        let leases = Leases::new(store.clone(), KEY);
        let mut host = leases.claim(None, &member(1), "a", Claim::Free).unwrap();
        host.heartbeat(&[member(1), member(2), member(3)]).unwrap();
        host.hand_off(generation(1, 1), Some(&member(2).uuid))
            .unwrap();
        let lease = leases.read().unwrap().unwrap().lease;
        assert_eq!(lease.playing(), 0);
        assert_eq!(turn(&lease, &member(3).uuid), 1);
        let seen = leases.read().unwrap().unwrap();
        let mut next = leases
            .claim(Some(&seen), &member(2), "b", Claim::Free)
            .unwrap();
        next.release().unwrap();
        let lease = leases.read().unwrap().unwrap().lease;
        assert_eq!(
            (lease.state, lease.playing(), lease.generation),
            (State::Closed, 0, Some(generation(1, 1)))
        );
        assert!(lease.players.is_empty());
    }

    #[test]
    fn a_host_freezes_before_anyone_may_take_over() {
        let start = Instant::now();
        let mut pulse = Pulse::new(start);
        assert!(!pulse.due(start + Duration::from_secs(14)));
        assert!(pulse.due(start + HEARTBEAT));
        assert!(!pulse.must_freeze(start + Duration::from_secs(44)));
        assert!(pulse.must_freeze(start + SELF_FENCE));
        pulse.succeeded(start + Duration::from_secs(40));
        assert!(!pulse.must_freeze(start + Duration::from_secs(80)));
        // An answer that comes back out of order doesn't move the timer backwards.
        pulse.succeeded(start + Duration::from_secs(10));
        assert!(!pulse.must_freeze(start + Duration::from_secs(80)));
    }

    /// A small deterministic generator, so a failing run can be repeated.
    struct Random(u64);
    impl Random {
        fn next(&mut self, below: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % below
        }
    }

    /// Many games on one lease, in a random order, with the storage failing at random: at no
    /// point do two of them write under one epoch, the epoch never goes back, and neither does
    /// the newest copy.
    #[test]
    fn many_games_never_share_an_epoch() {
        for seed in 1..=40u64 {
            let store = Arc::new(MemoryStore::new());
            let leases = Leases::new(store.clone(), KEY);
            let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut games: Vec<Option<Held>> = (0..5).map(|_| None).collect();
            let mut owner: std::collections::HashMap<u64, usize> = Default::default();
            let mut sequence = 0u32;
            let (mut last_epoch, mut last_generation) = (0u64, None::<GenId>);
            for _ in 0..600 {
                let who = random.next(5) as usize;
                match random.next(10) {
                    0 => store.fail_before.store(1, Ordering::SeqCst),
                    1 => store.fail_after_write.store(1, Ordering::SeqCst),
                    _ => {}
                }
                let me = member(who as u32);
                if let Some(mut held) = games[who].take() {
                    let epoch = held.lease().epoch;
                    let result = match random.next(4) {
                        0 => {
                            sequence += 1;
                            held.commit(generation(epoch, sequence))
                        }
                        1 => {
                            sequence += 1;
                            held.hand_off(generation(epoch, sequence), None)
                        }
                        _ => held.heartbeat(std::slice::from_ref(&me)),
                    };
                    match result {
                        Ok(()) => {
                            assert_eq!(*owner.entry(epoch).or_insert(who), who, "seed {seed}");
                            if held.lease().state != State::Closed {
                                games[who] = Some(held);
                            }
                        }
                        Err(Error::Fenced) => {}
                        Err(Error::Store(_)) => games[who] = Some(held),
                        Err(other) => panic!("seed {seed}: {other}"),
                    }
                } else if let Ok(seen) = leases.read() {
                    let why = if random.next(6) == 0 {
                        Claim::Takeover
                    } else {
                        Claim::Free
                    };
                    match leases.claim(seen.as_ref(), &me, &format!("device{who}"), why) {
                        Ok(held) => {
                            let epoch = held.lease().epoch;
                            assert_eq!(*owner.entry(epoch).or_insert(who), who, "seed {seed}");
                            games[who] = Some(held);
                        }
                        Err(Error::Refused | Error::Store(_)) => {}
                        Err(other) => panic!("seed {seed}: {other}"),
                    }
                }
                check(&store, &leases, &mut last_epoch, &mut last_generation, seed);
            }
        }
    }

    fn check(
        store: &MemoryStore,
        leases: &Leases,
        last_epoch: &mut u64,
        last_generation: &mut Option<GenId>,
        seed: u64,
    ) {
        // The check itself must not be failed by the switches.
        store.fail_before.store(0, Ordering::SeqCst);
        let Some(seen) = leases.read().unwrap() else {
            return;
        };
        assert!(
            seen.lease.epoch >= *last_epoch,
            "seed {seed}: the epoch went back"
        );
        assert!(
            seen.lease.generation >= *last_generation,
            "seed {seed}: the newest copy went back"
        );
        *last_epoch = seen.lease.epoch;
        *last_generation = seen.lease.generation;
    }
}
