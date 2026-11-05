//! Storage: an ordered key-value store with optimistic, serializable transactions.
//!
//! The runtime only talks to storage through [`Store`] and [`Transaction`], so a production
//! backend (`FoundationDB`, `TiKV`, ...) can replace [`MemoryStore`] without touching the
//! interpreter.

use crate::keys::{Key, KeyRange};

mod memory;

pub use memory::MemoryStore;

/// Scan direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Ascending key order.
    Forward,
    /// Descending key order.
    Reverse,
}

/// A storage failure that isn't a transaction conflict.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("storage unavailable: {0}")]
pub struct StoreError(pub String);

/// Why a commit failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommitError {
    /// Another transaction changed something this one read. Re-running it is safe.
    #[error("transaction conflict")]
    Conflict,
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// An ordered key-value store.
pub trait Store: Send + Sync {
    /// Starts a transaction.
    fn begin(&self) -> Box<dyn Transaction + '_>;
}

/// An optimistic transaction. Reads see the transaction's own writes. Nothing is visible to
/// others until [`Transaction::commit`] succeeds, and dropping the transaction discards it.
pub trait Transaction {
    /// Reads one key.
    fn get(&mut self, key: &Key) -> Result<Option<Vec<u8>>, StoreError>;
    /// Reads up to `limit` entries of `range` in `direction`.
    fn scan(
        &mut self,
        range: &KeyRange,
        limit: usize,
        direction: Direction,
    ) -> Result<Vec<(Key, Vec<u8>)>, StoreError>;
    /// Writes one key.
    fn put(&mut self, key: Key, value: Vec<u8>);
    /// Deletes one key.
    fn delete(&mut self, key: Key);
    /// Commits atomically, or fails with [`CommitError::Conflict`] if anything read has changed.
    fn commit(self: Box<Self>) -> Result<(), CommitError>;
}
