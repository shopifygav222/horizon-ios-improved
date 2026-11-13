//! One copy of a save (a generation): every file's size and SHA-256, cut into 64 KiB blocks,
//! each block with a short hash and the place in a pack where its bytes are.
//!
//! Region files are rewritten in place a few 4 KiB sectors at a time, so between two copies of a
//! world most blocks of most files are the same: a new generation uploads only the blocks that
//! changed, as one pack, and points at the earlier packs for the rest. Manifests and packs are
//! written once and never changed.
use crate::{
    Error, Result,
    codec::{base64, unbase64, unhex},
};
use serde::{Deserialize, Serialize};

/// The manifest's format.
pub const VERSION: u32 = 1;
/// A block: 16 region sectors.
pub const BLOCK: u64 = 64 * 1024;
/// A block's hash in a manifest: the first half of its SHA-256. It only says which blocks
/// differ; every rebuilt file is checked against its whole SHA-256.
pub const SHORT: usize = 16;
pub type BlockHash = [u8; SHORT];
/// The most a pack holds, and so the most ever uploaded or held in memory at once.
pub const PACK_LIMIT: u64 = 16 << 20;

/// Which copy: the host's epoch, its count of copies, and the day (`yyyymmdd`, for keeping one
/// copy a day). Ordered as written: a later epoch is always a later copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GenId {
    pub epoch: u64,
    pub seq: u32,
    pub day: u32,
}

impl GenId {
    /// The manifest's key under the world's prefix.
    pub fn manifest_key(&self) -> String {
        format!("gen/{:010}-{:06}-{:08}.json", self.epoch, self.seq, self.day)
    }

    /// The key of this generation's `n`th pack.
    pub fn pack_name(&self, n: u32) -> String {
        format!("pack/{:010}-{:06}-{n:03}.pack", self.epoch, self.seq)
    }

    /// The generation a manifest key (as [`Self::manifest_key`] writes it) names.
    pub fn from_manifest_key(key: &str) -> Option<Self> {
        let mut parts = key.strip_prefix("gen/")?.strip_suffix(".json")?.split('-');
        let id = Self {
            epoch: parts.next()?.parse().ok()?,
            seq: parts.next()?.parse().ok()?,
            day: parts.next()?.parse().ok()?,
        };
        (parts.next().is_none() && id.manifest_key() == key).then_some(id)
    }

    /// The (epoch, seq) a pack name (as [`Self::pack_name`] writes it) belongs to.
    pub fn of_pack(name: &str) -> Option<(u64, u32)> {
        let mut parts = name.strip_prefix("pack/")?.strip_suffix(".pack")?.split('-');
        let id = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
        parts.next()?.parse::<u32>().ok()?;
        parts.next().is_none().then_some(id)
    }
}

/// How many blocks a file of `size` bytes has.
pub fn block_count(size: u64) -> u64 {
    size.div_ceil(BLOCK)
}

/// The length of block `index` of a file of `size` bytes: [`BLOCK`], or less for the last.
pub fn block_len(size: u64, index: u64) -> u64 {
    (size - index * BLOCK).min(BLOCK)
}

/// The short hash of one block's bytes.
pub fn block_hash(data: &[u8]) -> BlockHash {
    let mut short = [0u8; SHORT];
    short.copy_from_slice(&crate::codec::sha256(data)[..SHORT]);
    short
}

/// Consecutive blocks of a file whose bytes lie one after another in one pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Index into the manifest's `packs`.
    pub pack: u32,
    /// Where the run's first block starts in the pack.
    pub offset: u64,
    pub blocks: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative to the world folder, with `/` between its parts.
    pub path: String,
    pub size: u64,
    /// 64 hex digits.
    pub sha256: String,
    #[serde(with = "short_hashes")]
    pub blocks: Vec<BlockHash>,
    /// Where the blocks' bytes are, in block order.
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: GenId,
    /// The generation this one was made from.
    pub parent: Option<GenId>,
    /// Who made it: the host's name, for people reading the storage.
    pub by: String,
    /// Names of the packs the runs point into (`pack/...`), under the world's prefix.
    pub packs: Vec<String>,
    /// Sorted by path.
    pub files: Vec<FileEntry>,
}

pub(crate) mod short_hashes {
    use super::{BlockHash, SHORT, base64, unbase64};
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(blocks: &[BlockHash], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64(blocks.as_flattened()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<BlockHash>, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = unbase64(&text).map_err(D::Error::custom)?;
        let pieces = bytes.chunks_exact(SHORT);
        if !pieces.remainder().is_empty() {
            return Err(D::Error::custom("block hashes cut short"));
        }
        Ok(pieces
            .map(|piece| {
                let mut hash = [0u8; SHORT];
                hash.copy_from_slice(piece);
                hash
            })
            .collect())
    }
}

/// Whether `path` is one a manifest may name: relative, `/`-separated, no part that climbs out
/// of the world folder or that an operating system treats specially.
pub fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.contains(['\\', ':', '\0', '*', '?', '"', '<', '>', '|'])
                && !part.chars().any(char::is_control)
        })
        && !reserved(path)
}

/// Paths the world folder holds that are never part of a copy: the lock a running game keeps,
/// and this crate's own working folder.
pub fn reserved(path: &str) -> bool {
    path == "session.lock" || path == WORK || path.starts_with(&format!("{WORK}/"))
}

/// This crate's working folder inside a world folder (files being rebuilt, the journal).
pub const WORK: &str = ".shared-world";

impl FileEntry {
    /// The pack (as an index into the manifest's `packs`) and offset of block `index`.
    pub fn locate(&self, index: u64) -> Option<(u32, u64)> {
        let mut first = 0u64;
        for run in &self.runs {
            let blocks = u64::from(run.blocks);
            if index < first + blocks {
                return Some((run.pack, run.offset + (index - first) * BLOCK));
            }
            first += blocks;
        }
        None
    }
}

impl Manifest {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Reads a manifest and checks everything a later step relies on, so nothing downstream
    /// has to trust what came from the storage.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(data)?;
        manifest.check()?;
        Ok(manifest)
    }

    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.files
            .binary_search_by(|f| f.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.files[i])
    }

    /// The bytes of the whole save.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn check(&self) -> Result<()> {
        let bad = |what: String| Err(Error::Format(format!("the file list is wrong: {what}")));
        if self.version > VERSION {
            return Err(Error::Format(
                "this world was saved by a newer version of the game: update to play it".into(),
            ));
        }
        for name in &self.packs {
            if GenId::of_pack(name).is_none() {
                return bad(format!("{name:?} is not a pack"));
            }
        }
        let mut previous: Option<&str> = None;
        for file in &self.files {
            let path = file.path.as_str();
            if !safe_path(path) {
                return bad(format!("{path:?} is not a path inside the world"));
            }
            if previous.is_some_and(|p| p >= path) {
                return bad(format!("{path:?} is out of order"));
            }
            previous = Some(path);
            if unhex(&file.sha256).is_none_or(|h| h.len() != 32) {
                return bad(format!("{path}: not a SHA-256"));
            }
            let blocks = block_count(file.size);
            if file.blocks.len() as u64 != blocks {
                return bad(format!("{path}: {} block hashes for {blocks} blocks", file.blocks.len()));
            }
            let mut covered = 0u64;
            for run in &file.runs {
                if run.blocks == 0 || run.pack as usize >= self.packs.len() {
                    return bad(format!("{path}: a run points nowhere"));
                }
                if run.offset.checked_add(u64::from(run.blocks) * BLOCK).is_none() {
                    return bad(format!("{path}: a run past the end of any pack"));
                }
                covered += u64::from(run.blocks);
            }
            if covered != blocks {
                return bad(format!("{path}: runs cover {covered} of {blocks} blocks"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, size: u64, runs: Vec<Run>) -> FileEntry {
        FileEntry {
            path: path.into(),
            size,
            sha256: "ab".repeat(32),
            blocks: (0..block_count(size)).map(|i| [i as u8; SHORT]).collect(),
            runs,
        }
    }

    fn manifest(files: Vec<FileEntry>) -> Manifest {
        Manifest {
            version: VERSION,
            id: GenId { epoch: 3, seq: 7, day: 20261113 },
            parent: Some(GenId { epoch: 3, seq: 6, day: 20261113 }),
            by: "Alex".into(),
            packs: vec!["pack/0000000003-000006-000.pack".into(), "pack/0000000003-000007-000.pack".into()],
            files,
        }
    }

    #[test]
    fn keys_sort_as_generations_do_and_read_back() {
        let old = GenId { epoch: 9, seq: 120, day: 20261113 };
        let new = GenId { epoch: 10, seq: 1, day: 20261114 };
        assert!(old < new);
        assert!(old.manifest_key() < new.manifest_key());
        assert_eq!(old.manifest_key(), "gen/0000000009-000120-20261113.json");
        assert_eq!(GenId::from_manifest_key(&old.manifest_key()), Some(old));
        assert_eq!(GenId::of_pack(&new.pack_name(4)), Some((10, 1)));
        assert_eq!(new.pack_name(4), "pack/0000000010-000001-004.pack");
        for key in ["gen/9-120-20261113.json", "gen/x.json", "pack/0000000009-000120-000.pack", "gen/0000000009-000120-20261113.json.bak"] {
            assert_eq!(GenId::from_manifest_key(key), None, "{key}");
        }
        assert_eq!(GenId::of_pack("pack/../lease"), None);
    }

    #[test]
    fn blocks_cover_a_file_exactly() {
        assert_eq!(block_count(0), 0);
        assert_eq!(block_count(1), 1);
        assert_eq!(block_count(BLOCK), 1);
        assert_eq!(block_count(BLOCK + 1), 2);
        assert_eq!(block_len(BLOCK + 5, 0), BLOCK);
        assert_eq!(block_len(BLOCK + 5, 1), 5);
        assert_eq!(block_len(BLOCK * 2, 1), BLOCK);
    }

    #[test]
    fn a_manifest_round_trips_and_finds_its_blocks() {
        let files = vec![
            entry("level.dat", 100, vec![Run { pack: 1, offset: 0, blocks: 1 }]),
            entry(
                "region/r.0.0.mca",
                BLOCK * 5 + 9,
                vec![
                    Run { pack: 0, offset: BLOCK * 10, blocks: 2 },
                    Run { pack: 1, offset: 100, blocks: 3 },
                    Run { pack: 0, offset: 0, blocks: 1 },
                ],
            ),
            entry("region/r.0.1.mca", 0, vec![]),
        ];
        let made = manifest(files);
        let read = Manifest::from_bytes(&made.to_bytes().unwrap()).unwrap();
        assert_eq!(read, made);
        let region = read.file("region/r.0.0.mca").unwrap();
        assert_eq!(region.locate(0), Some((0, BLOCK * 10)));
        assert_eq!(region.locate(1), Some((0, BLOCK * 11)));
        assert_eq!(region.locate(2), Some((1, 100)));
        assert_eq!(region.locate(4), Some((1, 100 + BLOCK * 2)));
        assert_eq!(region.locate(5), Some((0, 0)));
        assert_eq!(region.locate(6), None);
        assert!(read.file("region/r.9.9.mca").is_none());
        assert_eq!(read.size(), 100 + BLOCK * 5 + 9);
    }

    #[test]
    fn paths_that_leave_the_world_folder_are_refused() {
        for path in [
            "level.dat",
            "dimensions/minecraft/overworld/region/r.-1.0.mca",
            "players/data/0a1b.dat_old",
            "data/horizon/attachments.dat",
        ] {
            assert!(safe_path(path), "{path}");
        }
        for path in [
            "", "/etc/passwd", "../outside", "a/../../b", "a//b", "a/./b", "C:/x", "a\\b", "a/b.", "a/b ",
            "nul\0", "session.lock", ".shared-world", ".shared-world/journal.json", "a/b?c",
        ] {
            assert!(!safe_path(path), "{path:?}");
        }
        assert!(safe_path(".shared-worldly/x"));
    }

    #[test]
    fn a_wrong_manifest_is_refused_before_anything_uses_it() {
        let good = || {
            vec![
                entry("a", BLOCK * 2, vec![Run { pack: 0, offset: 0, blocks: 2 }]),
                entry("b", 5, vec![Run { pack: 1, offset: 0, blocks: 1 }]),
            ]
        };
        assert!(manifest(good()).check().is_ok());
        let broken: [fn(&mut Manifest); 9] = [
            |m| m.files.swap(0, 1),
            |m| m.files[1].path = "a".into(),
            |m| m.files[0].path = "../a".into(),
            |m| m.files[0].sha256 = "xyz".into(),
            |m| m.files[0].blocks.truncate(1),
            |m| m.files[0].runs[0].blocks = 1,
            |m| m.files[0].runs[0].pack = 2,
            |m| m.files[1].runs[0].blocks = 0,
            |m| m.packs[0] = "lease".into(),
        ];
        for (i, damage) in broken.iter().enumerate() {
            let mut m = manifest(good());
            damage(&mut m);
            assert!(m.check().is_err(), "damage {i}");
            assert!(Manifest::from_bytes(&m.to_bytes().unwrap()).is_err(), "damage {i}");
        }
        let mut newer = manifest(good());
        newer.version = VERSION + 1;
        assert!(newer.check().is_err());
        assert!(Manifest::from_bytes(b"{}").is_err());
    }
}
