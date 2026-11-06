//! Execution budgets and the cost vector that static analysis and the runtime meter both measure.

use serde::{Deserialize, Serialize};

/// Per-execution limits. Static analysis rejects plans whose worst case exceeds them, and the
/// runtime meter stops any execution that exceeds them anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Maximum rows, index entries, and counters read.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub reads: u64,
    /// Maximum rows, index entries, and counters written.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub writes: u64,
    /// Maximum plan operations evaluated.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub ops: u64,
    /// Maximum bytes read from storage.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub bytes_read: u64,
    /// Maximum effects (events, jobs, messages, sends, change triggers) released.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub effects: u64,
    /// Maximum integration calls.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub calls: u64,
    /// Maximum bytes of values a plan binds (rows read, lists built, results), summed.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub memory: u64,
    /// Maximum encoded size of a single row.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub max_row_bytes: u64,
    /// How many times a conflicting transaction is re-run before giving up.
    pub max_conflict_retries: u32,
}

impl Limits {
    /// The v1 limits for routes (see `guardrails.md` in the spec).
    pub const ROUTE: Self = Self {
        reads: 250,
        writes: 50,
        ops: 2_000,
        bytes_read: 2 * 1024 * 1024,
        effects: 50,
        calls: 3,
        memory: 8 * 1024 * 1024,
        max_row_bytes: 64 * 1024,
        max_conflict_retries: 5,
    };

    /// The v1 limits for handlers: queue jobs, triggers, and cron runs (one scan batch each).
    pub const JOB: Self = Self {
        reads: 500,
        writes: 200,
        ops: 5_000,
        bytes_read: 8 * 1024 * 1024,
        effects: 200,
        calls: 5,
        memory: 32 * 1024 * 1024,
        max_row_bytes: 64 * 1024,
        max_conflict_retries: 5,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::ROUTE
    }
}

/// A resource that executions consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum Resource {
    Reads,
    Writes,
    Ops,
    BytesRead,
    Effects,
    Calls,
    Memory,
}

impl Resource {
    /// All resources, in reporting order.
    pub const ALL: [Self; 7] = [
        Self::Reads,
        Self::Writes,
        Self::Ops,
        Self::BytesRead,
        Self::Effects,
        Self::Calls,
        Self::Memory,
    ];

    /// The resource's limit.
    pub fn limit(self, limits: &Limits) -> u64 {
        match self {
            Self::Reads => limits.reads,
            Self::Writes => limits.writes,
            Self::Ops => limits.ops,
            Self::BytesRead => limits.bytes_read,
            Self::Effects => limits.effects,
            Self::Calls => limits.calls,
            Self::Memory => limits.memory,
        }
    }
}

impl std::fmt::Display for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Reads => "reads",
            Self::Writes => "writes",
            Self::Ops => "ops",
            Self::BytesRead => "bytes read",
            Self::Effects => "effects",
            Self::Calls => "integration calls",
            Self::Memory => "bytes of memory",
        })
    }
}

/// An amount of each resource. Arithmetic saturates, so worst-case bounds never overflow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    /// Rows, index entries, and counters read.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub reads: u64,
    /// Rows, index entries, and counters written.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub writes: u64,
    /// Plan operations evaluated.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub ops: u64,
    /// Bytes read from storage.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub bytes_read: u64,
    /// Effects released.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub effects: u64,
    /// Integration calls.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub calls: u64,
    /// Bytes of values bound while running (see [`Limits::memory`]).
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub memory: u64,
}

impl Cost {
    /// The cost of evaluating `n` plan operations.
    pub const fn ops(n: u64) -> Self {
        Self {
            ops: n,
            ..Self::ZERO
        }
    }

    /// No cost.
    pub const ZERO: Self = Self {
        reads: 0,
        writes: 0,
        ops: 0,
        bytes_read: 0,
        effects: 0,
        calls: 0,
        memory: 0,
    };

    /// The amount of one resource.
    pub fn get(&self, resource: Resource) -> u64 {
        match resource {
            Resource::Reads => self.reads,
            Resource::Writes => self.writes,
            Resource::Ops => self.ops,
            Resource::BytesRead => self.bytes_read,
            Resource::Effects => self.effects,
            Resource::Calls => self.calls,
            Resource::Memory => self.memory,
        }
    }

    /// Component-wise saturating sum.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self {
            reads: self.reads.saturating_add(other.reads),
            writes: self.writes.saturating_add(other.writes),
            ops: self.ops.saturating_add(other.ops),
            bytes_read: self.bytes_read.saturating_add(other.bytes_read),
            effects: self.effects.saturating_add(other.effects),
            calls: self.calls.saturating_add(other.calls),
            memory: self.memory.saturating_add(other.memory),
        }
    }

    /// Component-wise maximum (the worst of two branches).
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self {
            reads: self.reads.max(other.reads),
            writes: self.writes.max(other.writes),
            ops: self.ops.max(other.ops),
            bytes_read: self.bytes_read.max(other.bytes_read),
            effects: self.effects.max(other.effects),
            calls: self.calls.max(other.calls),
            memory: self.memory.max(other.memory),
        }
    }

    /// Component-wise saturating multiplication (a body run `n` times).
    #[must_use]
    pub fn times(self, n: u64) -> Self {
        Self {
            reads: self.reads.saturating_mul(n),
            writes: self.writes.saturating_mul(n),
            ops: self.ops.saturating_mul(n),
            bytes_read: self.bytes_read.saturating_mul(n),
            effects: self.effects.saturating_mul(n),
            calls: self.calls.saturating_mul(n),
            memory: self.memory.saturating_mul(n),
        }
    }

    /// Whether every component is at most the other's.
    pub fn fits_within(&self, other: &Self) -> bool {
        Resource::ALL.iter().all(|r| self.get(*r) <= other.get(*r))
    }
}

impl std::ops::Add for Cost {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        self.plus(rhs)
    }
}

impl std::ops::AddAssign for Cost {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.plus(rhs);
    }
}
