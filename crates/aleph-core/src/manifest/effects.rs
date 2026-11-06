//! Asynchronous work: events, queues, channels, integrations, and the handlers that react to
//! them. Every effect a plan produces is written to a transactional outbox at commit, so a
//! rolled-back or retried attempt never releases it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{ErrorDecl, Field, Literal, Plan};

/// A declared event with a typed payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventDecl {
    /// Payload fields.
    pub payload: BTreeMap<String, Field>,
}

/// A declared queue: jobs with a typed payload, retried with exponential backoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueueDecl {
    /// Payload fields.
    pub payload: BTreeMap<String, Field>,
    /// How many times a failed job is retried (at most 10).
    #[serde(default = "default_retries")]
    pub retries: u32,
    /// Delay before the first retry, in milliseconds. Doubles on each retry.
    #[serde(default = "default_backoff")]
    pub backoff_ms: u32,
    /// The longest delay between retries, in milliseconds.
    #[serde(default = "default_max_backoff")]
    pub max_backoff_ms: u32,
}

fn default_retries() -> u32 {
    3
}
fn default_backoff() -> u32 {
    1_000
}
fn default_max_backoff() -> u32 {
    60_000
}

impl QueueDecl {
    /// The most retries a queue may declare.
    pub const MAX_RETRIES: u32 = 10;

    /// Retry settings for event and entity handlers, which have no queue declaration.
    pub const HANDLER_DEFAULTS: (u32, u32, u32) = (3, 1_000, 60_000);
}

/// A realtime channel: clients subscribe by key, plans publish messages to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelDecl {
    /// Fields that identify one stream of the channel, for example `{ roomId }`.
    pub key: BTreeMap<String, Field>,
    /// Message fields.
    pub message: BTreeMap<String, Field>,
}

/// A declared integration: the only way plans reach the outside world.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Integration {
    /// Which provider implements it, for example `"http"`, `"email"`, `"stripe"`.
    pub provider: String,
    /// Provider configuration (base URL, secret names, ...), interpreted by the platform.
    #[serde(default)]
    #[cfg_attr(
        feature = "codegen",
        ts(as = "Option<BTreeMap<String, String>>", optional)
    )]
    pub config: BTreeMap<String, String>,
    /// Operations, by name.
    pub operations: BTreeMap<String, IntegrationOp>,
}

/// How an integration operation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum OpMode {
    /// Runs during the plan and returns data. Journaled: a retried attempt reuses the result.
    Call,
    /// Fire-and-forget, delivered through the outbox after commit.
    Effect,
}

/// One integration operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegrationOp {
    /// Call or effect.
    pub mode: OpMode,
    /// Input fields.
    pub input: BTreeMap<String, Field>,
    /// Output fields (calls only). The response is validated against them.
    #[serde(default)]
    #[cfg_attr(
        feature = "codegen",
        ts(as = "Option<BTreeMap<String, Field>>", optional)
    )]
    pub output: BTreeMap<String, Field>,
}

/// A handler: a plan run asynchronously when something happens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Handler {
    /// Stable identifier, unique among handlers.
    pub id: String,
    /// What runs it.
    pub trigger: Trigger,
    /// Declared errors. Failing with one counts as a failed attempt, which is retried.
    #[serde(default)]
    #[cfg_attr(
        feature = "codegen",
        ts(as = "Option<BTreeMap<String, ErrorDecl>>", optional)
    )]
    pub errors: BTreeMap<String, ErrorDecl>,
    /// The handler's logic. Its output is ignored.
    pub plan: Plan,
}

/// A change to a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum Change {
    Inserted,
    Updated,
    Deleted,
}

/// What runs a handler. The handler reads its payload with `input` expressions of source
/// `payload`:
/// - `event`, `queue`: the declared payload fields
/// - `deadLetter`: the job's payload fields, plus `error` and `attempts`
/// - `entity`: `before` and `after` (rows, `null` when absent)
/// - `cron`: `firedAt`, plus `batch` (a list of rows) when it scans
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[serde(deny_unknown_fields)]
pub enum Trigger {
    /// An emitted event.
    Event {
        /// The event.
        event: String,
    },
    /// A committed change to a row.
    Entity {
        /// The entity.
        entity: String,
        /// Which change.
        change: Change,
    },
    /// A job on a queue. At most one handler per queue.
    Queue {
        /// The queue.
        queue: String,
    },
    /// A job that failed every retry.
    DeadLetter {
        /// The queue.
        queue: String,
    },
    /// A schedule: runs at every multiple of `every_ms` (plus `offset_ms`) since the epoch.
    Cron {
        /// Interval, in milliseconds (1 second to 30 days).
        every_ms: u32,
        /// Offset into each interval, in milliseconds.
        #[serde(default)]
        offset_ms: u32,
        /// Run once per batch of an entity's rows instead of once per tick. The platform
        /// paginates: each batch is a separate, separately-bounded execution.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        scan: Option<CronScan>,
    },
}

/// The rows a scanning cron walks through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CronScan {
    /// The entity.
    pub entity: String,
    /// Scan this index instead of the primary key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub index: Option<String>,
    /// Constant values for a prefix of the scanned fields.
    #[serde(default)]
    pub prefix: Vec<Literal>,
    /// Rows per batch (1-500).
    pub batch: u32,
}

impl CronScan {
    /// The largest batch a scan may declare.
    pub const MAX_BATCH: u32 = 500;
}
