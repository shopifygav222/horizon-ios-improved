//! Which older copies the storage keeps, and the removal of the rest.
//!
//! Kept: the newest [`NEWEST`] generations, and the last one of each of the newest [`DAYS`]
//! days, so a world can be taken back to how it was earlier today or last week. A pack is
//! removed only when no kept generation points into it and it is older than the world's
//! current generation, so one that is being uploaded right now is never touched. If any kept
//! manifest can't be read, nothing is removed at all.
use crate::{Error, Result, manifest::GenId, sync::Remote};
use std::collections::BTreeSet;

pub const NEWEST: usize = 20;
pub const DAYS: usize = 14;
/// A sweep is worth its requests once this many generations are stored.
pub const SWEEP_AT: usize = NEWEST + DAYS + 14;

/// The generations to keep of `all`, the world's newest copy being `current`.
pub fn kept(all: &[GenId], current: GenId) -> BTreeSet<GenId> {
    let mut older: Vec<GenId> = all.iter().copied().filter(|id| *id <= current).collect();
    older.sort_unstable_by(|a, b| b.cmp(a));
    older.dedup();
    let mut keep: BTreeSet<GenId> = older.iter().copied().take(NEWEST).collect();
    let mut days: Vec<u32> = Vec::new();
    for id in &older {
        if !days.contains(&id.day) {
            if days.len() == DAYS {
                break;
            }
            // The newest of its day: `older` runs from the newest back.
            days.push(id.day);
            keep.insert(*id);
        }
    }
    keep.insert(current);
    keep
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub manifests: usize,
    pub packs: usize,
    pub bytes: u64,
}

/// The generations whose manifests the storage lists.
pub fn generations(remote: Remote<'_>) -> Result<Vec<GenId>> {
    let prefix = remote.key("");
    Ok(remote
        .store
        .list(&remote.key("gen/"))?
        .iter()
        .filter_map(|o| GenId::from_manifest_key(o.key.strip_prefix(&prefix)?))
        .collect())
}

/// Removes the copies no longer kept. Only the world's host calls this, with the generation its
/// lease names.
pub fn sweep(remote: Remote<'_>, current: GenId) -> Result<Swept> {
    let all = generations(remote)?;
    if all.len() < SWEEP_AT {
        return Ok(Swept::default());
    }
    if !all.contains(&current) {
        return Err(Error::Store(
            "the storage doesn't list the world's newest copy: nothing is removed".into(),
        ));
    }
    let keep = kept(&all, current);
    // Every pack a kept generation points into. A manifest that can't be read ends the sweep
    // here, before anything is removed.
    let mut live: BTreeSet<String> = BTreeSet::new();
    for id in &keep {
        live.extend(remote.manifest(*id)?.packs);
    }
    let mut swept = Swept::default();
    for id in all.iter().filter(|id| **id < current && !keep.contains(id)) {
        remote.store.delete(&remote.key(&id.manifest_key()))?;
        swept.manifests += 1;
    }
    let prefix = remote.key("");
    for object in remote.store.list(&remote.key("pack/"))? {
        let Some(name) = object.key.strip_prefix(&prefix) else {
            continue;
        };
        let older = GenId::of_pack(name).is_some_and(|made| made < (current.epoch, current.seq));
        if older && !live.contains(name) {
            remote.store.delete(&object.key)?;
            swept.packs += 1;
            swept.bytes += object.size;
        }
    }
    Ok(swept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        scan::{LocalIndex, tests::Scratch},
        store::{MemoryStore, Store},
        sync::tests::{PREFIX, publish_world},
    };

    fn id(seq: u32, day: u32) -> GenId {
        GenId { epoch: 1, seq, day }
    }

    #[test]
    fn the_newest_and_one_a_day_are_kept() {
        // Three copies a day for 30 days.
        let all: Vec<GenId> = (0..90).map(|i| id(i + 1, 20261001 + i / 3)).collect();
        let current = all[89];
        let keep = kept(&all, current);
        // The newest 20 are seq 71..=90; they cover days 24..=30 (7 days). 7 more days back
        // keep their last copy each.
        let extra: Vec<u32> = keep.iter().filter(|g| g.seq <= 70).map(|g| g.seq).collect();
        assert_eq!(extra, [51, 54, 57, 60, 63, 66, 69]);
        assert_eq!(keep.len(), 27);
        assert!(keep.contains(&current));
        // A copy newer than the current one (an upload under way, a fenced host's) is no keeper.
        let mut with_newer = all.clone();
        with_newer.push(id(91, 20261030));
        assert!(!kept(&with_newer, current).contains(&id(91, 20261030)));
        assert_eq!(kept(&[], current).len(), 1);
    }

    #[test]
    fn a_sweep_removes_what_nothing_kept_points_into() {
        let store = MemoryStore::new();
        let remote = Remote {
            store: &store,
            prefix: PREFIX,
        };
        let world = Scratch::new("sweep");
        let mut index = LocalIndex::default();
        let mut previous = None;
        let total = SWEEP_AT as u32 + 12;
        // A file rewritten in every copy, and one that never changes.
        world.write("stays.dat", b"always here");
        for seq in 1..=total {
            world.write("level.dat", format!("copy {seq}").as_bytes());
            index.scanned = 0;
            let day = 20261001 + (seq - 1) / 30;
            let gen_id = GenId { epoch: 1, seq, day };
            previous = Some(publish_world(
                &store,
                &world,
                &mut index,
                previous.as_ref(),
                gen_id,
            ));
        }
        let newest = previous.unwrap();
        // An upload under way: a pack newer than the current copy, in no manifest yet.
        let in_flight = remote.key(
            &GenId {
                epoch: 1,
                seq: total + 1,
                day: 0,
            }
            .pack_name(0),
        );
        store
            .put(
                &in_flight,
                b"being uploaded",
                &crate::store::Condition::Always,
            )
            .unwrap();
        let before = generations(remote).unwrap().len();
        assert_eq!(before, total as usize);
        let swept = sweep(remote, newest.id).unwrap();
        let after = generations(remote).unwrap();
        let keep = kept(
            &(1..=total)
                .map(|s| GenId {
                    epoch: 1,
                    seq: s,
                    day: 20261001 + (s - 1) / 30,
                })
                .collect::<Vec<_>>(),
            newest.id,
        );
        assert_eq!(after.iter().copied().collect::<BTreeSet<_>>(), keep);
        assert_eq!(swept.manifests, before - keep.len());
        assert!(swept.packs > 0 && swept.bytes > 0);
        // Every kept copy still reads whole, the first pack (the file that never changed) with it.
        for kept_id in &keep {
            let manifest = remote.manifest(*kept_id).unwrap();
            for file in &manifest.files {
                for i in 0..file.blocks.len() as u64 {
                    crate::sync::read_run(remote, &manifest, file, i, 1).unwrap();
                }
            }
        }
        assert!(
            store.get(&in_flight).unwrap().is_some(),
            "an upload under way is left alone"
        );
        // Nothing more to remove.
        assert_eq!(sweep(remote, newest.id).unwrap(), Swept::default());
    }

    #[test]
    fn a_manifest_that_cant_be_read_stops_the_sweep_before_it_removes_anything() {
        let store = MemoryStore::new();
        let remote = Remote {
            store: &store,
            prefix: PREFIX,
        };
        let world = Scratch::new("sweepstop");
        let mut index = LocalIndex::default();
        let mut previous = None;
        let total = SWEEP_AT as u32 + 3;
        for seq in 1..=total {
            world.write("level.dat", format!("copy {seq}").as_bytes());
            index.scanned = 0;
            let gen_id = GenId {
                epoch: 1,
                seq,
                day: 20261001,
            };
            previous = Some(publish_world(
                &store,
                &world,
                &mut index,
                previous.as_ref(),
                gen_id,
            ));
        }
        let newest = previous.unwrap();
        let keys = store.keys();
        let damaged = remote.key(
            &GenId {
                epoch: 1,
                seq: total - 1,
                day: 20261001,
            }
            .manifest_key(),
        );
        store
            .put(&damaged, b"{", &crate::store::Condition::Always)
            .unwrap();
        assert!(sweep(remote, newest.id).is_err());
        assert_eq!(store.keys(), keys);
        // And a current copy the storage doesn't list.
        let missing = GenId {
            epoch: 9,
            seq: 1,
            day: 20261001,
        };
        assert!(sweep(remote, missing).is_err());
        assert_eq!(store.keys(), keys);
    }
}
