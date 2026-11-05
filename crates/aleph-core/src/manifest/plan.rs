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
    },
    /// Fail with a declared error.
    Fail {
        /// The declared error.
        error: String,
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

/// An operation. Operations are the only way a plan touches storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(tag = "op", rename_all = "camelCase")]
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
    /// Update fields of an existing row. Evaluates to the updated row.
    Update {
        /// The entity.
        entity: String,
        /// A value for every primary key field.
        key: BTreeMap<String, Expr>,
        /// New field values. Key fields can't be changed.
        set: BTreeMap<String, Expr>,
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
        /// Maximum number of rows. Must have a statically known upper bound.
        limit: Box<Expr>,
        /// Scan order.
        #[serde(default, skip_serializing_if = "Order::is_asc")]
        #[cfg_attr(feature = "codegen", ts(as = "Option<Order>", optional))]
        order: Order,
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
    /// Evaluate a pure expression.
    Eval {
        /// The expression.
        expr: Expr,
    },
}

impl Op {
    /// Whether the operation writes to storage.
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Self::Insert { .. } | Self::Update { .. } | Self::Delete { .. }
        )
    }
}
