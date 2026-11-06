//! Storage keys: an order-preserving tuple encoding, always prefixed by the tenant.
//!
//! Plans never build raw keys. Every key is produced by a [`Keyspace`], which is created for one
//! tenant and writes that tenant's prefix first. This is the isolation boundary between apps
//! sharing one store.
//!
//! The encoding follows the `FoundationDB` tuple layer: each part is type-tagged, strings are
//! zero-terminated with embedded zeros escaped, and integers are big-endian with the sign bit
//! flipped. Byte order then matches tuple order, and no encoded tuple is a prefix of another
//! tuple with a different part, so prefixes select exactly their sub-ranges.

use std::fmt;

use crate::value::Value;

const NULL: u8 = 0x00;
const STRING: u8 = 0x02;
const INT: u8 = 0x15;
const DECIMAL: u8 = 0x16;
const FALSE: u8 = 0x26;
const TRUE: u8 = 0x27;
const ESCAPE: u8 = 0xFF;

const ROWS: &str = "r";
const INDEXES: &str = "i";
const COUNTERS: &str = "c";
const IDEMPOTENCY: &str = "d";
const OUTBOX: &str = "o";
const SCHEDULES: &str = "s";
const USERS: &str = "u";
const JOBS: &str = "j";

/// A tenant (app) identifier: 1–64 characters of `[A-Za-z0-9_-]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TenantId(String);

/// Returned when a tenant id is malformed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid tenant id {0:?}: expected 1-64 characters of [A-Za-z0-9_-]")]
pub struct InvalidTenantId(pub String);

impl TenantId {
    /// Validates and wraps a tenant id.
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidTenantId> {
        let id = id.into();
        let valid = (1..=64).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if valid {
            Ok(Self(id))
        } else {
            Err(InvalidTenantId(id))
        }
    }

    /// The id as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An encoded storage key.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(Vec<u8>);

impl Key {
    /// The encoded bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// A key from its encoded bytes (from [`Key::as_bytes`]).
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The smallest key greater than this one.
    pub(crate) fn successor(&self) -> Self {
        let mut bytes = self.0.clone();
        bytes.push(0);
        Self(bytes)
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key(")?;
        for byte in &self.0 {
            if byte.is_ascii_graphic() {
                write!(f, "{}", char::from(*byte))?;
            } else {
                write!(f, "\\x{byte:02x}")?;
            }
        }
        write!(f, ")")
    }
}

/// A half-open key range `[start, end)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRange {
    /// Inclusive start.
    pub start: Key,
    /// Exclusive end.
    pub end: Key,
}

impl KeyRange {
    /// Whether the range contains `key`.
    pub fn contains(&self, key: &Key) -> bool {
        *key >= self.start && *key < self.end
    }

    /// The keys of this prefix range whose next tuple part is within the bounds. Each bound is
    /// `(value, inclusive)`.
    #[must_use]
    pub fn bounded(
        &self,
        lower: Option<(KeyPart<'_>, bool)>,
        upper: Option<(KeyPart<'_>, bool)>,
    ) -> Self {
        // `start` is the prefix itself; every key in the range extends it with more parts.
        let at = |part: KeyPart<'_>, past: bool| {
            let mut bytes = self.start.0.clone();
            part.encode_into(&mut bytes);
            if past {
                // Past every key whose next part equals `part`.
                bytes.push(ESCAPE);
            }
            Key(bytes)
        };
        let start = lower.map_or_else(
            || self.start.clone(),
            |(part, inclusive)| at(part, !inclusive),
        );
        let end = upper.map_or_else(|| self.end.clone(), |(part, inclusive)| at(part, inclusive));
        Self { start, end }
    }

    /// The overlap of two ranges (possibly empty).
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            start: self.start.clone().max(other.start.clone()),
            end: self.end.clone().min(other.end.clone()),
        }
    }

    /// An opaque cursor for resuming a scan of this range after `key`: the hex-encoded part of
    /// the key after the range's start. `None` if `key` isn't in the range.
    pub fn cursor_after(&self, key: &Key) -> Option<String> {
        if !self.contains(key) {
            return None;
        }
        let suffix = key.0.strip_prefix(self.start.0.as_slice())?;
        Some(
            suffix
                .iter()
                .fold(String::with_capacity(suffix.len() * 2), |mut out, byte| {
                    use std::fmt::Write as _;
                    let _ = write!(out, "{byte:02x}");
                    out
                }),
        )
    }

    /// The rest of this range after the key a cursor points at, in scan order. `None` if the
    /// cursor is malformed or points outside the range, so a forged cursor can never widen a
    /// scan beyond the range (or the tenant).
    pub fn resume(&self, cursor: &str, forward: bool) -> Option<Self> {
        if !cursor.len().is_multiple_of(2) || cursor.len() > 4096 {
            return None;
        }
        let mut bytes = self.start.0.clone();
        for pair in cursor.as_bytes().chunks(2) {
            let hex = std::str::from_utf8(pair).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
        }
        let key = Key(bytes);
        if !self.contains(&key) {
            return None;
        }
        Some(if forward {
            Self {
                start: key.successor(),
                end: self.end.clone(),
            }
        } else {
            Self {
                start: self.start.clone(),
                end: key,
            }
        })
    }
}

/// One part of a key tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPart<'a> {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A string.
    Str(&'a str),
    /// A decimal, as its units at scale 18.
    Decimal(i128),
}

impl<'a> KeyPart<'a> {
    /// Borrows a scalar value as a key part. Lists and objects can't be key parts.
    pub fn from_value(value: &'a Value) -> Option<Self> {
        Some(match value {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Bool(*b),
            Value::Int(i) => Self::Int(*i),
            Value::String(s) => Self::Str(s),
            Value::Decimal(d) => Self::Decimal(d.normalized()),
            Value::List(_) | Value::Object(_) => return None,
        })
    }

    fn encode_into(self, out: &mut Vec<u8>) {
        match self {
            Self::Null => out.push(NULL),
            Self::Bool(false) => out.push(FALSE),
            Self::Bool(true) => out.push(TRUE),
            Self::Int(i) => {
                out.push(INT);
                // Flipping the sign bit makes two's-complement order match unsigned byte order.
                out.extend_from_slice(&(i.cast_unsigned() ^ (1 << 63)).to_be_bytes());
            }
            Self::Decimal(units) => {
                out.push(DECIMAL);
                out.extend_from_slice(&(units.cast_unsigned() ^ (1 << 127)).to_be_bytes());
            }
            Self::Str(s) => {
                out.push(STRING);
                for &byte in s.as_bytes() {
                    out.push(byte);
                    if byte == NULL {
                        out.push(ESCAPE);
                    }
                }
                out.push(NULL);
            }
        }
    }
}

/// Builds every key for one tenant.
#[derive(Debug, Clone)]
pub struct Keyspace {
    tenant: TenantId,
    prefix: Vec<u8>,
}

impl Keyspace {
    /// A keyspace for `tenant`.
    pub fn new(tenant: &TenantId) -> Self {
        let mut prefix = Vec::new();
        KeyPart::Str(tenant.as_str()).encode_into(&mut prefix);
        Self {
            tenant: tenant.clone(),
            prefix,
        }
    }

    /// The tenant this keyspace belongs to.
    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    /// The range holding all of the tenant's keys.
    pub fn all(&self) -> KeyRange {
        self.range(&[])
    }

    /// The key of a row.
    pub fn row(&self, entity: &str, key: &[KeyPart<'_>]) -> Key {
        self.key(&[KeyPart::Str(ROWS), KeyPart::Str(entity)], key, &[])
    }

    /// The range of rows whose primary key starts with `prefix`.
    pub fn row_range(&self, entity: &str, prefix: &[KeyPart<'_>]) -> KeyRange {
        self.range(&[&[KeyPart::Str(ROWS), KeyPart::Str(entity)], prefix])
    }

    /// The key of an index entry. The primary key is appended so entries are unique.
    pub fn index_entry(
        &self,
        entity: &str,
        index: &str,
        values: &[KeyPart<'_>],
        primary_key: &[KeyPart<'_>],
    ) -> Key {
        self.key(
            &[
                KeyPart::Str(INDEXES),
                KeyPart::Str(entity),
                KeyPart::Str(index),
            ],
            values,
            primary_key,
        )
    }

    /// The range of index entries whose indexed values start with `prefix`.
    pub fn index_range(&self, entity: &str, index: &str, prefix: &[KeyPart<'_>]) -> KeyRange {
        self.range(&[
            &[
                KeyPart::Str(INDEXES),
                KeyPart::Str(entity),
                KeyPart::Str(index),
            ],
            prefix,
        ])
    }

    /// The key of an outbox entry due at `deliver_at`. Entries sort by due time.
    pub fn outbox(&self, deliver_at: i64, id: &str) -> Key {
        self.key(
            &[KeyPart::Str(OUTBOX)],
            &[KeyPart::Int(deliver_at), KeyPart::Str(id)],
            &[],
        )
    }

    /// The range of outbox entries due at or before `now`.
    pub fn outbox_due(&self, now: i64) -> KeyRange {
        let all = self.range(&[&[KeyPart::Str(OUTBOX)]]);
        let end = self.key(
            &[KeyPart::Str(OUTBOX)],
            &[KeyPart::Int(now.saturating_add(1))],
            &[],
        );
        KeyRange {
            start: all.start,
            end,
        }
    }

    /// The key of a schedule's state (the last tick a cron fired at).
    pub fn schedule(&self, handler: &str) -> Key {
        self.key(&[KeyPart::Str(SCHEDULES)], &[KeyPart::Str(handler)], &[])
    }

    /// The key of a stored idempotent response.
    /// The key of a user's profile in the tenant's user directory.
    pub fn user(&self, user_id: &str) -> Key {
        self.key(&[KeyPart::Str(USERS)], &[KeyPart::Str(user_id)], &[])
    }

    /// The key of a keyed job's pointer to its outbox entry.
    pub fn job(&self, queue: &str, job_key: &str) -> Key {
        self.key(
            &[KeyPart::Str(JOBS), KeyPart::Str(queue)],
            &[KeyPart::Str(job_key)],
            &[],
        )
    }

    /// The key of the idempotency record for `key` on `route`.
    pub fn idempotency(&self, route: &str, key: &str) -> Key {
        self.key(
            &[KeyPart::Str(IDEMPOTENCY), KeyPart::Str(route)],
            &[KeyPart::Str(key)],
            &[],
        )
    }

    /// The key of one counter group.
    pub fn counter(&self, counter: &str, group: &[KeyPart<'_>]) -> Key {
        self.key(&[KeyPart::Str(COUNTERS), KeyPart::Str(counter)], group, &[])
    }

    /// The range of a counter's keys that extend `group` (aggregate entries).
    pub fn counter_range(&self, counter: &str, group: &[KeyPart<'_>]) -> KeyRange {
        self.range(&[&[KeyPart::Str(COUNTERS), KeyPart::Str(counter)], group])
    }

    fn key(&self, head: &[KeyPart<'_>], middle: &[KeyPart<'_>], tail: &[KeyPart<'_>]) -> Key {
        let mut out = self.prefix.clone();
        for part in head.iter().chain(middle).chain(tail) {
            part.encode_into(&mut out);
        }
        Key(out)
    }

    fn range(&self, groups: &[&[KeyPart<'_>]]) -> KeyRange {
        let mut start = self.prefix.clone();
        for part in groups.iter().copied().flatten() {
            part.encode_into(&mut start);
        }
        // Every encoded part starts with a type tag below 0xFF, so `prefix ++ 0xFF` is greater
        // than every key extending the prefix.
        let mut end = start.clone();
        end.push(ESCAPE);
        KeyRange {
            start: Key(start),
            end: Key(end),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn tenant(id: &str) -> Keyspace {
        Keyspace::new(&TenantId::new(id).unwrap_or_else(|e| panic!("{e}")))
    }

    #[test]
    fn rejects_invalid_tenant_ids() {
        assert!(TenantId::new("").is_err());
        assert!(TenantId::new("a/b").is_err());
        assert!(TenantId::new("x".repeat(65)).is_err());
        assert!(TenantId::new("acme-prod_1").is_ok());
    }

    #[test]
    fn tenants_never_share_keys() {
        let a = tenant("a");
        let ab = tenant("ab");
        // "a" is a string prefix of "ab", but the terminator keeps the keyspaces disjoint.
        assert!(!ab.all().contains(&a.row("E", &[KeyPart::Int(1)])));
        assert!(!a.all().contains(&ab.row("E", &[KeyPart::Int(1)])));
    }

    proptest! {
        #[test]
        fn int_order_is_preserved(a: i64, b: i64) {
            let ks = tenant("t");
            let ka = ks.row("E", &[KeyPart::Int(a)]);
            let kb = ks.row("E", &[KeyPart::Int(b)]);
            prop_assert_eq!(a.cmp(&b), ka.cmp(&kb));
        }

        #[test]
        fn string_order_is_preserved(a in ".{0,12}", b in ".{0,12}") {
            let ks = tenant("t");
            let ka = ks.row("E", &[KeyPart::Str(&a)]);
            let kb = ks.row("E", &[KeyPart::Str(&b)]);
            prop_assert_eq!(a.as_bytes().cmp(b.as_bytes()), ka.cmp(&kb));
        }

        #[test]
        fn prefix_ranges_contain_exactly_their_extensions(
            prefix in ".{0,6}", other in ".{0,6}", suffix: i64
        ) {
            let ks = tenant("t");
            let range = ks.row_range("E", &[KeyPart::Str(&prefix)]);
            let key = ks.row("E", &[KeyPart::Str(&other), KeyPart::Int(suffix)]);
            prop_assert_eq!(range.contains(&key), prefix == other);
        }

        #[test]
        fn cursors_round_trip_and_never_escape_their_range(
            prefix in "[a-z]{1,4}", a in 0i64..1000, cursor in "[0-9a-f]{0,40}"
        ) {
            let ks = tenant("t");
            let range = ks.row_range("E", &[KeyPart::Str(&prefix)]);
            let key = ks.row("E", &[KeyPart::Str(&prefix), KeyPart::Int(a)]);
            let token = range.cursor_after(&key).unwrap();
            let rest = range.resume(&token, true).unwrap();
            prop_assert!(!rest.contains(&key) && rest.contains(&ks.row("E", &[KeyPart::Str(&prefix), KeyPart::Int(a + 1)])));
            // Any cursor, including forged ones, resumes inside the original range or is refused.
            if let Some(resumed) = range.resume(&cursor, true) {
                prop_assert!(resumed.start >= range.start && resumed.end <= range.end);
            }
            if let Some(resumed) = range.resume(&cursor, false) {
                prop_assert!(resumed.start >= range.start && resumed.end <= range.end);
            }
        }

        #[test]
        fn bounded_ranges_match_comparisons(prefix in "[a-z]{1,3}", lo: i64, hi: i64, v: i64, lo_inc: bool, hi_inc: bool) {
            let ks = tenant("t");
            let range = ks.row_range("E", &[KeyPart::Str(&prefix)])
                .bounded(Some((KeyPart::Int(lo), lo_inc)), Some((KeyPart::Int(hi), hi_inc)));
            let key = ks.row("E", &[KeyPart::Str(&prefix), KeyPart::Int(v), KeyPart::Str("pk")]);
            let above = if lo_inc { v >= lo } else { v > lo };
            let below = if hi_inc { v <= hi } else { v < hi };
            prop_assert_eq!(range.contains(&key), above && below);
        }

        #[test]
        fn every_key_is_inside_its_tenant(id in "[a-z0-9_-]{1,10}", s in ".{0,8}", i: i64) {
            let ks = tenant(&id);
            let all = ks.all();
            prop_assert!(all.contains(&ks.row("E", &[KeyPart::Str(&s)])));
            prop_assert!(all.contains(&ks.index_entry("E", "idx", &[KeyPart::Int(i)], &[KeyPart::Str(&s)])));
            prop_assert!(all.contains(&ks.counter("c", &[KeyPart::Bool(true)])));
        }
    }
}
