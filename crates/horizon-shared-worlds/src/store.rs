//! What a shared world needs from its storage: whole objects by key, a range of one, a listing,
//! and writes that can be made conditional on what is there. [`crate::s3::S3Store`] is the
//! bucket; [`MemoryStore`] behaves the same in memory, for tests and for the checks the game
//! runs without a network.
use crate::{Error, Result};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};

/// What must be true of the object for a write to go through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    /// Write whatever is there.
    Always,
    /// Only if no object has this key (`If-None-Match: *`).
    Absent,
    /// Only if the object is still the version with this tag (`If-Match`).
    Unchanged(Etag),
}

/// The storage's name for one version of an object: it changes with every write.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Etag(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub data: Vec<u8>,
    pub etag: Etag,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub key: String,
    pub size: u64,
}

pub trait Store: Send + Sync {
    /// The object, or None when the key has none.
    fn get(&self, key: &str) -> Result<Option<Object>>;
    /// `length` bytes of the object from `start`; an error when it has fewer.
    fn get_range(&self, key: &str, start: u64, length: u64) -> Result<Vec<u8>>;
    /// Writes the object and returns its new tag. [`Error::Refused`] when `condition` doesn't
    /// hold; nothing is written then.
    fn put(&self, key: &str, data: &[u8], condition: &Condition) -> Result<Etag>;
    /// Every object whose key starts with `prefix`, in key order.
    fn list(&self, prefix: &str) -> Result<Vec<Listed>>;
    /// Removes the object; a key without one is not an error.
    fn delete(&self, key: &str) -> Result<()>;
}

/// A store in memory with the bucket's rules, and switches a test turns to make it fail.
#[derive(Default)]
pub struct MemoryStore {
    objects: Mutex<BTreeMap<String, (Vec<u8>, u64)>>,
    versions: AtomicU64,
    /// The next this many calls fail before doing anything.
    pub fail_before: AtomicU32,
    /// The next this many writes go through, and then report a failure: what a caller sees
    /// when the answer is lost on the way back.
    pub fail_after_write: AtomicU32,
    /// Calls made so far.
    pub calls: AtomicU64,
}

/// Takes one from `counter` if it holds any.
fn take(counter: &AtomicU32) -> bool {
    let mut held = counter.load(Ordering::SeqCst);
    while held > 0 {
        match counter.compare_exchange(held, held - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(now) => held = now,
        }
    }
    false
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn enter(&self) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if take(&self.fail_before) {
            return Err(Error::Store("unreachable (test)".into()));
        }
        Ok(())
    }

    fn objects(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, (Vec<u8>, u64)>> {
        self.objects.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every key held, for a test's assertions.
    pub fn keys(&self) -> Vec<String> {
        self.objects().keys().cloned().collect()
    }

    /// Bytes held in all, for a test's assertions.
    pub fn bytes(&self) -> u64 {
        self.objects().values().map(|(d, _)| d.len() as u64).sum()
    }
}

fn tag(version: u64) -> Etag {
    Etag(format!("\"{version:016x}\""))
}

impl Store for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<Object>> {
        self.enter()?;
        Ok(self.objects().get(key).map(|(data, version)| Object {
            data: data.clone(),
            etag: tag(*version),
        }))
    }

    fn get_range(&self, key: &str, start: u64, length: u64) -> Result<Vec<u8>> {
        self.enter()?;
        let objects = self.objects();
        let (data, _) = objects
            .get(key)
            .ok_or_else(|| Error::Store(format!("{key} is missing")))?;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= data.len() as u64)
            .ok_or_else(|| Error::Corrupt(format!("{key} is shorter than {start}+{length}")))?;
        Ok(data[start as usize..end as usize].to_vec())
    }

    fn put(&self, key: &str, data: &[u8], condition: &Condition) -> Result<Etag> {
        self.enter()?;
        let mut objects = self.objects();
        let holds = match (condition, objects.get(key)) {
            (Condition::Always, _) | (Condition::Absent, None) => true,
            (Condition::Unchanged(etag), Some((_, version))) => tag(*version) == *etag,
            (Condition::Absent, Some(_)) | (Condition::Unchanged(_), None) => false,
        };
        if !holds {
            return Err(Error::Refused);
        }
        let version = self.versions.fetch_add(1, Ordering::SeqCst) + 1;
        objects.insert(key.to_owned(), (data.to_vec(), version));
        if take(&self.fail_after_write) {
            return Err(Error::Store("the answer was lost (test)".into()));
        }
        Ok(tag(version))
    }

    fn list(&self, prefix: &str) -> Result<Vec<Listed>> {
        self.enter()?;
        Ok(self
            .objects()
            .range(prefix.to_owned()..)
            .take_while(|(key, _)| key.starts_with(prefix))
            .map(|(key, (data, _))| Listed {
                key: key.clone(),
                size: data.len() as u64,
            })
            .collect())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.enter()?;
        self.objects().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_let_one_writer_through() {
        let store = MemoryStore::new();
        let first = store.put("k", b"one", &Condition::Absent).unwrap();
        assert!(matches!(
            store.put("k", b"two", &Condition::Absent),
            Err(Error::Refused)
        ));
        let second = store
            .put("k", b"two", &Condition::Unchanged(first.clone()))
            .unwrap();
        assert_ne!(first, second);
        // The tag that was read is no longer the object's: refused, and nothing written.
        assert!(matches!(
            store.put("k", b"three", &Condition::Unchanged(first)),
            Err(Error::Refused)
        ));
        assert!(matches!(
            store.put("absent", b"x", &Condition::Unchanged(second.clone())),
            Err(Error::Refused)
        ));
        let object = store.get("k").unwrap().unwrap();
        assert_eq!((object.data.as_slice(), object.etag), (&b"two"[..], second));
    }

    #[test]
    fn ranges_listings_and_deletes() {
        let store = MemoryStore::new();
        store.put("a/1", b"0123456789", &Condition::Always).unwrap();
        store.put("a/2", b"xy", &Condition::Always).unwrap();
        store.put("b/1", b"z", &Condition::Always).unwrap();
        assert_eq!(store.get_range("a/1", 3, 4).unwrap(), b"3456");
        assert!(matches!(
            store.get_range("a/1", 8, 3),
            Err(Error::Corrupt(_))
        ));
        assert!(store.get_range("nope", 0, 1).is_err());
        let listed = store.list("a/").unwrap();
        assert_eq!(
            listed,
            [
                Listed {
                    key: "a/1".into(),
                    size: 10
                },
                Listed {
                    key: "a/2".into(),
                    size: 2
                }
            ]
        );
        store.delete("a/1").unwrap();
        store.delete("a/1").unwrap();
        assert!(store.get("a/1").unwrap().is_none());
        assert_eq!(store.bytes(), 3);
    }

    #[test]
    fn the_switches_fail_as_a_network_does() {
        let store = MemoryStore::new();
        store.fail_before.store(1, Ordering::SeqCst);
        assert!(matches!(
            store.put("k", b"v", &Condition::Always),
            Err(Error::Store(_))
        ));
        assert!(store.get("k").unwrap().is_none());
        // The write lands although the caller is told it failed.
        store.fail_after_write.store(1, Ordering::SeqCst);
        assert!(matches!(
            store.put("k", b"v", &Condition::Absent),
            Err(Error::Store(_))
        ));
        assert_eq!(store.get("k").unwrap().unwrap().data, b"v");
    }
}
