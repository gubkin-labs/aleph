//! Worst-case row sizes. Rows are stored as JSON, so these bounds follow the JSON encoding.

use crate::manifest::{Entity, FieldType};

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
    }
}

/// Upper bound on the JSON size of one row, in bytes. Optional fields may also be `null` (4
/// bytes), which every bound above already covers.
pub(crate) fn row_bytes(entity: &Entity) -> u64 {
    let fields: u64 = entity
        .fields
        .iter()
        // "name": value,
        .map(|(name, field)| name.len() as u64 + 4 + field_bytes(&field.ty).max(4))
        .sum();
    2 + fields
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
