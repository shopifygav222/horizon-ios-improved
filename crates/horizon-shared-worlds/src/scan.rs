//! What this device has of a world: every file's size, time of change, SHA-256 and block hashes
//! (the index), kept between runs so that only files that changed are read again.
//!
//! A host's checkpoint uses it in two steps. While the world is held still, [`changes`] finds
//! the files that differ from the index and [`stage`] copies them aside; the world then runs on,
//! and [`absorb`] hashes the copies. A staged file is kept until a generation that holds it is
//! committed, so a failed upload is simply made again from the same bytes.
use crate::{
    Error, Result,
    codec::hex,
    manifest::{BLOCK, BlockHash, block_hash, reserved, safe_path, short_hashes},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// A file changed this close before a scan (or after it) is read again next time whatever its
/// size and time say: a file system's clock may be this coarse.
const RACY: u64 = 2_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalFile {
    pub size: u64,
    /// When it was last changed, in nanoseconds since 1970.
    pub modified: u64,
    pub sha256: String,
    #[serde(with = "short_hashes")]
    pub blocks: Vec<BlockHash>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalIndex {
    /// When the scan that made it began, in nanoseconds since 1970.
    pub scanned: u64,
    pub files: BTreeMap<String, LocalFile>,
}

/// A file as a walk of the folder found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: String,
    pub size: u64,
    pub modified: u64,
}

/// What differs from the index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub changed: Vec<Found>,
    pub removed: Vec<String>,
    /// When the scan began.
    pub at: u64,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }
}

fn nanos(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

/// `path` (as a manifest writes it) under `root`.
pub fn under(root: &Path, path: &str) -> PathBuf {
    path.split('/')
        .fold(root.to_path_buf(), |at, part| at.join(part))
}

/// Writes `data` beside `path` and renames it into place, so `path` is always whole.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".new");
    let temporary = path.with_file_name(name);
    let mut file = File::create(&temporary)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)
}

impl LocalIndex {
    /// The saved index, or an empty one when there is none or it can't be read: everything is
    /// then hashed again, which costs time and nothing else.
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        Ok(write_atomic(path, &serde_json::to_vec(self)?)?)
    }
}

/// Every file of the world folder that belongs to a copy, in path order. Links are not followed,
/// and a name a manifest couldn't carry is left out (and logged).
pub fn walk(root: &Path) -> io::Result<Vec<Found>> {
    fn visit(dir: &Path, prefix: &str, found: &mut Vec<Found>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                log::warn!(
                    "Shared worlds: {:?} has a name that can't be shared",
                    entry.path()
                );
                continue;
            };
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if reserved(&path) || kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                visit(&entry.path(), &path, found)?;
            } else if !safe_path(&path) {
                log::warn!("Shared worlds: {path:?} has a name that can't be shared");
            } else {
                let metadata = entry.metadata()?;
                found.push(Found {
                    path,
                    size: metadata.len(),
                    modified: nanos(metadata.modified()?),
                });
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    visit(root, "", &mut found)?;
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// A file's length, SHA-256 and block hashes, read once.
pub fn hash_file(path: &Path) -> io::Result<(u64, String, Vec<BlockHash>)> {
    let mut file = File::open(path)?;
    let mut whole = Sha256::new();
    let mut blocks = Vec::new();
    let mut buffer = vec![0u8; BLOCK as usize];
    let mut size = 0u64;
    loop {
        // A whole block, or what is left of the file.
        let mut filled = 0;
        while filled < buffer.len() {
            let n = file.read(&mut buffer[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        whole.update(&buffer[..filled]);
        blocks.push(block_hash(&buffer[..filled]));
        size += filled as u64;
        if filled < buffer.len() {
            break;
        }
    }
    Ok((size, hex(&whole.finalize()), blocks))
}

/// The files of `root` that differ from `index`, by size and time of change alone: nothing is
/// read, so this is quick enough to run while the world is held still.
pub fn changes(root: &Path, index: &LocalIndex) -> io::Result<Changes> {
    let at = nanos(SystemTime::now());
    let found = walk(root)?;
    let mut removed: Vec<String> = index
        .files
        .keys()
        .filter(|path| {
            found
                .binary_search_by(|f| f.path.as_str().cmp(path.as_str()))
                .is_err()
        })
        .cloned()
        .collect();
    removed.sort();
    let changed = found
        .into_iter()
        .filter(|f| match index.files.get(&f.path) {
            Some(known) => {
                known.size != f.size
                    || known.modified != f.modified
                    || f.modified.saturating_add(RACY) >= index.scanned
            }
            None => true,
        })
        .collect();
    Ok(Changes {
        changed,
        removed,
        at,
    })
}

/// Copies the changed files from `root` into `staging`, and forgets staged copies of files that
/// are gone. On Apple's file system a copy is a clone: no bytes move.
pub fn stage(root: &Path, staging: &Path, changes: &Changes) -> io::Result<()> {
    for file in &changes.changed {
        let to = under(staging, &file.path);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        // A clone can't replace a file, so the old copy goes first.
        match fs::remove_file(&to) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        fs::copy(under(root, &file.path), &to)?;
    }
    for path in &changes.removed {
        match fs::remove_file(under(staging, path)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Brings `index` up to `changes`: hashes each changed file where `source` says its bytes are
/// (the staged copy, or the world folder itself when the world is closed). A file whose length
/// isn't what the scan saw was written to after it: [`Error::Corrupt`], and the index is left
/// as it was.
pub fn absorb(
    index: &mut LocalIndex,
    changes: &Changes,
    source: &dyn Fn(&str) -> PathBuf,
) -> Result<()> {
    let mut hashed = Vec::with_capacity(changes.changed.len());
    for file in &changes.changed {
        let (size, sha256, blocks) = hash_file(&source(&file.path))?;
        if size != file.size {
            return Err(Error::Corrupt(format!(
                "{} changed while it was copied ({size} bytes, {} at the scan)",
                file.path, file.size
            )));
        }
        hashed.push((
            file.path.clone(),
            LocalFile {
                size,
                modified: file.modified,
                sha256,
                blocks,
            },
        ));
    }
    index.files.extend(hashed);
    for path in &changes.removed {
        index.files.remove(path);
    }
    index.scanned = changes.at;
    Ok(())
}

/// Brings `index` up to a folder nothing is writing to (a closed world).
pub fn rescan(root: &Path, index: &mut LocalIndex) -> Result<Changes> {
    let found = changes(root, index)?;
    absorb(index, &found, &|path| under(root, path))?;
    Ok(found)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::codec::{random, sha256};

    /// A scratch folder removed when the test ends.
    pub(crate) struct Scratch(pub PathBuf);
    impl Scratch {
        pub(crate) fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("hsw-{name}-{}", hex(&random::<8>().unwrap())));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        pub(crate) fn path(&self, path: &str) -> PathBuf {
            under(&self.0, path)
        }
        pub(crate) fn write(&self, path: &str, data: &[u8]) {
            let to = self.path(path);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::write(to, data).unwrap();
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Repeatable bytes that don't compress to a pattern.
    pub(crate) fn bytes(seed: u64, length: usize) -> Vec<u8> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect()
    }

    /// An index whose scan lies in the past, as one loaded from disk does.
    fn settled(mut index: LocalIndex) -> LocalIndex {
        index.scanned = index.scanned.saturating_add(10 * RACY);
        index
    }

    #[test]
    fn a_file_is_hashed_block_by_block() {
        let scratch = Scratch::new("hash");
        let data = bytes(1, BLOCK as usize * 2 + 77);
        scratch.write("f", &data);
        let (size, sha, blocks) = hash_file(&scratch.path("f")).unwrap();
        assert_eq!(size, data.len() as u64);
        assert_eq!(sha, hex(&sha256(&data)));
        assert_eq!(
            blocks,
            [
                block_hash(&data[..BLOCK as usize]),
                block_hash(&data[BLOCK as usize..BLOCK as usize * 2]),
                block_hash(&data[BLOCK as usize * 2..]),
            ]
        );
        scratch.write("empty", b"");
        assert_eq!(
            hash_file(&scratch.path("empty")).unwrap().2,
            Vec::<BlockHash>::new()
        );
        scratch.write("exact", &data[..BLOCK as usize]);
        assert_eq!(hash_file(&scratch.path("exact")).unwrap().2.len(), 1);
    }

    #[test]
    fn a_walk_leaves_out_the_lock_and_the_working_folder() {
        let scratch = Scratch::new("walk");
        scratch.write("level.dat", b"l");
        scratch.write("session.lock", b"x");
        scratch.write("dimensions/minecraft/overworld/region/r.0.0.mca", b"region");
        scratch.write(".shared-world/index.json", b"{}");
        scratch.write(".shared-world/staging/level.dat", b"old");
        let paths: Vec<String> = walk(&scratch.0)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(
            paths,
            [
                "dimensions/minecraft/overworld/region/r.0.0.mca",
                "level.dat"
            ]
        );
    }

    #[test]
    fn only_what_changed_is_read_again() {
        let scratch = Scratch::new("scan");
        scratch.write("level.dat", b"one");
        scratch.write("region/r.0.0.mca", &bytes(2, 1000));
        scratch.write("region/r.0.1.mca", &bytes(3, 1000));
        let mut index = LocalIndex::default();
        let first = rescan(&scratch.0, &mut index).unwrap();
        assert_eq!(first.changed.len(), 3);
        assert_eq!(index.files.len(), 3);
        // Just written: within the racy window everything is read again.
        assert_eq!(changes(&scratch.0, &index).unwrap().changed.len(), 3);
        let mut index = settled(index);
        assert!(changes(&scratch.0, &index).unwrap().is_empty());
        // A longer file, a new file and a removed one.
        scratch.write("level.dat", b"longer");
        scratch.write("region/r.1.1.mca", b"new");
        fs::remove_file(scratch.path("region/r.0.1.mca")).unwrap();
        let found = changes(&scratch.0, &index).unwrap();
        let changed: Vec<&str> = found.changed.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(changed, ["level.dat", "region/r.1.1.mca"]);
        assert_eq!(found.removed, ["region/r.0.1.mca"]);
        absorb(&mut index, &found, &|path| scratch.path(path)).unwrap();
        assert_eq!(index.files["level.dat"].sha256, hex(&sha256(b"longer")));
        assert!(!index.files.contains_key("region/r.0.1.mca"));
        assert_eq!(index.files.len(), 3);
    }

    #[test]
    fn staged_copies_are_what_the_scan_saw() {
        let world = Scratch::new("world");
        let staging = Scratch::new("staging");
        world.write("level.dat", b"at the checkpoint");
        world.write("region/r.0.0.mca", &bytes(4, 5000));
        let mut index = LocalIndex::default();
        let found = changes(&world.0, &index).unwrap();
        stage(&world.0, &staging.0, &found).unwrap();
        // The world runs on and writes again; the copy still holds the checkpoint.
        world.write("level.dat", b"after the checkpoint, and longer");
        absorb(&mut index, &found, &|path| staging.path(path)).unwrap();
        assert_eq!(
            index.files["level.dat"].sha256,
            hex(&sha256(b"at the checkpoint"))
        );
        // Staging again replaces the copy, and a removed file's copy goes.
        let mut index = settled(index);
        fs::remove_file(world.path("region/r.0.0.mca")).unwrap();
        let found = changes(&world.0, &index).unwrap();
        stage(&world.0, &staging.0, &found).unwrap();
        assert_eq!(
            fs::read(staging.path("level.dat")).unwrap(),
            b"after the checkpoint, and longer"
        );
        assert!(!staging.path("region/r.0.0.mca").exists());
        // A file that grew between the scan and its hashing is caught, and the index kept.
        let before = index.clone();
        let mut stale = found.clone();
        stale.changed[0].size += 1;
        assert!(matches!(
            absorb(&mut index, &stale, &|path| staging.path(path)),
            Err(Error::Corrupt(_))
        ));
        assert_eq!(index, before);
    }

    #[test]
    fn the_index_survives_a_restart_and_a_damaged_file_is_an_empty_index() {
        let scratch = Scratch::new("index");
        scratch.write("world/level.dat", b"x");
        let mut index = LocalIndex::default();
        rescan(&scratch.path("world"), &mut index).unwrap();
        let saved = scratch.path("state/index.json");
        index.save(&saved).unwrap();
        assert_eq!(LocalIndex::load(&saved), index);
        fs::write(&saved, b"{ not json").unwrap();
        assert_eq!(LocalIndex::load(&saved), LocalIndex::default());
        assert_eq!(
            LocalIndex::load(&scratch.path("state/none.json")),
            LocalIndex::default()
        );
    }
}
