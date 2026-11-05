//! Static types used by the checker. Deliberately small: enough to resolve fields, check
//! operands, and carry the static bounds that cost analysis needs.

use std::collections::BTreeMap;

use crate::manifest::{FieldType, Literal};

/// The static type of an expression or operation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ty {
    /// Not known statically. Never rejected, checked at runtime instead.
    Unknown,
    Null,
    Bool,
    /// An integer with a known upper bound, if any. Upper bounds make `limit` expressions and
    /// list lengths analyzable.
    Int {
        max: Option<i64>,
    },
    String,
    /// A string drawn from a fixed set.
    Enum(Vec<String>),
    /// A row of the named entity.
    Row(String),
    /// A row of the named entity, or `null`.
    OptRow(String),
    /// A list with a known maximum length.
    List {
        item: Box<Ty>,
        max: u64,
    },
    Object(BTreeMap<String, Ty>),
}

impl Ty {
    pub(crate) fn from_field_type(ty: &FieldType) -> Self {
        match ty {
            FieldType::String { .. } | FieldType::Id => Self::String,
            FieldType::Int { max, .. } => Self::Int { max: *max },
            FieldType::Timestamp => Self::Int { max: None },
            FieldType::Bool => Self::Bool,
            FieldType::Enum { values } => Self::Enum(values.clone()),
            FieldType::Array { items, max } => Self::List {
                item: Box::new(Self::from_field_type(items)),
                max: u64::from(*max),
            },
        }
    }

    pub(crate) fn from_literal(literal: &Literal) -> Self {
        match literal {
            Literal::Null => Self::Null,
            Literal::Bool(_) => Self::Bool,
            Literal::Int(i) => Self::Int { max: Some(*i) },
            // A string constant is a one-value enum, so `$.select(c, row.plan, "free")` keeps the
            // enum type and stays exhaustively matchable.
            Literal::String(s) => Self::Enum(vec![s.clone()]),
        }
    }

    pub(crate) fn is_boolish(&self) -> bool {
        matches!(self, Self::Bool | Self::Unknown)
    }

    pub(crate) fn is_intish(&self) -> bool {
        matches!(self, Self::Int { .. } | Self::Unknown)
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Unknown => "unknown".into(),
            Self::Null => "null".into(),
            Self::Bool => "bool".into(),
            Self::Int { .. } => "int".into(),
            Self::String => "string".into(),
            Self::Enum(values) => format!("enum({})", values.join("|")),
            Self::Row(e) => format!("row of {e}"),
            Self::OptRow(e) => format!("row of {e} or null"),
            Self::List { item, max } => format!("list of {} (max {max})", item.describe()),
            Self::Object(_) => "object".into(),
        }
    }

    /// The least type covering both `self` and `other` (used for branches).
    pub(crate) fn join(self, other: Self) -> Self {
        match (self, other) {
            (a, b) if a == b => a,
            (Self::Int { max: a }, Self::Int { max: b }) => Self::Int {
                max: a.zip(b).map(|(a, b)| a.max(b)),
            },
            (Self::Row(a) | Self::OptRow(a), Self::Row(b) | Self::OptRow(b)) if a == b => {
                Self::OptRow(a)
            }
            (Self::Null, Self::Row(e) | Self::OptRow(e))
            | (Self::Row(e) | Self::OptRow(e), Self::Null) => Self::OptRow(e),
            (Self::Enum(mut a), Self::Enum(b)) => {
                for value in b {
                    if !a.contains(&value) {
                        a.push(value);
                    }
                }
                Self::Enum(a)
            }
            (Self::Enum(_) | Self::String, Self::Enum(_) | Self::String) => Self::String,
            (Self::List { item: a, max: ma }, Self::List { item: b, max: mb }) => Self::List {
                item: Box::new(a.join(*b)),
                max: ma.max(mb),
            },
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_widens_rows_and_bounds() {
        assert_eq!(Ty::Row("A".into()).join(Ty::Null), Ty::OptRow("A".into()));
        assert_eq!(
            Ty::Int { max: Some(3) }.join(Ty::Int { max: Some(9) }),
            Ty::Int { max: Some(9) }
        );
        assert_eq!(
            Ty::Int { max: Some(3) }.join(Ty::Int { max: None }),
            Ty::Int { max: None }
        );
        assert_eq!(Ty::Bool.join(Ty::String), Ty::Unknown);
        let list = |max| Ty::List {
            item: Box::new(Ty::Bool),
            max,
        };
        assert_eq!(list(2).join(list(7)), list(7));
    }
}
