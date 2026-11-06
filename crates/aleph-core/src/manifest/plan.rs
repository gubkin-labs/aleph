//! Plans: statements and operations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Expr;

/// A variable identifier, unique within a plan.
pub type VarId = u32;

/// A route's logic: statements followed by the value returned to the caller.
///
/// A plan runs as **one optimistic transaction**. A statement that fails rolls it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    /// Statements, executed in order.
    pub body: Vec<Stmt>,
    /// The response body.
    pub output: Expr,
}

/// A statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub enum Stmt {
    /// Run an operation and bind its result to a new variable.
    Let {
        /// The variable to bind. Must not already be defined.
        var: VarId,
        /// The operation.
        op: Op,
    },
    /// Fail with a declared error unless `cond` is true.
    Ensure {
        /// The condition that must hold.
        cond: Expr,
        /// The declared error to fail with.
        error: String,
        /// Values for the error's declared `details` fields.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        #[cfg_attr(
            feature = "codegen",
            ts(as = "Option<BTreeMap<String, Expr>>", optional)
        )]
        details: BTreeMap<String, Expr>,
    },
    /// Fail with a declared error.
    Fail {
        /// The declared error.
        error: String,
        /// Values for the error's declared `details` fields.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        #[cfg_attr(
            feature = "codegen",
            ts(as = "Option<BTreeMap<String, Expr>>", optional)
        )]
        details: BTreeMap<String, Expr>,
    },
    /// Finish successfully now, responding with `value` (routes) or ending the run (handlers).
    /// The transaction commits.
    Return {
        /// The response body.
        value: Expr,
    },
    /// Set a response header. `set-cookie` adds a cookie each time; other headers are replaced.
    SetHeader {
        /// The header name, lowercase.
        name: String,
        /// The value (a string).
        value: Expr,
    },
    /// Set the success status (200-399; the default is 200).
    SetStatus {
        /// The status.
        status: u16,
    },
    /// Record a value for debugging. Logs are returned with the response in development and
    /// sampled in production; they never change what the plan does.
    Log {
        /// A short label.
        label: String,
        /// The value logged.
        value: Expr,
    },
    /// Conditional execution. Variables defined inside a branch aren't visible after it.
    If {
        /// The condition.
        cond: Expr,
        /// Statements run when `cond` is true.
        then: Vec<Stmt>,
        /// Statements run when `cond` is false.
        #[serde(rename = "else", default)]
        otherwise: Vec<Stmt>,
    },
    /// Run `body` once per item of a list with a statically bounded length.
    ForEach {
        /// The list.
        list: Expr,
        /// The variable each item is bound to.
        item: VarId,
        /// Statements run per item.
        body: Vec<Stmt>,
    },
}

/// Scan order for queries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Order {
    /// Ascending key order.
    #[default]
    Asc,
    /// Descending key order.
    Desc,
}

impl Order {
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde passes by reference
    fn is_asc(&self) -> bool {
        matches!(self, Self::Asc)
    }
}

/// Bounds on the field after a query's prefix. At most one lower and one upper bound. Like SQL
/// comparisons, a range never matches rows where that field is `null`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryRange {
    /// Greater than.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub gt: Option<Expr>,
    /// Greater than or equal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub gte: Option<Expr>,
    /// Less than.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub lt: Option<Expr>,
    /// Less than or equal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub lte: Option<Expr>,
}

/// An operation. Operations are the only way a plan touches storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[serde(deny_unknown_fields)]
pub enum Op {
    /// Read one row by its full primary key. Evaluates to the row or `null`.
    Get {
        /// The entity.
        entity: String,
        /// A value for every primary key field.
        key: BTreeMap<String, Expr>,
    },
    /// Insert a new row. Fails if a row with the same key exists. Evaluates to the row.
    Insert {
        /// The entity.
        entity: String,
        /// Field values. Missing fields take their default, or `null` if optional.
        values: BTreeMap<String, Expr>,
    },
    /// Insert a row, or update it if one with the same key exists. Evaluates to the row.
    Upsert {
        /// The entity.
        entity: String,
        /// Field values. On insert, missing fields take their default. On update, missing fields
        /// keep their value.
        values: BTreeMap<String, Expr>,
    },
    /// Insert a row unless one with the same key exists. Evaluates to whether it inserted.
    InsertIfAbsent {
        /// The entity.
        entity: String,
        /// Field values, as for `insert`.
        values: BTreeMap<String, Expr>,
    },
    /// Update fields of an existing row. Evaluates to the updated row.
    Update {
        /// The entity.
        entity: String,
        /// A value for every primary key field.
        key: BTreeMap<String, Expr>,
        /// New field values. Key fields can't be changed.
        set: BTreeMap<String, Expr>,
        /// For versioned entities: fail with 409 unless the row has this version.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        if_version: Option<Box<Expr>>,
    },
    /// Delete a row if it exists. Evaluates to `null`.
    Delete {
        /// The entity.
        entity: String,
        /// A value for every primary key field.
        key: BTreeMap<String, Expr>,
    },
    /// Read a bounded, ordered range of rows. Evaluates to a list of rows.
    Query {
        /// The entity.
        entity: String,
        /// Scan this secondary index instead of the primary key.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        index: Option<String>,
        /// Values for a prefix of the key (or index) fields.
        #[serde(default)]
        prefix: Vec<Expr>,
        /// Values for the field after the prefix: rows matching any of them are merged in key
        /// order (at most 10 values). Can't be combined with `after` or `page`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        any_of: Option<Vec<Expr>>,
        /// Maximum number of rows. Must have a statically known upper bound.
        limit: Box<Expr>,
        /// Scan order.
        #[serde(default, skip_serializing_if = "Order::is_asc")]
        #[cfg_attr(feature = "codegen", ts(as = "Option<Order>", optional))]
        order: Order,
        /// Bounds on the scanned field right after the prefix.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        range: Option<Box<QueryRange>>,
        /// Continue after this cursor (a string from a previous page, or `null` for the start).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        after: Option<Box<Expr>>,
        /// Evaluate to `{ items, cursor }` instead of a plain list. `cursor` is `null` when the
        /// page wasn't full.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "codegen", ts(as = "Option<bool>", optional))]
        page: bool,
    },
    /// Read a counter value. Evaluates to an integer (0 if never incremented).
    CounterGet {
        /// The counter.
        counter: String,
        /// Values for the counter's `groupBy` fields, in order.
        key: Vec<Expr>,
    },
    /// Transform each item of a bounded list. The body may read but never write.
    Map {
        /// The list.
        list: Expr,
        /// The variable each item is bound to.
        item: VarId,
        /// Read-only statements run per item.
        #[serde(default)]
        body: Vec<Stmt>,
        /// The new item.
        #[serde(rename = "yield")]
        result: Expr,
    },
    /// Keep the items of a bounded list for which `keep` is true. The body may read but never
    /// write.
    Filter {
        /// The list.
        list: Expr,
        /// The variable each item is bound to.
        item: VarId,
        /// Read-only statements run per item.
        #[serde(default)]
        body: Vec<Stmt>,
        /// The predicate.
        keep: Expr,
    },
    /// Emit an event. Released after commit.
    Emit {
        /// The event.
        event: String,
        /// Payload values.
        payload: BTreeMap<String, Expr>,
    },
    /// Put a job on a queue, optionally delayed (at most 30 days). Released after commit.
    Enqueue {
        /// The queue.
        queue: String,
        /// Payload values.
        payload: BTreeMap<String, Expr>,
        /// Delay in milliseconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        delay_ms: Option<Box<Expr>>,
        /// A key for the job (a string). A pending job with the same key is replaced, and
        /// `cancel` with the key removes it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        key: Option<Box<Expr>>,
    },
    /// Cancel the pending job with this key, if any. Released after commit.
    Cancel {
        /// The queue.
        queue: String,
        /// The job's key.
        key: Box<Expr>,
    },
    /// Read a user's profile from the tenant's user directory. Evaluates to
    /// `{ userId, ...profile fields }`, or `null` if there's no such user.
    GetUser {
        /// The user id.
        user_id: Box<Expr>,
    },
    /// Publish a message to a realtime channel. Released after commit.
    Publish {
        /// The channel.
        channel: String,
        /// Values for the channel's key fields.
        key: BTreeMap<String, Expr>,
        /// Message values.
        message: BTreeMap<String, Expr>,
    },
    /// Run an integration call and evaluate to its (validated) output. Journaled.
    Call {
        /// The integration.
        integration: String,
        /// The operation (mode `call`).
        operation: String,
        /// Input values.
        input: BTreeMap<String, Expr>,
    },
    /// Send an integration effect. Released after commit.
    Send {
        /// The integration.
        integration: String,
        /// The operation (mode `effect`).
        operation: String,
        /// Input values.
        input: BTreeMap<String, Expr>,
    },
    /// Evaluate a pure expression.
    Eval {
        /// The expression.
        expr: Expr,
    },
}

impl Op {
    /// Whether the operation produces an effect released after commit.
    pub fn is_effect(&self) -> bool {
        matches!(
            self,
            Self::Emit { .. }
                | Self::Enqueue { .. }
                | Self::Cancel { .. }
                | Self::Publish { .. }
                | Self::Send { .. }
        )
    }

    /// Whether the operation writes to storage.
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Self::Insert { .. }
                | Self::Upsert { .. }
                | Self::InsertIfAbsent { .. }
                | Self::Update { .. }
                | Self::Delete { .. }
        )
    }
}
