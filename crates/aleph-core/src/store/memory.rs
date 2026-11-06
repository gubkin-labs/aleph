//! An in-memory store with serializable optimistic concurrency control. Used by `aleph dev`, by
//! tests, and as the reference behavior for real backends.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{CommitError, Direction, Store, StoreError, Transaction};
use crate::keys::{Key, KeyRange};

#[derive(Debug, Default)]
struct Inner {
    data: BTreeMap<Key, Versioned>,
    /// Incremented on every commit. Versions start at 1, and 0 means "absent".
    clock: u64,
}

#[derive(Debug, Clone)]
struct Versioned {
    version: u64,
    value: Vec<u8>,
}

/// An in-memory [`Store`].
#[derive(Debug, Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every committed entry, in key order. For tests and debugging.
    pub fn entries(&self) -> Vec<(Key, Vec<u8>)> {
        self.lock()
            .data
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect()
    }

    /// Replaces the contents with `entries` (from [`MemoryStore::entries`]), as committed data.
    /// Used to persist `aleph dev` data between restarts.
    pub fn restore(&self, entries: Vec<(Key, Vec<u8>)>) {
        let mut inner = self.lock();
        inner.clock += 1;
        let version = inner.clock;
        inner.data = entries
            .into_iter()
            .map(|(k, value)| (k, Versioned { version, value }))
            .collect();
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock can't leave `Inner` half-updated: commits validate before
        // they write, and writes are plain map inserts.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Store for MemoryStore {
    fn begin(&self) -> Box<dyn Transaction + '_> {
        Box::new(MemoryTx {
            store: self,
            reads: BTreeMap::new(),
            scans: Vec::new(),
            writes: BTreeMap::new(),
        })
    }
}

/// A scanned interval and the committed `(key, version)` pairs seen in it. If another commit
/// adds, removes, or changes anything inside the interval, the scan's result could differ, so the
/// transaction must not commit (this is how phantoms are prevented).
struct ScanRecord {
    covered: KeyRange,
    seen: Vec<(Key, u64)>,
}

struct MemoryTx<'a> {
    store: &'a MemoryStore,
    /// Version of each key read from committed state (0 if it was absent).
    reads: BTreeMap<Key, u64>,
    scans: Vec<ScanRecord>,
    /// Buffered writes. `None` is a delete.
    writes: BTreeMap<Key, Option<Vec<u8>>>,
}

impl MemoryTx<'_> {
    /// Fails if any recorded read or scan would now see something different.
    fn check(&self, inner: &Inner) -> Result<(), CommitError> {
        for (key, version) in &self.reads {
            if inner.data.get(key).map_or(0, |v| v.version) != *version {
                return Err(CommitError::Conflict);
            }
        }
        for scan in &self.scans {
            if committed_in(inner, &scan.covered) != scan.seen {
                return Err(CommitError::Conflict);
            }
        }
        Ok(())
    }
}

fn committed_in(inner: &Inner, range: &KeyRange) -> Vec<(Key, u64)> {
    inner
        .data
        .range(range.start.clone()..range.end.clone())
        .map(|(k, v)| (k.clone(), v.version))
        .collect()
}

impl Transaction for MemoryTx<'_> {
    fn get(&mut self, key: &Key) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(buffered) = self.writes.get(key) {
            return Ok(buffered.clone());
        }
        let inner = self.store.lock();
        let found = inner.data.get(key);
        self.reads
            .entry(key.clone())
            .or_insert(found.map_or(0, |v| v.version));
        Ok(found.map(|v| v.value.clone()))
    }

    fn scan(
        &mut self,
        range: &KeyRange,
        limit: usize,
        direction: Direction,
    ) -> Result<Vec<(Key, Vec<u8>)>, StoreError> {
        let inner = self.store.lock();
        // Committed state overlaid with this transaction's own writes.
        let mut merged: BTreeMap<Key, Vec<u8>> = inner
            .data
            .range(range.start.clone()..range.end.clone())
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect();
        for (key, write) in self.writes.range(range.start.clone()..range.end.clone()) {
            match write {
                Some(value) => merged.insert(key.clone(), value.clone()),
                None => merged.remove(key),
            };
        }
        let result: Vec<(Key, Vec<u8>)> = match direction {
            Direction::Forward => merged.into_iter().take(limit).collect(),
            Direction::Reverse => merged.into_iter().rev().take(limit).collect(),
        };

        // A full page only depends on the interval up to its last key. A short page depends on
        // the whole range.
        let covered = match (result.last(), result.len() == limit) {
            (Some((last, _)), true) => match direction {
                Direction::Forward => KeyRange {
                    start: range.start.clone(),
                    end: last.successor(),
                },
                Direction::Reverse => KeyRange {
                    start: last.clone(),
                    end: range.end.clone(),
                },
            },
            _ if limit == 0 => KeyRange {
                start: range.start.clone(),
                end: range.start.clone(),
            },
            _ => range.clone(),
        };
        let seen = committed_in(&inner, &covered);
        self.scans.push(ScanRecord { covered, seen });
        Ok(result)
    }

    fn put(&mut self, key: Key, value: Vec<u8>) {
        self.writes.insert(key, Some(value));
    }

    fn delete(&mut self, key: Key) {
        self.writes.insert(key, None);
    }

    fn validate(&self) -> Result<(), CommitError> {
        self.check(&self.store.lock())
    }

    fn commit(self: Box<Self>) -> Result<(), CommitError> {
        let mut inner = self.store.lock();
        self.check(&inner)?;
        if self.writes.is_empty() {
            return Ok(());
        }
        inner.clock += 1;
        let version = inner.clock;
        for (key, write) in self.writes {
            match write {
                Some(value) => inner.data.insert(key, Versioned { version, value }),
                None => inner.data.remove(&key),
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{KeyPart, Keyspace, TenantId};

    fn ks() -> Keyspace {
        Keyspace::new(&TenantId::new("t").unwrap())
    }

    fn key(i: i64) -> Key {
        ks().row("E", &[KeyPart::Int(i)])
    }

    fn put(store: &MemoryStore, i: i64, value: &[u8]) {
        let mut tx = store.begin();
        tx.put(key(i), value.to_vec());
        tx.commit().unwrap();
    }

    #[test]
    fn reads_see_own_writes_and_drop_discards() {
        let store = MemoryStore::new();
        let mut tx = store.begin();
        tx.put(key(1), b"a".to_vec());
        assert_eq!(tx.get(&key(1)).unwrap(), Some(b"a".to_vec()));
        drop(tx);
        assert_eq!(store.entries(), vec![]);
    }

    #[test]
    fn concurrent_read_modify_write_conflicts() {
        let store = MemoryStore::new();
        put(&store, 1, b"0");
        let mut a = store.begin();
        let mut b = store.begin();
        a.get(&key(1)).unwrap();
        b.get(&key(1)).unwrap();
        a.put(key(1), b"a".to_vec());
        b.put(key(1), b"b".to_vec());
        assert_eq!(a.commit(), Ok(()));
        assert_eq!(b.commit(), Err(CommitError::Conflict));
    }

    #[test]
    fn insert_into_scanned_range_is_a_phantom_conflict() {
        let store = MemoryStore::new();
        put(&store, 1, b"x");
        let range = ks().row_range("E", &[]);
        let mut reader = store.begin();
        assert_eq!(
            reader.scan(&range, 10, Direction::Forward).unwrap().len(),
            1
        );
        reader.put(key(99), b"summary".to_vec());
        put(&store, 2, b"y"); // lands inside the scanned range
        assert_eq!(reader.commit(), Err(CommitError::Conflict));
    }

    #[test]
    fn writes_beyond_a_full_page_dont_conflict() {
        let store = MemoryStore::new();
        for i in 1..=3 {
            put(&store, i, b"x");
        }
        let range = ks().row_range("E", &[]);
        let mut reader = store.begin();
        let page = reader.scan(&range, 2, Direction::Forward).unwrap();
        assert_eq!(page.len(), 2);
        reader.put(key(100), b"y".to_vec());
        put(&store, 50, b"z"); // after the page's last key
        assert_eq!(reader.commit(), Ok(()));
    }

    #[test]
    fn validation_detects_stale_reads_without_writing() {
        let store = MemoryStore::new();
        put(&store, 1, b"0");
        let mut tx = store.begin();
        tx.get(&key(1)).unwrap();
        tx.put(key(2), b"never".to_vec());
        assert_eq!(tx.validate(), Ok(()));
        put(&store, 1, b"changed");
        assert_eq!(tx.validate(), Err(CommitError::Conflict));
        drop(tx);
        assert_eq!(store.entries().len(), 1); // the buffered write was never applied
    }

    #[test]
    fn reverse_scans_merge_buffered_writes() {
        let store = MemoryStore::new();
        put(&store, 1, b"a");
        put(&store, 2, b"b");
        let mut tx = store.begin();
        tx.delete(key(2));
        tx.put(key(3), b"c".to_vec());
        let rows = tx
            .scan(&ks().row_range("E", &[]), 10, Direction::Reverse)
            .unwrap();
        let values: Vec<&[u8]> = rows.iter().map(|(_, v)| v.as_slice()).collect();
        assert_eq!(values, vec![b"c".as_slice(), b"a".as_slice()]);
    }
}
