//! Static types used by the checker. Deliberately small: enough to resolve fields, check
//! operands, infer response types, and carry the static bounds that cost analysis needs.

use std::collections::BTreeMap;

use crate::manifest::{Field, FieldType, Literal};

/// The longest caller id the platform passes in, in characters.
pub(crate) const CALLER_ID_MAX: u64 = 256;
/// The longest pagination cursor, in characters.
pub(crate) const CURSOR_MAX: u64 = 8192;

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
    /// A decimal, with its scale if known.
    Decimal {
        scale: Option<u8>,
    },
    /// A string with a known maximum length in characters, if any.
    String {
        max: Option<u64>,
    },
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
    /// A value of the inner type, or `null`.
    Opt(Box<Ty>),
}

impl Ty {
    pub(crate) fn string(max: u64) -> Self {
        Self::String { max: Some(max) }
    }

    pub(crate) fn from_field_type(ty: &FieldType) -> Self {
        match ty {
            FieldType::String { max, .. } => Self::string(u64::from(*max)),
            FieldType::Id => Self::string(26),
            FieldType::TimeZone => Self::string(64),
            FieldType::Int { max, .. } => Self::Int { max: *max },
            FieldType::Timestamp => Self::Int { max: None },
            FieldType::Decimal { scale, .. } => Self::Decimal {
                scale: Some(*scale),
            },
            FieldType::Bool => Self::Bool,
            FieldType::Enum { values } => Self::Enum(values.clone()),
            FieldType::Array { items, max } => Self::List {
                item: Box::new(Self::from_field_type(items)),
                max: u64::from(*max),
            },
            FieldType::Object { fields } => Self::Object(
                fields
                    .iter()
                    .map(|(n, f)| (n.clone(), Self::from_field(f)))
                    .collect(),
            ),
        }
    }

    /// A field's type: optional fields may be `null`.
    pub(crate) fn from_field(field: &Field) -> Self {
        let ty = Self::from_field_type(&field.ty);
        if field.optional { ty.nullable() } else { ty }
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

    /// This type, or `null`.
    pub(crate) fn nullable(self) -> Self {
        match self {
            Self::Null | Self::Unknown | Self::Opt(_) | Self::OptRow(_) => self,
            Self::Row(e) => Self::OptRow(e),
            other => Self::Opt(Box::new(other)),
        }
    }

    /// The type without `null`: what operations see once `null` has been ruled out.
    pub(crate) fn non_null(&self) -> &Self {
        match self {
            Self::Opt(inner) => inner,
            other => other,
        }
    }

    pub(crate) fn is_boolish(&self) -> bool {
        matches!(self.non_null(), Self::Bool | Self::Unknown)
    }

    pub(crate) fn is_intish(&self) -> bool {
        matches!(self.non_null(), Self::Int { .. } | Self::Unknown)
    }

    /// An int or a decimal.
    pub(crate) fn is_numeric(&self) -> bool {
        matches!(
            self.non_null(),
            Self::Int { .. } | Self::Decimal { .. } | Self::Unknown
        )
    }

    pub(crate) fn is_stringish(&self) -> bool {
        matches!(
            self.non_null(),
            Self::String { .. } | Self::Enum(_) | Self::Unknown | Self::Null
        )
    }

    /// The longest string this type can format as, in characters (for concatenation bounds).
    pub(crate) fn max_chars(&self) -> Option<u64> {
        match self {
            Self::String { max } => *max,
            Self::Enum(values) => values.iter().map(|v| v.chars().count() as u64).max(),
            Self::Null => Some(4),
            Self::Bool => Some(5),
            Self::Int { .. } => Some(20),
            Self::Decimal { .. } => Some(42),
            Self::Opt(inner) => inner.max_chars().map(|n| n.max(4)),
            _ => None,
        }
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Unknown => "unknown".into(),
            Self::Null => "null".into(),
            Self::Bool => "bool".into(),
            Self::Int { .. } => "int".into(),
            Self::Decimal { scale: Some(s) } => format!("decimal({s})"),
            Self::Decimal { scale: None } => "decimal".into(),
            Self::String { .. } => "string".into(),
            Self::Enum(values) => format!("enum({})", values.join("|")),
            Self::Row(e) => format!("row of {e}"),
            Self::OptRow(e) => format!("row of {e} or null"),
            Self::List { item, max } => format!("list of {} (max {max})", item.describe()),
            Self::Object(_) => "object".into(),
            Self::Opt(inner) => format!("{} or null", inner.describe()),
        }
    }

    /// Like [`Ty::describe`], with the bounds that can grow: string lengths, list lengths, and
    /// an object's fields.
    pub(crate) fn describe_fields(&self) -> String {
        match self {
            Self::Object(fields) => {
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(name, ty)| format!("{name}: {}", ty.describe_fields()))
                    .collect();
                format!("{{ {} }}", parts.join(", "))
            }
            Self::String { max: Some(max) } => format!("string of at most {max} characters"),
            Self::List { item, max } => {
                format!("list of {} (max {max})", item.describe_fields())
            }
            Self::Opt(inner) => format!("{} or null", inner.describe_fields()),
            other => other.describe(),
        }
    }

    /// The least type covering both `self` and `other` (used for branches).
    pub(crate) fn join(self, other: Self) -> Self {
        match (self, other) {
            (a, b) if a == b => a,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::Null, other) | (other, Self::Null) => other.nullable(),
            (Self::Opt(a), b) | (b, Self::Opt(a)) => a.join(b).nullable(),
            (Self::Int { max: a }, Self::Int { max: b }) => Self::Int {
                max: a.zip(b).map(|(a, b)| a.max(b)),
            },
            (Self::Decimal { scale: a }, Self::Decimal { scale: b }) => Self::Decimal {
                scale: if a == b { a } else { None },
            },
            (Self::Row(a) | Self::OptRow(a), Self::Row(b) | Self::OptRow(b)) if a == b => {
                Self::OptRow(a)
            }
            (Self::Enum(mut a), Self::Enum(b)) => {
                for value in b {
                    if !a.contains(&value) {
                        a.push(value);
                    }
                }
                Self::Enum(a)
            }
            (
                a @ (Self::Enum(_) | Self::String { .. }),
                b @ (Self::Enum(_) | Self::String { .. }),
            ) => Self::String {
                max: a.max_chars().zip(b.max_chars()).map(|(a, b)| a.max(b)),
            },
            (Self::List { item: a, max: ma }, Self::List { item: b, max: mb }) => Self::List {
                item: Box::new(a.join(*b)),
                max: ma.max(mb),
            },
            (Self::Object(a), Self::Object(b)) if a.keys().eq(b.keys()) => Self::Object(
                a.into_iter()
                    .zip(b)
                    .map(|((name, a), (_, b))| (name, a.join(b)))
                    .collect(),
            ),
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
        assert_eq!(Ty::Bool.join(Ty::string(3)), Ty::Unknown);
        let list = |max| Ty::List {
            item: Box::new(Ty::Bool),
            max,
        };
        assert_eq!(list(2).join(list(7)), list(7));
    }

    #[test]
    fn join_tracks_null() {
        assert_eq!(
            Ty::Int { max: None }.join(Ty::Null),
            Ty::Opt(Box::new(Ty::Int { max: None }))
        );
        assert_eq!(
            Ty::Null.join(Ty::string(4)).join(Ty::string(9)),
            Ty::Opt(Box::new(Ty::string(9)))
        );
        assert!(Ty::Int { max: None }.nullable().is_intish());
    }
}
