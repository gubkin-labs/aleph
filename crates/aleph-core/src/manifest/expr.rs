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
    /// Remainder with the sign of the divisor's absolute value (always `0 <= r < |b|`).
    Mod,
    /// Integer power. The exponent must be 0-63.
    Pow,
    /// Division rounding toward negative infinity.
    DivFloor,
    /// Division rounding toward positive infinity.
    DivCeil,
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

/// List aggregations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum AggOp {
    /// The sum of the values (0 for an empty list).
    Sum,
    /// The smallest value, or `null` for an empty list.
    Min,
    /// The largest value, or `null` for an empty list.
    Max,
    /// `true` if the predicate holds for any item.
    Some,
    /// `true` if the predicate holds for every item.
    Every,
    /// How many items the predicate holds for (or all items, without one).
    Count,
}

/// How to round a decimal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Rounding {
    /// To the nearest value; ties to the even neighbor (banker's rounding).
    #[default]
    HalfEven,
    /// To the nearest value; ties away from zero.
    HalfUp,
    /// Toward zero.
    Down,
    /// Away from zero.
    Up,
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceil,
}

impl Rounding {
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde passes by reference
    fn is_default(&self) -> bool {
        matches!(self, Self::HalfEven)
    }
}

/// Calendar units for time arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum TimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    /// ISO weeks, starting on Monday.
    Week,
    Month,
    Year,
}

/// Parts of a point in time, in a time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum TimePart {
    Year,
    /// 1-12.
    Month,
    /// Day of the month, 1-31.
    Day,
    /// 0-23.
    Hour,
    Minute,
    Second,
    /// ISO weekday: 1 is Monday, 7 is Sunday.
    Weekday,
    /// 1-366.
    DayOfYear,
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
    /// A decimal constant, like `"0.08"`.
    Decimal {
        /// The value, as decimal text.
        value: String,
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
    /// A field of the caller's profile (declared in `auth.profile`), or `null`.
    CallerProfile {
        /// The profile field.
        field: String,
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
    /// The first item of a list, or `null` if it's empty.
    First {
        /// The list.
        of: Box<Expr>,
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
    /// Aggregates a bounded list. `value` is evaluated per item with the item bound to `item`:
    /// the summed or compared value, or the predicate for `some`, `every`, and `count`. Without
    /// it, the items themselves are used.
    Agg {
        /// The aggregation.
        op: AggOp,
        /// The list.
        list: Box<Expr>,
        /// The variable each item is bound to.
        item: VarId,
        /// The per-item value or predicate.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        value: Option<Box<Expr>>,
    },
    /// The items of a bounded list sorted by a key (stable).
    Sort {
        /// The list.
        list: Box<Expr>,
        /// The variable each item is bound to.
        item: VarId,
        /// The sort key: a string, int, decimal, or bool.
        by: Box<Expr>,
        /// Largest first.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "codegen", ts(as = "Option<bool>", optional))]
        desc: bool,
    },
    /// Items (of a list) or characters (of a string) from `start` up to `end`, like JavaScript's
    /// `slice`: negative positions count from the end.
    Slice {
        /// The list or string.
        of: Box<Expr>,
        /// The first position.
        start: Box<Expr>,
        /// The position after the last, or the end.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        end: Option<Box<Expr>>,
    },
    /// The list without repeated items (first occurrences kept, in order).
    Distinct {
        /// The list.
        of: Box<Expr>,
    },
    /// The items of several lists, in order.
    ListConcat {
        /// The lists.
        parts: Vec<Expr>,
    },
    /// `true` if the list contains the value.
    Includes {
        /// The list.
        list: Box<Expr>,
        /// The value.
        value: Box<Expr>,
    },
    /// Splits a string by a separator into at most `max` parts; the last part keeps the rest.
    Split {
        /// The string.
        of: Box<Expr>,
        /// The separator (not empty).
        sep: Box<Expr>,
        /// Maximum number of parts.
        max: u32,
    },
    /// Replaces every occurrence of `find` (not empty) with `with`.
    Replace {
        /// The string.
        of: Box<Expr>,
        /// The text to find.
        find: Box<Expr>,
        /// The replacement.
        with: Box<Expr>,
    },
    /// Pads a string to `length` characters with `fill`, at the start or the end.
    Pad {
        /// The string.
        of: Box<Expr>,
        /// The target length in characters (at most 1000).
        length: u32,
        /// The fill text (not empty).
        fill: String,
        /// Pad at the end instead of the start.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "codegen", ts(as = "Option<bool>", optional))]
        end: bool,
    },
    /// `true` if the string matches a regular expression. Matching takes linear time.
    Matches {
        /// The string.
        of: Box<Expr>,
        /// The pattern (at most 256 characters).
        pattern: String,
    },
    /// Formats an int or decimal with `decimals` digits after the point, optionally grouping
    /// thousands with commas.
    FormatNumber {
        /// The number.
        of: Box<Expr>,
        /// Digits after the point (the decimal's own scale if omitted).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        decimals: Option<u8>,
        /// Group thousands with commas.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "codegen", ts(as = "Option<bool>", optional))]
        grouping: bool,
    },
    /// An int or decimal as a decimal with `scale` digits after the point.
    Round {
        /// The number.
        of: Box<Expr>,
        /// Digits after the point (0-18).
        scale: u8,
        /// How to round.
        #[serde(default, skip_serializing_if = "Rounding::is_default")]
        #[cfg_attr(feature = "codegen", ts(as = "Option<Rounding>", optional))]
        mode: Rounding,
    },
    /// An int or decimal as an int.
    ToInt {
        /// The number.
        of: Box<Expr>,
        /// How to round.
        #[serde(default, skip_serializing_if = "Rounding::is_default")]
        #[cfg_attr(feature = "codegen", ts(as = "Option<Rounding>", optional))]
        mode: Rounding,
    },
    /// The start of the unit containing a timestamp, in a time zone (UTC if `null`).
    StartOf {
        /// The timestamp (ms).
        of: Box<Expr>,
        /// The unit.
        unit: TimeUnit,
        /// An IANA time zone name, or `null` for UTC.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        tz: Option<Box<Expr>>,
    },
    /// A timestamp moved by `amount` units (negative moves back). Days and larger units follow
    /// the calendar in the time zone, so `+1 day` keeps the wall-clock time across DST changes.
    AddTime {
        /// The timestamp (ms).
        of: Box<Expr>,
        /// How many units.
        amount: Box<Expr>,
        /// The unit.
        unit: TimeUnit,
        /// An IANA time zone name, or `null` for UTC.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        tz: Option<Box<Expr>>,
    },
    /// One part of a timestamp, such as its weekday, in a time zone.
    TimePart {
        /// The timestamp (ms).
        of: Box<Expr>,
        /// The part.
        part: TimePart,
        /// An IANA time zone name, or `null` for UTC.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        tz: Option<Box<Expr>>,
    },
    /// Formats a timestamp: ISO 8601 by default, or a `strftime` pattern.
    FormatTime {
        /// The timestamp (ms).
        of: Box<Expr>,
        /// A `strftime` pattern such as `%Y-%m-%d` (at most 64 characters).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        pattern: Option<String>,
        /// An IANA time zone name, or `null` for UTC.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        tz: Option<Box<Expr>>,
    },
}
