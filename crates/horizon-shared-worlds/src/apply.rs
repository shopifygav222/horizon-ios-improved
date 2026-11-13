//! Bringing this device's copy of a world to a generation, without ever leaving the world folder
//! half one copy and half another.
//!
//! 1. Only blocks that differ are downloaded, each checked against its hash as it arrives.
//! 2. Every file that changes is rebuilt in the working folder, beside the world's files, and its
//!    whole SHA-256 compared with the generation's file list. Until all of them pass, the world
//!    folder is untouched.
//! 3. A journal of the renames is written, and only then are they made. A game that dies between
//!    two renames finds the journal at its next start and finishes them ([`recover`]).
//! 4. Each file that is replaced or removed is moved to `previous/`, not deleted.
use crate::{
    Error, Result,
    manifest::{BLOCK, FileEntry, Manifest, WORK, block_len, safe_path},
    scan::{LocalFile, LocalIndex, hash_file, rescan, under, write_atomic},
    sync::{Progress, Remote, read_run},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// The most read from the storage in one request.
const FETCH: u64 = 8 << 20;
/// How many sets of replaced files are kept.
const KEEP_PREVIOUS: usize = 2;

/// Where things are in a world folder.
#[derive(Clone, Debug)]
pub struct Layout {
    pub world: PathBuf,
}

impl Layout {
    pub fn new(world: impl Into<PathBuf>) -> Self {
        Self {
            world: world.into(),
        }
    }
    fn work(&self) -> PathBuf {
        self.world.join(WORK)
    }
    /// What marks the folder as a shared world and says which.
    pub fn marker(&self) -> PathBuf {
        self.work().join("world.json")
    }
    pub fn index(&self) -> PathBuf {
        self.work().join("index.json")
    }
    /// The manifest of the generation this copy was last brought to or published as.
    pub fn base(&self) -> PathBuf {
        self.work().join("base.json")
    }
    /// Copies of files changed since the last committed generation.
    pub fn staging(&self) -> PathBuf {
        self.work().join("staging")
    }
    fn fresh(&self) -> PathBuf {
        self.work().join("new")
    }
    fn journal(&self) -> PathBuf {
        self.work().join("journal.json")
    }
    /// Files a later generation replaced, a folder for each.
    pub fn previous(&self) -> PathBuf {
        self.work().join("previous")
    }

    /// The saved manifest of this copy's generation, if there is one and it reads.
    pub fn load_base(&self) -> Option<Manifest> {
        Manifest::from_bytes(&fs::read(self.base()).ok()?).ok()
    }

    pub fn save_base(&self, manifest: &Manifest) -> Result<()> {
        Ok(write_atomic(&self.base(), &manifest.to_bytes()?)?)
    }
}

#[derive(Serialize, Deserialize)]
struct Replace {
    path: String,
    /// The rebuilt file's name in the working folder.
    temp: String,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    /// Names the folder under `previous/` the replaced files go to.
    generation: String,
    replace: Vec<Replace>,
    remove: Vec<String>,
}

/// What a fetch did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    pub downloaded: u64,
    pub replaced: usize,
    pub removed: usize,
}

/// The blocks of `file` this device doesn't already have at the same place in its own file.
fn missing(file: &FileEntry, local: Option<&LocalFile>) -> Vec<u64> {
    (0..file.blocks.len())
        .filter(|i| local.and_then(|l| l.blocks.get(*i)) != Some(&file.blocks[*i]))
        .map(|i| i as u64)
        .collect()
}

fn same(file: &FileEntry, local: Option<&LocalFile>) -> bool {
    local.is_some_and(|l| l.size == file.size && l.sha256 == file.sha256)
}

/// How much bringing `index` to `target` would download: (bytes, files that change).
pub fn download_size(target: &Manifest, index: &LocalIndex) -> (u64, usize) {
    let mut bytes = 0;
    let mut files = 0;
    for file in &target.files {
        let local = index.files.get(&file.path);
        if !same(file, local) {
            files += 1;
            bytes += missing(file, local)
                .iter()
                .map(|i| block_len(file.size, *i))
                .sum::<u64>();
        }
    }
    (bytes, files)
}

fn ignore_missing(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Builds `file` at `temp`: this device's own file where it has the same blocks, the storage's
/// bytes for `blocks`.
fn rebuild(
    remote: Remote<'_>,
    target: &Manifest,
    file: &FileEntry,
    live: Option<&Path>,
    blocks: &[u64],
    temp: &Path,
    count: &mut dyn FnMut(u64) -> bool,
) -> Result<()> {
    ignore_missing(fs::remove_file(temp))?;
    if let Some(live) = live {
        fs::copy(live, temp)?;
    }
    let mut out = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(temp)?;
    out.set_len(file.size)?;
    let mut at = 0;
    while at < blocks.len() {
        // As many of the next blocks as follow each other in the file and in one pack.
        let first = blocks[at];
        let start = file.locate(first);
        let mut n = 1u64;
        while at + (n as usize) < blocks.len()
            && blocks[at + n as usize] == first + n
            && n * BLOCK < FETCH
            && file.locate(first + n) == start.map(|(pack, offset)| (pack, offset + n * BLOCK))
        {
            n += 1;
        }
        let data = read_run(remote, target, file, first, n)?;
        out.seek(SeekFrom::Start(first * BLOCK))?;
        out.write_all(&data)?;
        if !count(data.len() as u64) {
            return Err(Error::Cancelled);
        }
        at += n as usize;
    }
    out.sync_all()?;
    Ok(())
}

fn whole(file: &FileEntry, temp: &Path) -> Result<bool> {
    let (size, sha256, _) = hash_file(temp)?;
    Ok(size == file.size && sha256 == file.sha256)
}

/// Brings the world folder to `target`. The world must be closed. `index` is brought up to the
/// folder first, so files changed on this device since the last scan are seen for what they are.
pub fn fetch(
    remote: Remote<'_>,
    target: &Manifest,
    layout: &Layout,
    index: &mut LocalIndex,
    progress: Progress<'_>,
) -> Result<Fetched> {
    target.check()?;
    recover(layout)?;
    fs::create_dir_all(&layout.world)?;
    rescan(&layout.world, index)?;
    let (total, _) = download_size(target, index);
    let mut done = 0u64;
    let mut count = |bytes: u64| {
        done += bytes;
        progress(done.min(total), total)
    };
    let fresh = layout.fresh();
    ignore_missing(fs::remove_dir_all(&fresh))?;
    fs::create_dir_all(&fresh)?;
    let mut journal = Journal {
        generation: target
            .id
            .manifest_key()
            .trim_start_matches("gen/")
            .trim_end_matches(".json")
            .to_owned(),
        replace: Vec::new(),
        remove: Vec::new(),
    };
    for file in &target.files {
        let local = index.files.get(&file.path);
        if same(file, local) {
            continue;
        }
        let name = format!("{:06}", journal.replace.len());
        let temp = fresh.join(&name);
        let live = under(&layout.world, &file.path);
        let live = local.is_some().then_some(live.as_path());
        rebuild(
            remote,
            target,
            file,
            live,
            &missing(file, local),
            &temp,
            &mut count,
        )?;
        if !whole(file, &temp)? {
            // The blocks this device seemed to share were not the same after all: take all of
            // them from the storage.
            let all: Vec<u64> = (0..file.blocks.len() as u64).collect();
            rebuild(remote, target, file, None, &all, &temp, &mut |_| true)?;
            if !whole(file, &temp)? {
                return Err(Error::Corrupt(format!(
                    "{} in the storage is not the file its copy lists",
                    file.path
                )));
            }
        }
        journal.replace.push(Replace {
            path: file.path.clone(),
            temp: name,
        });
    }
    journal.remove = index
        .files
        .keys()
        .filter(|path| target.file(path).is_none())
        .cloned()
        .collect();
    let fetched = Fetched {
        downloaded: done,
        replaced: journal.replace.len(),
        removed: journal.remove.len(),
    };
    // From here the copy is decided: the journal is the point of no return.
    write_atomic(&layout.journal(), &serde_json::to_vec(&journal)?)?;
    finish(layout, &journal, &mut None)?;
    for replaced in &journal.replace {
        let Some(file) = target.file(&replaced.path) else {
            continue;
        };
        let metadata = fs::metadata(under(&layout.world, &file.path))?;
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        index.files.insert(
            file.path.clone(),
            LocalFile {
                size: file.size,
                modified,
                sha256: file.sha256.clone(),
                blocks: file.blocks.clone(),
            },
        );
    }
    for path in &journal.remove {
        index.files.remove(path);
    }
    index.save(&layout.index())?;
    layout.save_base(target)?;
    prune_previous(layout);
    Ok(fetched)
}

/// One step of [`finish`] against a test's budget of steps: the game "dies" when it runs out.
fn step(budget: &mut Option<usize>) -> Result<()> {
    match budget {
        Some(0) => Err(Error::Cancelled),
        Some(left) => {
            *left -= 1;
            Ok(())
        }
        None => Ok(()),
    }
}

fn move_aside(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    ignore_missing(fs::remove_file(to))?;
    fs::rename(from, to)
}

/// Makes the journal's renames. Each is safe to make again, so this can be cut off anywhere and
/// run once more from the top.
fn finish(layout: &Layout, journal: &Journal, budget: &mut Option<usize>) -> Result<()> {
    let fresh = layout.fresh();
    let keep = layout.previous().join(&journal.generation);
    for replace in &journal.replace {
        let temp = fresh.join(&replace.temp);
        if !temp.exists() {
            // Already in place.
            continue;
        }
        let live = under(&layout.world, &replace.path);
        if live.exists() {
            step(budget)?;
            move_aside(&live, &under(&keep, &replace.path))?;
        }
        if let Some(parent) = live.parent() {
            fs::create_dir_all(parent)?;
        }
        step(budget)?;
        fs::rename(&temp, &live)?;
    }
    for path in &journal.remove {
        let live = under(&layout.world, path);
        if live.exists() {
            step(budget)?;
            move_aside(&live, &under(&keep, path))?;
        }
    }
    step(budget)?;
    ignore_missing(fs::remove_file(layout.journal()))?;
    ignore_missing(fs::remove_dir_all(&fresh))?;
    Ok(())
}

/// Finishes a fetch a dead game left half-made, or clears what it left before the point of no
/// return. Returns whether there was a journal to finish. Call it before a shared world is
/// opened or scanned.
pub fn recover(layout: &Layout) -> Result<bool> {
    let read = match fs::read(layout.journal()) {
        Ok(data) => data,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            ignore_missing(fs::remove_dir_all(layout.fresh()))?;
            return Ok(false);
        }
        Err(e) => return Err(e.into()),
    };
    let journal: Journal = serde_json::from_slice(&read)?;
    let named_well = journal.replace.iter().all(|r| {
        safe_path(&r.path) && !r.temp.is_empty() && r.temp.bytes().all(|b| b.is_ascii_digit())
    }) && journal.remove.iter().all(|p| safe_path(p))
        && safe_path(&journal.generation)
        && !journal.generation.contains('/');
    if !named_well {
        return Err(Error::Corrupt(
            "the journal of an unfinished update names files outside the world".into(),
        ));
    }
    finish(layout, &journal, &mut None)?;
    Ok(true)
}

/// Drops all but the newest sets of replaced files. Best effort: they are only a convenience.
fn prune_previous(layout: &Layout) {
    let Ok(entries) = fs::read_dir(layout.previous()) else {
        return;
    };
    let mut sets: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    sets.sort();
    let extra = sets.len().saturating_sub(KEEP_PREVIOUS);
    for set in &sets[..extra] {
        let _ = fs::remove_dir_all(set);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        scan::{
            tests::{Scratch, bytes},
            walk,
        },
        store::{Condition, MemoryStore, Store},
        sync::tests::{PREFIX, go, id, publish_world},
    };
    use std::collections::BTreeMap;

    /// Every shared file of a folder with its bytes.
    fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
        walk(root)
            .unwrap()
            .into_iter()
            .map(|f| {
                let data = fs::read(under(root, &f.path)).unwrap();
                (f.path, data)
            })
            .collect()
    }

    fn fetch_into(store: &MemoryStore, target: &Manifest, world: &Scratch) -> Result<Fetched> {
        let layout = Layout::new(&world.0);
        let mut index = LocalIndex::load(&layout.index());
        fetch(
            Remote {
                store,
                prefix: PREFIX,
            },
            target,
            &layout,
            &mut index,
            &mut go,
        )
    }

    /// A host's world with two generations published: (store, host world, first, second).
    fn two_generations() -> (MemoryStore, Scratch, Manifest, Manifest) {
        let store = MemoryStore::new();
        let host = Scratch::new("host");
        let mut region = bytes(11, BLOCK as usize * 6 + 300);
        host.write("level.dat", b"first");
        host.write("dimensions/minecraft/overworld/region/r.0.0.mca", &region);
        host.write(
            "dimensions/minecraft/overworld/region/r.0.1.mca",
            &bytes(12, 9000),
        );
        host.write("players/data/aa.dat", b"alex");
        let mut index = LocalIndex::default();
        let first = publish_world(&store, &host, &mut index, None, id(1, 1));
        region[BLOCK as usize * 2 + 10] ^= 1;
        region.truncate(BLOCK as usize * 5 + 50);
        host.write("dimensions/minecraft/overworld/region/r.0.0.mca", &region);
        host.write("level.dat", b"second, longer");
        host.write("players/data/bb.dat", b"steve");
        fs::remove_file(host.path("dimensions/minecraft/overworld/region/r.0.1.mca")).unwrap();
        index.scanned = 0;
        let second = publish_world(&store, &host, &mut index, Some(&first), id(1, 2));
        (store, host, first, second)
    }

    #[test]
    fn a_new_device_gets_the_whole_world_and_then_only_what_changed() {
        let (store, host, first, second) = two_generations();
        let guest = Scratch::new("guest");
        let fetched = fetch_into(&store, &first, &guest).unwrap();
        assert_eq!(fetched.replaced, 4);
        assert_eq!(fetched.downloaded, first.size());
        assert_eq!(Layout::new(&guest.0).load_base(), Some(first.clone()));
        // The same generation again: nothing to do.
        assert_eq!(
            fetch_into(&store, &first, &guest).unwrap(),
            Fetched::default()
        );
        let calls = store.calls.load(std::sync::atomic::Ordering::SeqCst);
        let fetched = fetch_into(&store, &second, &guest).unwrap();
        assert_eq!(tree(&guest.0), tree(&host.0));
        assert_eq!((fetched.replaced, fetched.removed), (3, 1));
        // The region changed in one block and lost its tail; the level and a new player file.
        assert_eq!(fetched.downloaded, BLOCK + 50 + 14 + 5);
        assert!(
            store.calls.load(std::sync::atomic::Ordering::SeqCst) - calls <= 4,
            "one request a run"
        );
        // What was replaced is kept.
        let kept = Layout::new(&guest.0)
            .previous()
            .join("0000000001-000002-20261113");
        assert_eq!(fs::read(kept.join("level.dat")).unwrap(), b"first");
        assert!(
            kept.join("dimensions/minecraft/overworld/region/r.0.1.mca")
                .is_file()
        );
        assert_eq!(Layout::new(&guest.0).load_base(), Some(second));
    }

    #[test]
    fn a_device_can_go_back_to_an_older_copy() {
        let (store, _host, first, second) = two_generations();
        let guest = Scratch::new("back");
        fetch_into(&store, &second, &guest).unwrap();
        let newer = tree(&guest.0);
        fetch_into(&store, &first, &guest).unwrap();
        let fresh = Scratch::new("fresh");
        fetch_into(&store, &first, &fresh).unwrap();
        assert_eq!(tree(&guest.0), tree(&fresh.0));
        assert_ne!(tree(&guest.0), newer);
    }

    #[test]
    fn files_changed_on_the_device_are_put_right() {
        let (store, host, _first, second) = two_generations();
        let guest = Scratch::new("drift");
        fetch_into(&store, &second, &guest).unwrap();
        // Played alone, a stray file, a deleted one.
        guest.write("level.dat", b"edited here");
        guest.write("stray.txt", b"x");
        fs::remove_file(guest.path("players/data/aa.dat")).unwrap();
        let fetched = fetch_into(&store, &second, &guest).unwrap();
        assert_eq!(tree(&guest.0), tree(&host.0));
        assert_eq!((fetched.replaced, fetched.removed), (2, 1));
    }

    #[test]
    fn damaged_storage_leaves_the_world_as_it_was() {
        let (store, _host, first, second) = two_generations();
        let guest = Scratch::new("damage");
        fetch_into(&store, &first, &guest).unwrap();
        let before = tree(&guest.0);
        let pack = format!("{PREFIX}{}", second.packs.last().unwrap());
        let mut data = store.get(&pack).unwrap().unwrap().data;
        data[0] ^= 0x55;
        store.put(&pack, &data, &Condition::Always).unwrap();
        assert!(matches!(
            fetch_into(&store, &second, &guest),
            Err(Error::Corrupt(_))
        ));
        assert_eq!(tree(&guest.0), before);
        // And an unreachable storage, or a cancel: the same.
        store
            .fail_before
            .store(1, std::sync::atomic::Ordering::SeqCst);
        assert!(fetch_into(&store, &second, &guest).is_err());
        let layout = Layout::new(&guest.0);
        let mut index = LocalIndex::load(&layout.index());
        let remote = Remote {
            store: &store,
            prefix: PREFIX,
        };
        let cancelled = fetch(remote, &second, &layout, &mut index, &mut |_, _| false);
        assert!(cancelled.is_err());
        assert_eq!(tree(&guest.0), before);
    }

    /// The update is cut off after every possible number of renames; each time, the next start
    /// finishes it and the world is the new copy, whole.
    #[test]
    fn an_update_cut_off_at_any_rename_is_finished_at_the_next_start() {
        let (store, host, first, second) = two_generations();
        let wanted = tree(&host.0);
        let mut finished_whole = false;
        for budget in 0..40 {
            let guest = Scratch::new("cut");
            fetch_into(&store, &first, &guest).unwrap();
            let layout = Layout::new(&guest.0);
            let mut index = LocalIndex::load(&layout.index());
            // The fetch up to its point of no return, as `fetch` makes it.
            rescan(&layout.world, &mut index).unwrap();
            let fresh = layout.fresh();
            fs::create_dir_all(&fresh).unwrap();
            let remote = Remote {
                store: &store,
                prefix: PREFIX,
            };
            let mut journal = Journal {
                generation: "cut".into(),
                replace: Vec::new(),
                remove: index
                    .files
                    .keys()
                    .filter(|p| second.file(p).is_none())
                    .cloned()
                    .collect(),
            };
            for file in &second.files {
                let local = index.files.get(&file.path);
                if same(file, local) {
                    continue;
                }
                let name = format!("{:06}", journal.replace.len());
                let live = under(&layout.world, &file.path);
                let live = local.is_some().then_some(live.as_path());
                rebuild(
                    remote,
                    &second,
                    file,
                    live,
                    &missing(file, local),
                    &fresh.join(&name),
                    &mut |_| true,
                )
                .unwrap();
                journal.replace.push(Replace {
                    path: file.path.clone(),
                    temp: name,
                });
            }
            write_atomic(&layout.journal(), &serde_json::to_vec(&journal).unwrap()).unwrap();
            let cut = finish(&layout, &journal, &mut Some(budget));
            if cut.is_ok() {
                finished_whole = true;
            } else {
                assert!(matches!(cut, Err(Error::Cancelled)));
                assert!(
                    recover(&layout).unwrap(),
                    "budget {budget}: the journal is found"
                );
            }
            assert_eq!(tree(&guest.0), wanted, "budget {budget}");
            assert!(
                !recover(&layout).unwrap(),
                "budget {budget}: nothing left to finish"
            );
            assert!(!layout.fresh().exists());
            if finished_whole {
                break;
            }
        }
        assert!(
            finished_whole,
            "the budget grew until the update ran through"
        );
    }

    #[test]
    fn leftovers_from_before_the_point_of_no_return_are_cleared() {
        let (store, _host, first, _second) = two_generations();
        let guest = Scratch::new("left");
        fetch_into(&store, &first, &guest).unwrap();
        let before = tree(&guest.0);
        let layout = Layout::new(&guest.0);
        fs::create_dir_all(layout.fresh()).unwrap();
        fs::write(layout.fresh().join("000000"), b"").unwrap();
        assert!(!recover(&layout).unwrap());
        assert!(!layout.fresh().exists());
        assert_eq!(tree(&guest.0), before);
        // A journal that points outside the world is refused, not followed.
        fs::write(
            layout.journal(),
            br#"{"generation":"g","replace":[{"path":"../../outside","temp":"000000"}],"remove":[]}"#,
        )
        .unwrap();
        assert!(matches!(recover(&layout), Err(Error::Corrupt(_))));
    }

    #[test]
    fn only_the_newest_replaced_files_are_kept() {
        let store = MemoryStore::new();
        let host = Scratch::new("many");
        let guest = Scratch::new("manyguest");
        let mut index = LocalIndex::default();
        let mut previous: Option<Manifest> = None;
        for seq in 1..=5 {
            host.write("level.dat", format!("copy {seq}").as_bytes());
            index.scanned = 0;
            let made = publish_world(&store, &host, &mut index, previous.as_ref(), id(1, seq));
            fetch_into(&store, &made, &guest).unwrap();
            previous = Some(made);
        }
        let mut sets: Vec<String> = fs::read_dir(Layout::new(&guest.0).previous())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        sets.sort();
        assert_eq!(
            sets,
            ["0000000001-000004-20261113", "0000000001-000005-20261113"]
        );
    }
}
