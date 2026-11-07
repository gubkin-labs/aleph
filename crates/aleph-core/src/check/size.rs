//! Worst-case sizes. Rows are stored as JSON, so these bounds follow the JSON encoding, and
//! every runtime value's [`crate::value::Value::size`] stays within its type's [`ty_bytes`].

use std::collections::BTreeMap;

use super::Ty;
use crate::manifest::{Entity, Field, FieldType};
use crate::value::{ENTRY_BYTES, STRING_BYTES, VALUE_BYTES};

/// Upper bound on the JSON size of one field value, in bytes.
pub(crate) fn field_bytes(ty: &FieldType) -> u64 {
    match ty {
        // Quotes, plus up to 6 bytes per character for escaped control characters.
        FieldType::String { max, .. } => 2 + 6 * u64::from(*max),
        // "-9223372036854775808"
        FieldType::Int { .. } | FieldType::Timestamp => 20,
        // "false"
        FieldType::Bool => 5,
        FieldType::Enum { values } => {
            2 + values.iter().map(|v| 6 * v.len() as u64).max().unwrap_or(0)
        }
        // A quoted 26-character ULID.
        FieldType::Id => 28,
        // Brackets, commas, and items.
        FieldType::Array { items, max } => 2 + u64::from(*max) * (field_bytes(items) + 1),
        FieldType::Object { fields } => object_bytes(fields),
        // A stored decimal: {"$dec":"-99999999999999999999.999999999999999999"}
        FieldType::Decimal { .. } => 52,
        // A quoted IANA name (at most 64 characters).
        FieldType::TimeZone => 2 + 64,
    }
}

/// Upper bound on the JSON size of an object with these fields.
fn object_bytes(fields: &BTreeMap<String, Field>) -> u64 {
    let fields: u64 = fields
        .iter()
        // "name": value,
        .map(|(name, field)| name.len() as u64 + 4 + field_bytes(&field.ty).max(4))
        .sum();
    2 + fields
}

/// Upper bound on the stored size of a user profile with these declared fields.
pub(crate) fn profile_bytes(fields: &BTreeMap<String, Field>) -> u64 {
    object_bytes(fields)
}

/// Upper bound on the in-memory size of a value of type `ty` ([`crate::value::Value::size`]).
/// Values of unknown size count as `fallback` bytes; the runtime meter is the backstop for them.
pub(crate) fn ty_bytes(ty: &Ty, rows: &dyn Fn(&str) -> u64, fallback: u64) -> u64 {
    match ty {
        Ty::Unknown => fallback,
        Ty::Null | Ty::Bool | Ty::Int { .. } | Ty::Decimal { .. } => VALUE_BYTES,
        // UTF-8 uses at most 4 bytes per character.
        Ty::String { max } => max.map_or(fallback, |max| VALUE_BYTES + STRING_BYTES + 4 * max),
        Ty::Enum(values) => {
            VALUE_BYTES + STRING_BYTES + values.iter().map(|v| v.len() as u64).max().unwrap_or(0)
        }
        Ty::Row(entity) | Ty::OptRow(entity) => rows(entity).max(VALUE_BYTES),
        Ty::List { item, max } => {
            VALUE_BYTES.saturating_add(max.saturating_mul(ty_bytes(item, rows, fallback)))
        }
        Ty::Object(fields) => VALUE_BYTES.saturating_add(
            fields
                .iter()
                .map(|(name, ty)| {
                    (ENTRY_BYTES + name.len() as u64).saturating_add(ty_bytes(ty, rows, fallback))
                })
                .fold(0_u64, u64::saturating_add),
        ),
        Ty::Opt(inner) => ty_bytes(inner, rows, fallback).max(VALUE_BYTES),
    }
}

/// Upper bound on the in-memory size of one row ([`crate::value::Value::size`]).
pub(crate) fn row_memory(entity: &Entity) -> u64 {
    let fields = entity
        .fields
        .iter()
        .map(|(name, field)| (name.clone(), Ty::from_field(field)))
        .collect();
    ty_bytes(&Ty::Object(fields), &|_| 0, 0)
}

/// Upper bound on the JSON size of one row, in bytes. Optional fields may also be `null` (4
/// bytes), which every bound above already covers.
pub(crate) fn row_bytes(entity: &Entity) -> u64 {
    object_bytes(&entity.fields)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::manifest::Field;
    use crate::value::Value;

    fn field(ty: FieldType) -> Field {
        Field {
            ty,
            optional: false,
            default: None,
            deprecated: None,
        }
    }

    #[test]
    fn bound_covers_the_largest_real_row() {
        let entity = Entity {
            key: vec!["id".into()],
            fields: BTreeMap::from([
                ("id".into(), field(FieldType::Id)),
                ("name".into(), field(FieldType::String { min: 0, max: 8 })),
                (
                    "count".into(),
                    field(FieldType::Int {
                        min: None,
                        max: None,
                    }),
                ),
                (
                    "tags".into(),
                    field(FieldType::Array {
                        items: Box::new(FieldType::Bool),
                        max: 3,
                    }),
                ),
            ]),
            indexes: BTreeMap::new(),
            invariants: BTreeMap::new(),
            versioned: false,
            ttl: None,
            deprecated: None,
            mutable: true,
        };
        let worst = Value::Object(BTreeMap::from([
            ("id".into(), Value::String("01J".repeat(9)[..26].to_owned())),
            ("name".into(), Value::String("\u{1}".repeat(8))),
            ("count".into(), Value::Int(i64::MIN)),
            ("tags".into(), Value::List(vec![Value::Bool(false); 3])),
        ]));
        let actual = serde_json::to_vec(&worst).unwrap().len() as u64;
        assert!(
            actual <= row_bytes(&entity),
            "{actual} > {}",
            row_bytes(&entity)
        );
    }
}
