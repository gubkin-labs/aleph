//! Expressions: pure computations over values. Expressions never touch storage.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::VarId;

/// A constant value in a manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(untagged)]
pub enum Literal {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(#[cfg_attr(feature = "codegen", ts(type = "number"))] i64),
    /// A string.
    String(String),
}

/// Where an input value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum InputSource {
    Params,
    Query,
    Body,
    /// A handler's payload.
    Payload,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum BinaryOp {
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

impl BinaryOp {
    /// Whether the operator compares two values and yields a boolean.
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Eq | Self::Neq | Self::Lt | Self::Lte | Self::Gt | Self::Gte
        )
    }
}

/// String transformations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum StrOp {
    Lower,
    Upper,
    Trim,
}

/// String tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum StrTest {
    StartsWith,
    EndsWith,
    Contains,
}

/// An expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub enum Expr {
    /// A constant.
    Lit {
        /// The value.
        value: Literal,
    },
    /// A variable.
    Var {
        /// The variable.
        id: VarId,
    },
    /// A validated route input.
    Input {
        /// Params, query, or body.
        source: InputSource,
        /// The input name.
        name: String,
    },
    /// The signed-in caller's user id, or `null`.
    Caller,
    /// The caller's role in an org (a declared role), or `null` if they aren't a member.
    CallerRole {
        /// The org id.
        org: Box<Expr>,
    },
    /// The current time in milliseconds.
    Now,
    /// A field of a row or object.
    Field {
        /// The row or object.
        of: Box<Expr>,
        /// The field name.
        name: String,
    },
    /// `true` if the value is `null`.
    IsNull {
        /// The value.
        of: Box<Expr>,
    },
    /// Boolean negation.
    Not {
        /// The operand.
        arg: Box<Expr>,
    },
    /// Integer negation.
    Neg {
        /// The operand.
        arg: Box<Expr>,
    },
    /// A binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// `true` if the value equals one of the constants.
    In {
        /// The value.
        value: Box<Expr>,
        /// The constants.
        list: Vec<Literal>,
    },
    /// Choose between two values.
    Select {
        /// The condition.
        cond: Box<Expr>,
        /// Value when `cond` is true.
        then: Box<Expr>,
        /// Value when `cond` is false.
        #[serde(rename = "else")]
        otherwise: Box<Expr>,
    },
    /// Choose a value by matching a string. `_` is the default case.
    Match {
        /// The value matched.
        on: Box<Expr>,
        /// Cases, by value.
        cases: BTreeMap<String, Expr>,
    },
    /// An object built from named values.
    Object {
        /// The fields.
        fields: BTreeMap<String, Expr>,
    },
    /// A list built from values.
    List {
        /// The items.
        items: Vec<Expr>,
    },
    /// The length of a list or string.
    Length {
        /// The list or string.
        of: Box<Expr>,
    },
    /// A string transformation.
    Str {
        /// The transformation.
        op: StrOp,
        /// The string.
        of: Box<Expr>,
    },
    /// A string test, such as `startsWith`.
    StrTest {
        /// The test.
        op: StrTest,
        /// The string tested.
        of: Box<Expr>,
        /// The other string.
        arg: Box<Expr>,
    },
    /// The first `max` characters of a string.
    Truncate {
        /// The string.
        of: Box<Expr>,
        /// Maximum length in characters.
        max: u32,
    },
    /// Absolute value of an integer.
    Abs {
        /// The operand.
        arg: Box<Expr>,
    },
    /// String concatenation. Non-string parts are formatted.
    Concat {
        /// The parts.
        parts: Vec<Expr>,
    },
}
