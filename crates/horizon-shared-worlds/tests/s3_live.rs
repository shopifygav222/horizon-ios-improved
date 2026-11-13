//! The same flows against a real S3 API, when one is given:
//!
//!     SHARED_WORLDS_S3_ENDPOINT=http://127.0.0.1:9000 SHARED_WORLDS_S3_BUCKET=test \
//!     SHARED_WORLDS_S3_KEY=... SHARED_WORLDS_S3_SECRET=... cargo test -p horizon-shared-worlds --test s3_live
//!
//! CI runs it against MinIO. Without the variables every test here passes without doing anything.
use horizon_shared_worlds::{
    Error,
    apply::{Layout, fetch},
    codec::{hex, random},
    lease::{Claim, Leases, Member, State},
    manifest::{BLOCK, GenId},
    retention::generations,
    s3::{Bucket, S3Store, self_test},
    scan::{LocalIndex, rescan, under, walk},
    store::{Condition, Store},
    sync::{Remote, publish},
};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn store() -> Option<S3Store> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let bucket = Bucket {
        endpoint: var("SHARED_WORLDS_S3_ENDPOINT")?,
        bucket: var("SHARED_WORLDS_S3_BUCKET")?,
        region: var("SHARED_WORLDS_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
        access_key: var("SHARED_WORLDS_S3_KEY")?,
        secret_key: var("SHARED_WORLDS_S3_SECRET")?,
    };
    Some(S3Store::new(&bucket).unwrap())
}

fn prefix() -> String {
    format!("w/{}/", hex(&random::<8>().unwrap()))
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("hsw-live-{name}-{}", hex(&random::<8>().unwrap())));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn write(&self, path: &str, data: &[u8]) {
        let to = under(&self.0, path);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::write(to, data).unwrap();
    }
    fn tree(&self) -> BTreeMap<String, Vec<u8>> {
        walk(&self.0)
            .unwrap()
            .into_iter()
            .map(|f| {
                let data = fs::read(under(&self.0, &f.path)).unwrap();
                (f.path, data)
            })
            .collect()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bytes(seed: u64, length: usize) -> Vec<u8> {
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

fn go(_: u64, _: u64) -> bool {
    true
}

#[test]
fn the_storage_enforces_what_a_shared_world_relies_on() {
    let Some(store) = store() else { return };
    self_test(&store, &prefix()).unwrap();
}

#[test]
fn objects_ranges_and_listings() {
    let Some(store) = store() else { return };
    let prefix = prefix();
    assert!(store.get(&format!("{prefix}none")).unwrap().is_none());
    let data = bytes(1, 300_000);
    let key = format!("{prefix}pack/one two+three.pack");
    let etag = store.put(&key, &data, &Condition::Absent).unwrap();
    let object = store.get(&key).unwrap().unwrap();
    assert_eq!(object.etag, etag);
    assert_eq!(object.data, data);
    assert_eq!(store.get_range(&key, 100_000, 70_000).unwrap(), data[100_000..170_000]);
    assert_eq!(store.get_range(&key, 299_999, 1).unwrap(), data[299_999..]);
    assert!(store.get_range(&key, 299_999, 2).is_err());
    assert!(matches!(store.put(&key, b"x", &Condition::Absent), Err(Error::Refused)));
    for i in 0..5 {
        store.put(&format!("{prefix}gen/{i}"), b"g", &Condition::Always).unwrap();
    }
    let listed = store.list(&format!("{prefix}gen/")).unwrap();
    assert_eq!(listed.len(), 5);
    assert!(listed.iter().all(|o| o.size == 1));
    assert_eq!(store.list(&prefix).unwrap().len(), 6);
    for object in store.list(&prefix).unwrap() {
        store.delete(&object.key).unwrap();
    }
    store.delete(&key).unwrap();
    assert!(store.list(&prefix).unwrap().is_empty());
}

#[test]
fn one_of_two_games_gets_the_world() {
    let Some(store) = store() else { return };
    let prefix = prefix();
    let key = format!("{prefix}lease");
    let leases = Leases::new(&store, &key);
    let alex = Member { uuid: "0".repeat(32), name: "Alex".into() };
    let steve = Member { uuid: "1".repeat(32), name: "Steve".into() };
    let mut held = leases.claim(None, &alex, "phone", Claim::Free).unwrap();
    assert!(matches!(leases.claim(None, &steve, "pc", Claim::Free), Err(Error::Refused)));
    held.open("CODE", false).unwrap();
    held.heartbeat(&[alex.clone(), steve.clone()]).unwrap();
    let seen = leases.read().unwrap().unwrap();
    assert_eq!((seen.lease.state, seen.lease.playing()), (State::Hosting, 2));
    // Steve takes over; Alex's next write is refused and Alex is told.
    let mut taken = leases.claim(Some(&seen), &steve, "pc", Claim::Takeover).unwrap();
    assert!(matches!(held.heartbeat(&[alex.clone()]), Err(Error::Fenced)));
    let generation = GenId { epoch: 2, seq: 1, day: 20261113 };
    taken.hand_off(generation, None).unwrap();
    let closed = leases.read().unwrap().unwrap();
    assert_eq!((closed.lease.state, closed.lease.generation), (State::Closed, Some(generation)));
    // Two games open the closed world at once, in threads: exactly one gets it.
    let won: Vec<bool> = std::thread::scope(|scope| {
        let games: Vec<_> = [&alex, &steve]
            .into_iter()
            .map(|me| {
                let closed = &closed;
                scope.spawn(move || leases.claim(Some(closed), me, "device", Claim::Free).is_ok())
            })
            .collect();
        games.into_iter().map(|g| g.join().unwrap()).collect()
    });
    assert_eq!(won.iter().filter(|w| **w).count(), 1, "{won:?}");
    store.delete(&key).unwrap();
}

#[test]
fn a_world_goes_up_and_comes_down_the_same() {
    let Some(store) = store() else { return };
    let prefix = prefix();
    let remote = Remote { store: &store, prefix: &prefix };
    let host = Scratch::new("host");
    let mut region = bytes(2, BLOCK as usize * 20 + 77);
    host.write("level.dat", b"first");
    host.write("dimensions/minecraft/overworld/region/r.0.0.mca", &region);
    host.write("dimensions/minecraft/overworld/region/r.-1.0.mca", &bytes(3, 40_000));
    let mut index = LocalIndex::default();
    rescan(&host.0, &mut index).unwrap();
    let first_id = GenId { epoch: 1, seq: 1, day: 20261113 };
    let first = publish(remote, None, &index, &|p| under(&host.0, p), first_id, "Alex", &mut go).unwrap();
    let guest = Scratch::new("guest");
    let layout = Layout::new(&guest.0);
    let mut guest_index = LocalIndex::default();
    let fetched = fetch(remote, &remote.manifest(first_id).unwrap(), &layout, &mut guest_index, &mut go).unwrap();
    assert_eq!(fetched.downloaded, first.size());
    assert_eq!(guest.tree(), host.tree());
    // The host plays on: three blocks of the region, the level file, one file gone.
    for block in [2usize, 3, 17] {
        region[BLOCK as usize * block + 5] ^= 0x0f;
    }
    host.write("dimensions/minecraft/overworld/region/r.0.0.mca", &region);
    host.write("level.dat", b"second");
    fs::remove_file(under(&host.0, "dimensions/minecraft/overworld/region/r.-1.0.mca")).unwrap();
    index.scanned = 0;
    rescan(&host.0, &mut index).unwrap();
    let second_id = GenId { epoch: 1, seq: 2, day: 20261113 };
    publish(remote, Some(&first), &index, &|p| under(&host.0, p), second_id, "Alex", &mut go).unwrap();
    let second = remote.manifest(second_id).unwrap();
    let fetched = fetch(remote, &second, &layout, &mut guest_index, &mut go).unwrap();
    assert_eq!(fetched.downloaded, BLOCK * 3 + 6);
    assert_eq!(guest.tree(), host.tree());
    assert_eq!(generations(remote).unwrap(), [first_id, second_id]);
    for object in store.list(&prefix).unwrap() {
        store.delete(&object.key).unwrap();
    }
}
