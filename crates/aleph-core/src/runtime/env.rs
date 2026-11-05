//! The execution environment: time and id generation, injectable so runs can be deterministic.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// Supplies the current time and new ids.
pub trait Env: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
    /// A new id for the given time.
    fn new_id(&self, now_ms: i64) -> String;
}

/// A deterministic environment: time only moves when told to, and ids come from a seeded
/// generator. Used by tests, conformance runs, and the WASM dev engine.
#[derive(Debug)]
pub struct FixedEnv {
    now: AtomicI64,
    rng: AtomicU64,
}

impl FixedEnv {
    /// An environment frozen at `now_ms`, generating ids from `seed`.
    pub fn new(now_ms: i64, seed: u64) -> Self {
        Self {
            now: AtomicI64::new(now_ms),
            rng: AtomicU64::new(seed),
        }
    }

    /// Sets the current time.
    pub fn set_now(&self, now_ms: i64) {
        self.now.store(now_ms, Ordering::Relaxed);
    }

    /// Moves time forward.
    pub fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::Relaxed);
    }
}

impl Env for FixedEnv {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::Relaxed)
    }

    fn new_id(&self, now_ms: i64) -> String {
        ulid(now_ms, random_80(&self.rng))
    }
}

/// The real clock with random ids. Not available on `wasm32-unknown-unknown`, which has no clock.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct SystemEnv {
    rng: AtomicU64,
}

#[cfg(not(target_arch = "wasm32"))]
impl SystemEnv {
    /// A system environment seeded from the clock.
    pub fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        #[allow(
            clippy::cast_possible_truncation,
            reason = "mixing seed bits; truncation is fine"
        )]
        Self {
            rng: AtomicU64::new((nanos as u64) ^ (nanos >> 64) as u64),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for SystemEnv {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Env for SystemEnv {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    fn new_id(&self, now_ms: i64) -> String {
        ulid(now_ms, random_80(&self.rng))
    }
}

/// `SplitMix64`: tiny, fast, and good enough for id randomness (not for secrets).
fn next(state: &AtomicU64) -> u64 {
    let mut z = state
        .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn random_80(state: &AtomicU64) -> u128 {
    (u128::from(next(state)) << 16 | u128::from(next(state) & 0xFFFF)) & ((1 << 80) - 1)
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A ULID: 48 bits of milliseconds then 80 random bits, in Crockford base32. Lexicographic order
/// follows time order.
pub(crate) fn ulid(now_ms: i64, random: u128) -> String {
    let time = u128::from(now_ms.clamp(0, (1 << 48) - 1).cast_unsigned());
    let bits = (time << 80) | random;
    (0..26)
        .rev()
        .map(|i| char::from(CROCKFORD[usize::try_from((bits >> (i * 5)) & 31).unwrap_or(0)]))
        .collect()
}

/// Whether `s` is a well-formed ULID.
pub(crate) fn is_ulid(s: &str) -> bool {
    s.len() == 26 && s.bytes().all(|b| CROCKFORD.contains(&b)) && s.as_bytes()[0] <= b'7'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_valid_and_time_ordered() {
        let env = FixedEnv::new(1_000, 7);
        let a = env.new_id(env.now_ms());
        env.advance(1);
        let b = env.new_id(env.now_ms());
        assert!(is_ulid(&a) && is_ulid(&b));
        assert!(a < b, "{a} should sort before {b}");
    }

    #[test]
    fn fixed_env_is_deterministic() {
        let ids = |seed| {
            let env = FixedEnv::new(5, seed);
            (0..3).map(|_| env.new_id(5)).collect::<Vec<_>>()
        };
        assert_eq!(ids(1), ids(1));
        assert_ne!(ids(1), ids(2));
    }
}
