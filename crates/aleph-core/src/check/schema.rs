//! Response types: what each route can return, as a schema that docs and generated clients use.
//! Inferred from the plan by the checker, or taken from the route's declared `returns`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Ty;
use crate::manifest::{Field, FieldType, Manifest};

/// The type of a JSON value a route returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TypeSchema {
    /// Any JSON value (the checker couldn't tell).
    Any,
    /// `null`.
    Null,
    /// A boolean.
    Bool,
    /// An integer.
    Int,
    /// An exact decimal, sent as a string like `"12.50"`.
    Decimal {
        /// Digits after the point, if known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional))]
        scale: Option<u8>,
    },
    /// A string.
    String {
        /// Maximum length in characters, if known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional, type = "number"))]
        max_length: Option<u64>,
    },
    /// One of a fixed set of strings.
    Enum {
        /// The values.
        values: Vec<String>,
    },
    /// A list.
    Array {
        /// The item type.
        items: Box<TypeSchema>,
        /// Maximum number of items.
        #[cfg_attr(feature = "codegen", ts(type = "number"))]
        max_items: u64,
    },
    /// An object with known fields.
    Object {
        /// Fields, by name.
        fields: BTreeMap<String, TypeSchema>,
    },
    /// A value of the inner type, or `null`.
    Nullable {
        /// The inner type.
        of: Box<TypeSchema>,
    },
    /// Any one of several types (routes that return different shapes on different branches).
    OneOf {
        /// The alternatives.
        variants: Vec<TypeSchema>,
    },
}

impl TypeSchema {
    /// The schema of a value of static type `ty`.
    pub(crate) fn of_ty(ty: &Ty, m: &Manifest) -> Self {
        Self::of_ty_at(ty, m, 0)
    }

    fn of_ty_at(ty: &Ty, m: &Manifest, depth: usize) -> Self {
        if depth > 8 {
            return Self::Any;
        }
        match ty {
            Ty::Unknown => Self::Any,
            Ty::Null => Self::Null,
            Ty::Bool => Self::Bool,
            Ty::Int { .. } => Self::Int,
            Ty::Decimal { scale } => Self::Decimal { scale: *scale },
            Ty::String { max } => Self::String { max_length: *max },
            Ty::Enum(values) => Self::Enum {
                values: values.clone(),
            },
            Ty::Row(entity) => m
                .entities
                .get(entity)
                .map_or(Self::Any, |e| Self::of_fields(&e.fields)),
            Ty::OptRow(entity) => Self::Nullable {
                of: Box::new(Self::of_ty_at(&Ty::Row(entity.clone()), m, depth)),
            },
            Ty::List { item, max } => Self::Array {
                items: Box::new(Self::of_ty_at(item, m, depth + 1)),
                max_items: *max,
            },
            Ty::Object(fields) => Self::Object {
                fields: fields
                    .iter()
                    .map(|(n, t)| (n.clone(), Self::of_ty_at(t, m, depth + 1)))
                    .collect(),
            },
            Ty::Opt(inner) => Self::Nullable {
                of: Box::new(Self::of_ty_at(inner, m, depth)),
            },
        }
    }

    /// The schema of an object with declared fields.
    pub(crate) fn of_fields(fields: &BTreeMap<String, Field>) -> Self {
        Self::Object {
            fields: fields
                .iter()
                .map(|(n, f)| (n.clone(), Self::of_field(f)))
                .collect(),
        }
    }

    fn of_field(field: &Field) -> Self {
        let ty = Self::of_field_type(&field.ty);
        if field.optional {
            Self::Nullable { of: Box::new(ty) }
        } else {
            ty
        }
    }

    /// The schema of a declared field type.
    pub(crate) fn of_field_type(ty: &FieldType) -> Self {
        match ty {
            FieldType::String { max, .. } => Self::String {
                max_length: Some(u64::from(*max)),
            },
            FieldType::Id => Self::String {
                max_length: Some(26),
            },
            FieldType::TimeZone => Self::String {
                max_length: Some(64),
            },
            FieldType::Int { .. } | FieldType::Timestamp => Self::Int,
            FieldType::Decimal { scale, .. } => Self::Decimal {
                scale: Some(*scale),
            },
            FieldType::Bool => Self::Bool,
            FieldType::Enum { values } => Self::Enum {
                values: values.clone(),
            },
            FieldType::Array { items, max } => Self::Array {
                items: Box::new(Self::of_field_type(items)),
                max_items: u64::from(*max),
            },
            FieldType::Object { fields } => Self::of_fields(fields),
        }
    }

    /// The union of several response schemas, without duplicates.
    pub(crate) fn union(schemas: Vec<Self>) -> Self {
        let mut variants: Vec<Self> = Vec::new();
        for schema in schemas {
            let parts = match schema {
                Self::OneOf { variants } => variants,
                other => vec![other],
            };
            for part in parts {
                if !variants.contains(&part) {
                    variants.push(part);
                }
            }
        }
        match variants.len() {
            0 => Self::Any,
            1 => variants.remove(0),
            _ => Self::OneOf { variants },
        }
    }
}

/// Checks that a value of static type `ty` can be returned as `declared`: every declared field
/// is there with a fitting type. Extra fields are allowed (they're dropped from the response).
/// Returns a description of the first problem, with its path inside the value.
pub(crate) fn fits(
    ty: &Ty,
    declared: &FieldType,
    optional: bool,
    m: &Manifest,
) -> Result<(), String> {
    let mismatch = |found: &Ty| {
        Err(format!(
            "returns {} where the declared type needs {}",
            found.describe(),
            describe(declared)
        ))
    };
    match ty {
        Ty::Unknown => Ok(()),
        Ty::Null if optional => Ok(()),
        Ty::Opt(inner) if optional => fits(inner, declared, false, m),
        Ty::OptRow(e) if optional => fits(&Ty::Row(e.clone()), declared, false, m),
        Ty::Null | Ty::Opt(_) | Ty::OptRow(_) => Err(format!(
            "may return null, but the declared {} isn't optional",
            describe(declared)
        )),
        Ty::Bool => match declared {
            FieldType::Bool => Ok(()),
            _ => mismatch(ty),
        },
        Ty::Int { .. } => match declared {
            FieldType::Int { .. } | FieldType::Timestamp | FieldType::Decimal { .. } => Ok(()),
            _ => mismatch(ty),
        },
        Ty::Decimal { .. } => match declared {
            FieldType::Decimal { .. } => Ok(()),
            _ => mismatch(ty),
        },
        Ty::String { .. } => match declared {
            FieldType::String { .. }
            | FieldType::Id
            | FieldType::TimeZone
            | FieldType::Enum { .. } => Ok(()),
            _ => mismatch(ty),
        },
        Ty::Enum(values) => match declared {
            FieldType::Enum { values: allowed } => {
                match values.iter().find(|v| !allowed.contains(v)) {
                    Some(extra) => Err(format!(
                        "may return {extra:?}, which isn't one of {allowed:?}"
                    )),
                    None => Ok(()),
                }
            }
            FieldType::String { .. } | FieldType::Id | FieldType::TimeZone => Ok(()),
            _ => mismatch(ty),
        },
        Ty::List { item, .. } => match declared {
            FieldType::Array { items, .. } => {
                fits(item, items, false, m).map_err(|e| format!("items: {e}"))
            }
            _ => mismatch(ty),
        },
        Ty::Row(_) | Ty::Object(_) => {
            let FieldType::Object { fields } = declared else {
                return mismatch(ty);
            };
            for (name, field) in fields {
                let found = match ty {
                    Ty::Row(entity) => m
                        .entities
                        .get(entity)
                        .and_then(|e| e.fields.get(name))
                        .map(Ty::from_field),
                    Ty::Object(found) => found.get(name).cloned(),
                    _ => None,
                };
                match found {
                    Some(found) => fits(&found, &field.ty, field.optional, m)
                        .map_err(|e| format!("{name}: {e}"))?,
                    None if field.optional => {}
                    None => return Err(format!("{name}: missing from the returned value")),
                }
            }
            Ok(())
        }
    }
}

fn describe(ty: &FieldType) -> String {
    match ty {
        FieldType::String { .. } => "string".into(),
        FieldType::Int { .. } => "int".into(),
        FieldType::Decimal { scale, .. } => format!("decimal({scale})"),
        FieldType::Bool => "bool".into(),
        FieldType::Enum { values } => format!("enum({})", values.join("|")),
        FieldType::Id => "id".into(),
        FieldType::Timestamp => "timestamp".into(),
        FieldType::TimeZone => "time zone".into(),
        FieldType::Array { .. } => "array".into(),
        FieldType::Object { .. } => "object".into(),
    }
}
