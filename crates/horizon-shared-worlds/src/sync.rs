//! Publishing a copy: the blocks that differ from the last committed generation go up as packs,
//! then the generation's manifest. Nothing here touches the lease: the copy counts only once
//! its host commits it ([`crate::lease::Held::commit`]).
//!
//! Every block is hashed again as it is read and must be the block the index holds, so a file
//! that changed underneath is caught here instead of being published half old, half new.
use crate::{
    Error, Result,
    manifest::{
        BLOCK, FileEntry, GenId, Manifest, PACK_LIMIT, Run, VERSION, block_hash, block_len,
    },
    scan::LocalIndex,
    store::{Condition, Store},
};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

/// Called with the bytes done and the bytes in all; false calls the work off.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64) -> bool;

/// Where a block's bytes are.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// In a pack of the previous manifest (its index there), at this offset.
    Kept(u32, u64),
    /// In this generation's `n`th pack, at this offset.
    New(u32, u64),
}

/// A world's keys in its bucket.
#[derive(Clone, Copy)]
pub struct Remote<'a> {
    pub store: &'a dyn Store,
    /// `w/<world id>/`.
    pub prefix: &'a str,
}

impl Remote<'_> {
    pub fn key(&self, name: &str) -> String {
        format!("{}{name}", self.prefix)
    }

    pub fn lease_key(&self) -> String {
        self.key("lease")
    }

    /// A generation's manifest, checked.
    pub fn manifest(&self, id: GenId) -> Result<Manifest> {
        let key = self.key(&id.manifest_key());
        let object = self
            .store
            .get(&key)?
            .ok_or_else(|| Error::Store(format!("the copy {key} is no longer in the storage")))?;
        let manifest = Manifest::from_bytes(&object.data)?;
        if manifest.id != id {
            return Err(Error::Corrupt(format!(
                "{key} holds another copy's file list"
            )));
        }
        Ok(manifest)
    }

    /// Writes an object that is written once. If it is already there with the same bytes (a
    /// write whose answer was lost), that is the write.
    fn put_once(&self, key: &str, data: &[u8]) -> Result<()> {
        match self.store.put(key, data, &Condition::Absent) {
            Ok(_) => Ok(()),
            Err(Error::Refused) => match self.store.get(key)? {
                Some(object) if object.data == data => Ok(()),
                _ => Err(Error::Corrupt(format!(
                    "{key} is already there with other contents"
                ))),
            },
            Err(e) => Err(e),
        }
    }
}

/// How much of a save a new generation would upload: (bytes to upload, bytes of the save).
pub fn upload_size(previous: Option<&Manifest>, index: &LocalIndex) -> (u64, u64) {
    let mut upload = 0;
    let mut whole = 0;
    for (path, file) in &index.files {
        whole += file.size;
        let old = previous.and_then(|m| m.file(path));
        for (i, hash) in file.blocks.iter().enumerate() {
            if old.and_then(|o| o.blocks.get(i)) != Some(hash) {
                upload += block_len(file.size, i as u64);
            }
        }
    }
    (upload, whole)
}

/// Uploads generation `id` of the save `index` describes and returns its manifest. `previous` is
/// the last *committed* generation, whose blocks are reused where they are the same. `source`
/// says where a file's bytes are to be read (a staged copy, or the closed world's own file).
///
/// `id` must be new: a generation that failed half-way is made again under the next `seq`.
pub fn publish(
    remote: Remote<'_>,
    previous: Option<&Manifest>,
    index: &LocalIndex,
    source: &dyn Fn(&str) -> PathBuf,
    id: GenId,
    by: &str,
    progress: Progress<'_>,
) -> Result<Manifest> {
    let (total, _) = upload_size(previous, index);
    let mut done = 0u64;
    let mut pack: Vec<u8> = Vec::new();
    let mut pack_number = 0u32;
    let mut pack_names: Vec<String> = Vec::new();
    // Every file's blocks with their places.
    let mut placed: Vec<(&str, Vec<Place>)> = Vec::with_capacity(index.files.len());
    let flush = |pack: &mut Vec<u8>, number: &mut u32, names: &mut Vec<String>| -> Result<()> {
        if pack.is_empty() {
            return Ok(());
        }
        let name = id.pack_name(*number);
        remote.put_once(&remote.key(&name), pack)?;
        names.push(name);
        *number += 1;
        pack.clear();
        Ok(())
    };
    for (path, file) in &index.files {
        let old = previous.and_then(|m| m.file(path));
        let mut places = Vec::with_capacity(file.blocks.len());
        let mut reader: Option<File> = None;
        for (i, hash) in file.blocks.iter().enumerate() {
            let i = i as u64;
            let kept = old
                .filter(|o| o.blocks.get(i as usize) == Some(hash))
                .and_then(|o| o.locate(i));
            if let Some((pack_index, offset)) = kept {
                places.push(Place::Kept(pack_index, offset));
                continue;
            }
            let length = block_len(file.size, i);
            if pack.len() as u64 + length > PACK_LIMIT {
                flush(&mut pack, &mut pack_number, &mut pack_names)?;
            }
            if reader.is_none() {
                reader = Some(File::open(source(path))?);
            }
            let Some(reader) = reader.as_mut() else {
                continue;
            };
            let start = pack.len();
            pack.resize(start + length as usize, 0);
            reader.seek(SeekFrom::Start(i * BLOCK))?;
            let read = reader.read_exact(&mut pack[start..]);
            if read.is_err() || block_hash(&pack[start..]) != *hash {
                return Err(Error::Corrupt(format!(
                    "{path} is not the file that was scanned: it changed, and this copy is given up"
                )));
            }
            places.push(Place::New(pack_number, start as u64));
            done += length;
            if !progress(done, total) {
                return Err(Error::Cancelled);
            }
        }
        placed.push((path.as_str(), places));
    }
    flush(&mut pack, &mut pack_number, &mut pack_names)?;
    // The packs this generation points into: those kept from the previous one that are still
    // used, then its own.
    let old_names: &[String] = previous.map_or(&[][..], |m| m.packs.as_slice());
    let mut used = vec![false; old_names.len()];
    for (_, places) in &placed {
        for place in places {
            if let Place::Kept(pack_index, _) = place {
                used[*pack_index as usize] = true;
            }
        }
    }
    let mut renumbered = vec![u32::MAX; old_names.len()];
    let mut packs: Vec<String> = Vec::new();
    for (i, name) in old_names.iter().enumerate() {
        if used[i] {
            renumbered[i] = packs.len() as u32;
            packs.push(name.clone());
        }
    }
    let first_new = packs.len() as u32;
    packs.extend(pack_names);
    let files = placed
        .into_iter()
        .map(|(path, places)| {
            let file = &index.files[path];
            let mut runs: Vec<Run> = Vec::new();
            for place in places {
                let (pack_index, offset) = match place {
                    Place::Kept(pack_index, offset) => (renumbered[pack_index as usize], offset),
                    Place::New(number, offset) => (first_new + number, offset),
                };
                match runs.last_mut() {
                    Some(run)
                        if run.pack == pack_index
                            && run.offset + u64::from(run.blocks) * BLOCK == offset =>
                    {
                        run.blocks += 1;
                    }
                    _ => runs.push(Run {
                        pack: pack_index,
                        offset,
                        blocks: 1,
                    }),
                }
            }
            FileEntry {
                path: path.to_owned(),
                size: file.size,
                sha256: file.sha256.clone(),
                blocks: file.blocks.clone(),
                runs,
            }
        })
        .collect();
    let manifest = Manifest {
        version: VERSION,
        id,
        parent: previous.map(|m| m.id),
        by: by.to_owned(),
        packs,
        files,
    };
    manifest.check()?;
    remote.put_once(&remote.key(&id.manifest_key()), &manifest.to_bytes()?)?;
    Ok(manifest)
}

/// Reads the bytes of one run of blocks of `file` from its pack.
pub fn read_run(
    remote: Remote<'_>,
    manifest: &Manifest,
    file: &FileEntry,
    first: u64,
    blocks: u64,
) -> Result<Vec<u8>> {
    let (pack_index, offset) = file
        .locate(first)
        .ok_or_else(|| Error::Format(format!("{} has no block {first}", file.path)))?;
    let length: u64 = (first..first + blocks)
        .map(|i| block_len(file.size, i))
        .sum();
    let name = &manifest.packs[pack_index as usize];
    let data = remote.store.get_range(&remote.key(name), offset, length)?;
    let mut at = 0usize;
    for i in first..first + blocks {
        let end = at + block_len(file.size, i) as usize;
        if block_hash(&data[at..end]) != file.blocks[i as usize] {
            return Err(Error::Corrupt(format!(
                "{name} holds other bytes for {}",
                file.path
            )));
        }
        at = end;
    }
    Ok(data)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        scan::{
            rescan,
            tests::{Scratch, bytes},
            under,
        },
        store::MemoryStore,
    };

    pub(crate) const PREFIX: &str = "w/0123456789abcdef/";

    pub(crate) fn id(epoch: u64, seq: u32) -> GenId {
        GenId {
            epoch,
            seq,
            day: 20261113,
        }
    }

    pub(crate) fn go(_: u64, _: u64) -> bool {
        true
    }

    /// Scans `world` and publishes it as generation `id` on top of `previous`.
    pub(crate) fn publish_world(
        store: &MemoryStore,
        world: &Scratch,
        index: &mut LocalIndex,
        previous: Option<&Manifest>,
        id: GenId,
    ) -> Manifest {
        rescan(&world.0, index).unwrap();
        let remote = Remote {
            store,
            prefix: PREFIX,
        };
        publish(
            remote,
            previous,
            index,
            &|path| under(&world.0, path),
            id,
            "Alex",
            &mut go,
        )
        .unwrap()
    }

    /// The whole of `file` as the storage holds it.
    fn stored(store: &MemoryStore, manifest: &Manifest, path: &str) -> Vec<u8> {
        let remote = Remote {
            store,
            prefix: PREFIX,
        };
        let file = manifest.file(path).unwrap();
        let mut data = Vec::new();
        for i in 0..file.blocks.len() as u64 {
            data.extend(read_run(remote, manifest, file, i, 1).unwrap());
        }
        data
    }

    #[test]
    fn a_first_copy_holds_every_file() {
        let store = MemoryStore::new();
        let world = Scratch::new("pub1");
        let region = bytes(1, BLOCK as usize * 3 + 123);
        world.write("level.dat", b"level");
        world.write("region/r.0.0.mca", &region);
        world.write("data/empty.dat", b"");
        let mut index = LocalIndex::default();
        let manifest = publish_world(&store, &world, &mut index, None, id(1, 1));
        assert_eq!(manifest.parent, None);
        assert_eq!(manifest.files.len(), 3);
        assert_eq!(manifest.packs, ["pack/0000000001-000001-000.pack"]);
        assert_eq!(stored(&store, &manifest, "region/r.0.0.mca"), region);
        assert_eq!(stored(&store, &manifest, "level.dat"), b"level");
        assert!(manifest.file("data/empty.dat").unwrap().runs.is_empty());
        // The blocks of one file lie one after another: one run.
        assert_eq!(manifest.file("region/r.0.0.mca").unwrap().runs.len(), 1);
        let remote = Remote {
            store: &store,
            prefix: PREFIX,
        };
        assert_eq!(remote.manifest(id(1, 1)).unwrap(), manifest);
        assert!(remote.manifest(id(1, 2)).is_err());
    }

    #[test]
    fn the_next_copy_uploads_only_what_changed() {
        let store = MemoryStore::new();
        let world = Scratch::new("pub2");
        let mut region = bytes(2, BLOCK as usize * 8);
        world.write("region/r.0.0.mca", &region);
        world.write("region/r.0.1.mca", &bytes(3, BLOCK as usize * 4));
        world.write("level.dat", b"one");
        let mut index = LocalIndex::default();
        let first = publish_world(&store, &world, &mut index, None, id(1, 1));
        let before = store.bytes();
        // One sector of one region, the level file, a new file, a removed one.
        region[BLOCK as usize * 5 + 4096] ^= 0xff;
        world.write("region/r.0.0.mca", &region);
        world.write("level.dat", b"two");
        world.write("region/r.1.1.mca", b"fresh");
        std::fs::remove_file(world.path("region/r.0.1.mca")).unwrap();
        index.scanned = 0;
        let (upload, whole) = {
            let mut probe = index.clone();
            rescan(&world.0, &mut probe).unwrap();
            upload_size(Some(&first), &probe)
        };
        assert_eq!(upload, BLOCK + 3 + 5);
        assert_eq!(whole, BLOCK * 8 + 3 + 5);
        let second = publish_world(&store, &world, &mut index, Some(&first), id(1, 2));
        let manifest_bytes = second.to_bytes().unwrap().len() as u64;
        assert_eq!(store.bytes() - before, upload + manifest_bytes);
        assert_eq!(second.parent, Some(id(1, 1)));
        assert_eq!(stored(&store, &second, "region/r.0.0.mca"), region);
        assert_eq!(stored(&store, &second, "level.dat"), b"two");
        assert_eq!(stored(&store, &second, "region/r.1.1.mca"), b"fresh");
        assert!(second.file("region/r.0.1.mca").is_none());
        // The region is the old pack around one block of the new one.
        let runs = &second.file("region/r.0.0.mca").unwrap().runs;
        assert_eq!(runs.iter().map(|r| r.blocks).collect::<Vec<_>>(), [5, 1, 2]);
        // Nothing changed: nothing but a manifest goes up, and it points at the same packs.
        let before = store.bytes();
        let third = publish_world(&store, &world, &mut index, Some(&second), id(1, 3));
        assert_eq!(
            store.bytes() - before,
            third.to_bytes().unwrap().len() as u64
        );
        assert_eq!(third.packs, second.packs);
        assert_eq!(third.files, second.files);
    }

    #[test]
    fn a_pack_no_longer_used_is_dropped_from_the_list() {
        let store = MemoryStore::new();
        let world = Scratch::new("pub3");
        world.write("a", &bytes(4, 100));
        let mut index = LocalIndex::default();
        let first = publish_world(&store, &world, &mut index, None, id(1, 1));
        world.write("a", &bytes(5, 100));
        index.scanned = 0;
        let second = publish_world(&store, &world, &mut index, Some(&first), id(1, 2));
        assert_eq!(second.packs, ["pack/0000000001-000002-000.pack"]);
        assert_eq!(stored(&store, &second, "a"), bytes(5, 100));
    }

    #[test]
    fn a_large_save_is_cut_into_packs() {
        let store = MemoryStore::new();
        let world = Scratch::new("pub4");
        let big = bytes(6, PACK_LIMIT as usize + BLOCK as usize * 3 + 17);
        world.write("region/r.0.0.mca", &big);
        let mut index = LocalIndex::default();
        let manifest = publish_world(&store, &world, &mut index, None, id(2, 1));
        assert_eq!(manifest.packs.len(), 2);
        assert_eq!(manifest.file("region/r.0.0.mca").unwrap().runs.len(), 2);
        assert_eq!(stored(&store, &manifest, "region/r.0.0.mca"), big);
        for key in store.keys().iter().filter(|k| k.contains("pack/")) {
            assert!(store.get(key).unwrap().unwrap().data.len() as u64 <= PACK_LIMIT);
        }
    }

    #[test]
    fn a_file_that_changed_since_the_scan_is_not_published() {
        let store = MemoryStore::new();
        let world = Scratch::new("pub5");
        world.write("level.dat", b"scanned");
        let mut index = LocalIndex::default();
        rescan(&world.0, &mut index).unwrap();
        world.write("level.dat", b"written");
        let remote = Remote {
            store: &store,
            prefix: PREFIX,
        };
        let result = publish(
            remote,
            None,
            &index,
            &|path| under(&world.0, path),
            id(1, 1),
            "Alex",
            &mut go,
        );
        assert!(matches!(result, Err(Error::Corrupt(_))));
        assert!(
            store.keys().iter().all(|k| !k.contains("gen/")),
            "no manifest for a copy given up"
        );
    }

    #[test]
    fn publishing_can_be_called_off_and_a_lost_answer_is_not_a_conflict() {
        let store = MemoryStore::new();
        let world = Scratch::new("pub6");
        world.write("a", &bytes(7, BLOCK as usize * 4));
        let mut index = LocalIndex::default();
        rescan(&world.0, &mut index).unwrap();
        let remote = Remote {
            store: &store,
            prefix: PREFIX,
        };
        let mut stop = |done: u64, _: u64| done < BLOCK * 2;
        let result = publish(
            remote,
            None,
            &index,
            &|path| under(&world.0, path),
            id(1, 1),
            "Alex",
            &mut stop,
        );
        assert!(matches!(result, Err(Error::Cancelled)));
        // The pack lands and its answer is lost; the manifest's too. Each is found to be there.
        store
            .fail_after_write
            .store(1, std::sync::atomic::Ordering::SeqCst);
        let made = publish(
            remote,
            None,
            &index,
            &|path| under(&world.0, path),
            id(1, 2),
            "Alex",
            &mut go,
        );
        assert!(
            matches!(made, Err(Error::Store(_))),
            "the pack's write reports the failure"
        );
        // Made again under the same id, the pack that did land is accepted as written.
        let made = publish(
            remote,
            None,
            &index,
            &|path| under(&world.0, path),
            id(1, 2),
            "Alex",
            &mut go,
        )
        .unwrap();
        assert_eq!(stored(&store, &made, "a"), bytes(7, BLOCK as usize * 4));
    }
}
