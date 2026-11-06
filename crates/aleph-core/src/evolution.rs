//! Schema evolution: comparing a new manifest with the deployed one.
//!
//! Every change is classified as **safe** (deploy it), **needs backfill** (deploy it, but
//! existing data must be indexed or counted first; the platform runs the plan in
//! [`EvolutionReport::migration`] before activating the revision), or **breaking** (existing
//! data or clients would break). Breaking changes are allowed only when the old manifest marked
//! the thing deprecated, which gives clients a window to move.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::manifest::{Entity, Field, FieldType, Manifest, Route};

/// How risky one change is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    /// Nothing existing is affected.
    Safe,
    /// Existing rows must be processed (indexed or counted) before the change takes effect.
    NeedsBackfill,
    /// Existing data or clients would break.
    Breaking,
}

/// One difference between two manifests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SchemaChange {
    /// Where, like `entities.Task.fields.title`.
    pub path: String,
    /// How risky it is.
    pub kind: ChangeKind,
    /// What changed, and what to do about it.
    pub message: String,
    /// For breaking changes: whether the old manifest deprecated the thing, which allows it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "codegen", ts(as = "Option<bool>", optional))]
    pub allowed: bool,
}

/// One step the platform runs before activating a new revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(
    tag = "task",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MigrationTask {
    /// Write index entries for every existing row.
    BuildIndex {
        /// The entity.
        entity: String,
        /// The index.
        index: String,
    },
    /// Recompute a counter from every existing row.
    BuildCounter {
        /// The counter.
        counter: String,
    },
    /// Remove a dropped index's entries.
    DropIndex {
        /// The entity.
        entity: String,
        /// The index.
        index: String,
    },
    /// Remove a dropped counter's values.
    DropCounter {
        /// The counter.
        counter: String,
    },
}

/// Every change between two manifests, and the migration plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EvolutionReport {
    /// The changes, in manifest order.
    pub changes: Vec<SchemaChange>,
    /// What the platform runs before the new revision serves traffic.
    pub migration: Vec<MigrationTask>,
}

impl EvolutionReport {
    /// Whether the new manifest can be deployed: no breaking change that wasn't deprecated.
    pub fn deployable(&self) -> bool {
        self.changes
            .iter()
            .all(|c| c.kind != ChangeKind::Breaking || c.allowed)
    }
}

/// Compares the deployed manifest `old` with `new`.
#[allow(clippy::too_many_lines, reason = "one comparison per manifest section")]
pub fn compare(old: &Manifest, new: &Manifest) -> EvolutionReport {
    let mut report = EvolutionReport::default();
    let mut add = |path: String, kind: ChangeKind, message: String, allowed: bool| {
        report.changes.push(SchemaChange {
            path,
            kind,
            message,
            allowed,
        });
    };

    for (name, old_entity) in &old.entities {
        let path = format!("entities.{name}");
        match new.entities.get(name) {
            None => add(
                path,
                ChangeKind::Breaking,
                format!("entity {name} was removed; its rows would become unreachable"),
                old_entity.deprecated.is_some(),
            ),
            Some(new_entity) => compare_entity(&path, old_entity, new_entity, &mut add),
        }
    }
    for name in new
        .entities
        .keys()
        .filter(|n| !old.entities.contains_key(*n))
    {
        add(
            format!("entities.{name}"),
            ChangeKind::Safe,
            format!("new entity {name}"),
            false,
        );
    }

    let mut migration = Vec::new();
    for (name, new_entity) in &new.entities {
        let Some(old_entity) = old.entities.get(name) else {
            continue;
        };
        for (index, def) in &new_entity.indexes {
            let path = format!("entities.{name}.indexes.{index}");
            match old_entity.indexes.get(index) {
                Some(old_def) if old_def == def => {}
                Some(_) => {
                    add(
                        path,
                        ChangeKind::NeedsBackfill,
                        format!("index {index} changed: it's rebuilt from existing rows"),
                        false,
                    );
                    migration.push(MigrationTask::DropIndex {
                        entity: name.clone(),
                        index: index.clone(),
                    });
                    migration.push(MigrationTask::BuildIndex {
                        entity: name.clone(),
                        index: index.clone(),
                    });
                }
                None => {
                    let unique = if def.unique {
                        " Existing duplicates would fail the build."
                    } else {
                        ""
                    };
                    add(
                        path,
                        ChangeKind::NeedsBackfill,
                        format!("new index {index}: built from existing rows before use.{unique}"),
                        false,
                    );
                    migration.push(MigrationTask::BuildIndex {
                        entity: name.clone(),
                        index: index.clone(),
                    });
                }
            }
        }
        for index in old_entity
            .indexes
            .keys()
            .filter(|i| !new_entity.indexes.contains_key(*i))
        {
            add(
                format!("entities.{name}.indexes.{index}"),
                ChangeKind::Safe,
                format!("index {index} was removed"),
                false,
            );
            migration.push(MigrationTask::DropIndex {
                entity: name.clone(),
                index: index.clone(),
            });
        }
    }

    for (name, counter) in &new.counters {
        let path = format!("counters.{name}");
        match old.counters.get(name) {
            Some(old_counter) if old_counter == counter => {}
            Some(_) => {
                add(
                    path,
                    ChangeKind::NeedsBackfill,
                    format!("counter {name} changed: it's recomputed from existing rows"),
                    false,
                );
                migration.push(MigrationTask::DropCounter {
                    counter: name.clone(),
                });
                migration.push(MigrationTask::BuildCounter {
                    counter: name.clone(),
                });
            }
            None if old.entities.contains_key(&counter.of) => {
                add(
                    path,
                    ChangeKind::NeedsBackfill,
                    format!(
                        "new counter {name} over existing {}: computed before use",
                        counter.of
                    ),
                    false,
                );
                migration.push(MigrationTask::BuildCounter {
                    counter: name.clone(),
                });
            }
            None => add(path, ChangeKind::Safe, format!("new counter {name}"), false),
        }
    }
    for name in old
        .counters
        .keys()
        .filter(|n| !new.counters.contains_key(*n))
    {
        add(
            format!("counters.{name}"),
            ChangeKind::Safe,
            format!("counter {name} was removed"),
            false,
        );
        migration.push(MigrationTask::DropCounter {
            counter: name.clone(),
        });
    }

    compare_routes(&old.routes, &new.routes, &mut add);
    report.migration = migration;
    report
}

type Add<'a> = dyn FnMut(String, ChangeKind, String, bool) + 'a;

fn compare_entity(path: &str, old: &Entity, new: &Entity, add: &mut Add<'_>) {
    if old.key != new.key {
        add(
            format!("{path}.key"),
            ChangeKind::Breaking,
            format!(
                "the primary key changed from {:?} to {:?}; existing rows are stored by the old key",
                old.key, new.key
            ),
            false,
        );
    }
    for (name, old_field) in &old.fields {
        let field_path = format!("{path}.fields.{name}");
        match new.fields.get(name) {
            None => add(
                field_path,
                ChangeKind::Breaking,
                format!("field {name} was removed; deprecate it first, then drop it"),
                old_field.deprecated.is_some(),
            ),
            Some(new_field) => compare_field(&field_path, old_field, new_field, add),
        }
    }
    for (name, field) in new
        .fields
        .iter()
        .filter(|(n, _)| !old.fields.contains_key(*n))
    {
        let (kind, message) = if field.optional || field.default.is_some() {
            (ChangeKind::Safe, format!("new field {name}"))
        } else {
            (
                ChangeKind::Breaking,
                format!(
                    "new required field {name} has no value in existing rows; make it optional or give it a default"
                ),
            )
        };
        add(format!("{path}.fields.{name}"), kind, message, false);
    }
    if !old.mutable && new.mutable {
        add(
            format!("{path}.mutable"),
            ChangeKind::Safe,
            "the entity is no longer append-only".into(),
            false,
        );
    } else if old.mutable && !new.mutable {
        add(
            format!("{path}.mutable"),
            ChangeKind::Breaking,
            "the entity became append-only; plans that update or delete it would fail".into(),
            false,
        );
    }
}

fn compare_field(path: &str, old: &Field, new: &Field, add: &mut Add<'_>) {
    if old.optional && !new.optional && new.default.is_none() {
        add(
            path.to_owned(),
            ChangeKind::Breaking,
            "the field became required, but existing rows may not have it".into(),
            false,
        );
    }
    match widening(&old.ty, &new.ty) {
        Ok(None) => {}
        Ok(Some(message)) => add(path.to_owned(), ChangeKind::Safe, message, false),
        Err(message) => add(
            path.to_owned(),
            ChangeKind::Breaking,
            message,
            old.deprecated.is_some(),
        ),
    }
}

/// `Ok(None)` if the types are equal, `Ok(Some(_))` if `new` accepts every `old` value, and
/// `Err(_)` otherwise.
fn widening(old: &FieldType, new: &FieldType) -> Result<Option<String>, String> {
    if old == new {
        return Ok(None);
    }
    let widened = |what: &str| Ok(Some(format!("{what} widened")));
    match (old, new) {
        (FieldType::String { min: a, max: b }, FieldType::String { min: c, max: d }) => {
            if c <= a && d >= b {
                widened("string length range")
            } else {
                Err(format!("string length {a}-{b} narrowed to {c}-{d}; existing values may not fit"))
            }
        }
        (FieldType::Int { min: a, max: b }, FieldType::Int { min: c, max: d }) => {
            let lower_ok = c.is_none_or(|c| a.is_some_and(|a| c <= a));
            let upper_ok = d.is_none_or(|d| b.is_some_and(|b| d >= b));
            if lower_ok && upper_ok {
                widened("int range")
            } else {
                Err("int range narrowed; existing values may not fit".into())
            }
        }
        (FieldType::Enum { values: a }, FieldType::Enum { values: b }) => {
            let removed: Vec<&String> = a.iter().filter(|v| !b.contains(v)).collect();
            if removed.is_empty() {
                widened("enum")
            } else {
                Err(format!("enum values {removed:?} were removed; existing rows may hold them"))
            }
        }
        (
            FieldType::Array {
                items: old_items,
                max: old_max,
            },
            FieldType::Array {
                items: new_items,
                max: new_max,
            },
        ) => {
            if new_max < old_max {
                return Err(format!("array max lowered from {old_max} to {new_max}"));
            }
            match widening(old_items, new_items) {
                Ok(_) => widened("array"),
                Err(e) => Err(format!("array items: {e}")),
            }
        }
        (FieldType::Decimal { scale: a, .. }, FieldType::Decimal { scale: b, .. }) if b >= a => {
            widened("decimal")
        }
        (FieldType::Object { fields: a }, FieldType::Object { fields: b }) => {
            for (name, old) in a {
                match b.get(name) {
                    None => return Err(format!("object field {name} was removed")),
                    Some(new) => {
                        if old.optional && !new.optional {
                            return Err(format!("object field {name} became required"));
                        }
                        widening(&old.ty, &new.ty).map_err(|e| format!("{name}: {e}"))?;
                    }
                }
            }
            if let Some((name, _)) = b
                .iter()
                .find(|(n, f)| !a.contains_key(*n) && !f.optional)
            {
                return Err(format!("new required object field {name} isn't in existing values"));
            }
            widened("object")
        }
        (FieldType::Int { .. }, FieldType::Decimal { .. }) => widened("int to decimal"),
        (FieldType::Id | FieldType::Enum { .. } | FieldType::TimeZone, FieldType::String { .. }) => {
            Err("changing to a plain string may not keep existing values within its length; declare a new field instead".into())
        }
        _ => Err("the type changed; existing values may not fit. Add a new field and deprecate this one".into()),
    }
}

fn compare_routes(old: &[Route], new: &[Route], add: &mut Add<'_>) {
    let by_id = |routes: &[Route]| -> BTreeMap<String, Route> {
        routes.iter().map(|r| (r.id.clone(), r.clone())).collect()
    };
    let (old, new) = (by_id(old), by_id(new));
    for (id, route) in &old {
        let path = format!("routes.{id}");
        let Some(next) = new.get(id) else {
            add(
                path,
                ChangeKind::Breaking,
                format!(
                    "route {id} ({} {}) was removed; clients calling it would fail",
                    route.method.as_str(),
                    route.path
                ),
                route.deprecated.is_some(),
            );
            continue;
        };
        if route.method != next.method || route.path != next.path {
            add(
                path.clone(),
                ChangeKind::Breaking,
                format!(
                    "route {id} moved from {} {} to {} {}",
                    route.method.as_str(),
                    route.path,
                    next.method.as_str(),
                    next.path
                ),
                route.deprecated.is_some(),
            );
        }
        for (source, before, after) in [
            ("query", &route.input.query, &next.input.query),
            ("body", &route.input.body, &next.input.body),
        ] {
            for (name, field) in after {
                let required = !field.optional && field.default.is_none();
                if required && !before.contains_key(name) {
                    add(
                        format!("{path}.input.{source}.{name}"),
                        ChangeKind::Breaking,
                        format!(
                            "new required input {source}.{name}; existing clients don't send it"
                        ),
                        false,
                    );
                }
            }
            for name in before.keys().filter(|n| !after.contains_key(*n)) {
                add(
                    format!("{path}.input.{source}.{name}"),
                    ChangeKind::Breaking,
                    format!(
                        "input {source}.{name} was removed; clients sending it would be rejected"
                    ),
                    route.deprecated.is_some(),
                );
            }
        }
    }
    for id in new.keys().filter(|id| !old.contains_key(*id)) {
        add(
            format!("routes.{id}"),
            ChangeKind::Safe,
            format!("new route {id}"),
            false,
        );
    }
}
