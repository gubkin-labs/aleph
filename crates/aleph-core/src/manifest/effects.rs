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
    /// Allows the handler to be part of a trigger cycle (it writes, emits, or enqueues what
    /// eventually triggers it again), running at most this many handler hops after the request
    /// or schedule that started the chain (1 to `MAX_CHAIN_DEPTH`). Without it, a cycle is a
    /// validation error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub max_depth: Option<u32>,
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
/// - `workflow`: `instanceId`, `input`, `state`, `signals` (the signal that resumed the step, by
///   name; the others are `null`), and `error` (in a compensation step)
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
    /// One step of a workflow. It ends by moving to another step (`next`), waiting for a
    /// signal (`waitFor`), or finishing (`complete`); a step that ends without one completes.
    Workflow {
        /// The workflow.
        workflow: String,
        /// The step's name.
        step: String,
        /// The step to run if this one fails every retry (a compensating action). Without one,
        /// the workflow fails.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        on_fail: Option<String>,
    },
    /// A schedule: runs at every multiple of `every_ms` (plus `offset_ms`) since the epoch.
    Cron {
        /// Interval, in milliseconds (1 second to 30 days). Ignored (and 0) with `schedule`.
        #[serde(default)]
        every_ms: u32,
        /// Offset into each interval, in milliseconds.
        #[serde(default)]
        offset_ms: u32,
        /// Run once per batch of an entity's rows instead of once per tick. The platform
        /// paginates: each batch is a separate, separately-bounded execution.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        scan: Option<CronScan>,
        /// A calendar schedule instead of an interval.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        schedule: Option<CronSchedule>,
    },
}

/// A durable workflow: steps that run one after another, each its own bounded transaction, with
/// typed state carried between them, waits for signals with timeouts, delays, retries, and
/// compensation. Instances are identified by a key the starter chooses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowDecl {
    /// What a workflow is started with (read-only afterwards).
    pub input: BTreeMap<String, Field>,
    /// State carried between steps. Every field is optional: it starts `null`, and steps set it.
    #[serde(default)]
    pub state: BTreeMap<String, Field>,
    /// Signals the workflow can wait for, with their payloads.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(
        feature = "codegen",
        ts(as = "Option<BTreeMap<String, BTreeMap<String, Field>>>", optional)
    )]
    pub signals: BTreeMap<String, BTreeMap<String, Field>>,
    /// The first step.
    pub start: String,
    /// The most steps one instance may run (1-1000), so every instance finishes.
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    /// Retries of a failed step (at most 10), with exponential backoff.
    #[serde(default = "default_retries")]
    pub retries: u32,
    /// Delay before a step's first retry, in milliseconds.
    #[serde(default = "default_backoff")]
    pub backoff_ms: u32,
}

fn default_max_steps() -> u32 {
    100
}

/// A calendar schedule: a five-field cron expression in a time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CronSchedule {
    /// `minute hour day-of-month month weekday`, like `0 9 * * mon`.
    pub cron: String,
    /// An IANA time zone (default UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub tz: Option<String>,
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
