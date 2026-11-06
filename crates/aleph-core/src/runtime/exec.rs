//! The plan interpreter. Runs one attempt of one plan (a route's or a handler's) inside one
//! transaction, charging the meter for everything it does.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use super::effects::{Delivery, DeliveryKind, Integrations, Journal, OutboxItem, Pending};
use super::env::Env;
use super::input::{Inputs, check_value, missing};
use super::meter::{Meter, OverBudget};
use crate::keys::{Key, KeyPart, Keyspace, TenantId};
use crate::manifest::{
    BinaryOp, Change, Counter, Entity, Expr, Field, IndexPart, InputSource, Manifest, Op, Order,
    Plan, QueryRange, Stmt, StrOp, StrTest, Trigger, VarId,
};
use crate::store::{Direction, StoreError, Transaction};
use crate::value::Value;

type Row = BTreeMap<String, Value>;

/// Why an attempt stopped early.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ExecError {
    /// The plan failed with a declared error. Not a bug: the transaction rolls back and the
    /// declared status is returned.
    #[error("failed with declared error {0:?}")]
    Failed(String),
    #[error(transparent)]
    OverBudget(#[from] OverBudget),
    #[error("{0} not found")]
    NotFound(String),
    #[error("{0} already exists")]
    Duplicate(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A write would give two rows the same value in a unique index (`Entity.index`).
    #[error("unique index {0} already has a row with this value")]
    Unique(String),
    /// A write would break an entity invariant (`Entity.name`).
    #[error("invariant {0} would be violated")]
    Invariant(String),
    /// An integration call failed.
    #[error("integration call failed: {0}")]
    CallFailed(String),
    /// The idempotency key was already used with different inputs.
    #[error("idempotency key reused with different inputs")]
    KeyReused,
    /// A request value the plan used turned out to be invalid (for example a forged cursor).
    #[error("{0}")]
    BadInput(String),
    /// Something the static checks can't rule out, such as integer overflow or a value that
    /// violates its field's constraints.
    #[error("{0}")]
    Runtime(String),
}

type R<T> = Result<T, ExecError>;

/// What a query scans.
struct Scan<'a> {
    index: Option<&'a str>,
    prefix: &'a [Expr],
    limit: &'a Expr,
    order: Order,
    range: Option<&'a QueryRange>,
}

fn runtime(message: impl Into<String>) -> ExecError {
    ExecError::Runtime(message.into())
}

pub(crate) struct Exec<'a> {
    pub(crate) manifest: &'a Manifest,
    pub(crate) plan: &'a Plan,
    pub(crate) tenant: &'a TenantId,
    pub(crate) integrations: &'a dyn Integrations,
    pub(crate) journal: &'a mut Journal,
    /// Effects buffered until commit.
    pub(crate) effects: Vec<Pending>,
    pub(crate) tx: &'a mut dyn Transaction,
    pub(crate) keys: &'a Keyspace,
    pub(crate) meter: &'a mut Meter,
    pub(crate) env: &'a dyn Env,
    pub(crate) now: i64,
    pub(crate) caller: Option<&'a super::Caller>,
    pub(crate) inputs: &'a Inputs,
    pub(crate) vars: HashMap<VarId, Value>,
    /// Off while evaluating counter predicates, which are part of the write they belong to.
    pub(crate) metered: bool,
}

impl<'a> Exec<'a> {
    /// Runs the plan and evaluates its output.
    pub(crate) fn run(&mut self) -> R<Value> {
        let plan = self.plan;
        self.block(&plan.body, false)?;
        self.eval(&plan.output)
    }

    fn block(&mut self, stmts: &'a [Stmt], pure: bool) -> R<()> {
        for stmt in stmts {
            self.stmt(stmt, pure)?;
        }
        Ok(())
    }

    fn stmt(&mut self, stmt: &'a Stmt, pure: bool) -> R<()> {
        self.meter.op()?;
        match stmt {
            Stmt::Let { var, op } => {
                let value = self.op(op, pure)?;
                self.vars.insert(*var, value);
            }
            Stmt::Ensure { cond, error } => {
                if !self.eval_bool(cond)? {
                    return Err(ExecError::Failed(error.clone()));
                }
            }
            Stmt::Fail { error } => return Err(ExecError::Failed(error.clone())),
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                let branch = if self.eval_bool(cond)? {
                    then
                } else {
                    otherwise
                };
                self.block(branch, pure)?;
            }
            Stmt::ForEach { list, item, body } => {
                for value in self.eval_list(list)? {
                    self.meter.op()?;
                    self.vars.insert(*item, value);
                    self.block(body, pure)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines, reason = "one arm per operation")]
    fn op(&mut self, op: &'a Op, pure: bool) -> R<Value> {
        self.meter.op()?;
        if pure && op.is_write() {
            // The checker rejects this. Refusing again here is defense in depth.
            return Err(runtime("write inside a read-only block"));
        }
        match op {
            Op::Get { entity, key } => {
                let e = self.entity(entity)?;
                let key = self.key_values(e, key)?;
                Ok(self
                    .read_row(entity, &key)?
                    .map_or(Value::Null, |(_, row)| Value::Object(row)))
            }
            Op::Insert { entity, values } => {
                let row = self.new_row(entity, values)?;
                if !self.insert_row(entity, &row)? {
                    return Err(ExecError::Duplicate(entity.clone()));
                }
                Ok(Value::Object(row))
            }
            Op::InsertIfAbsent { entity, values } => {
                let row = self.new_row(entity, values)?;
                Ok(Value::Bool(self.insert_row(entity, &row)?))
            }
            Op::Upsert { entity, values } => self.upsert(entity, values),
            Op::Update { entity, key, set } => self.update(entity, key, set),
            Op::Delete { entity, key } => self.delete(entity, key),
            Op::Query {
                entity,
                index,
                prefix,
                limit,
                order,
                range,
                after,
                page,
            } => {
                let after = match after {
                    Some(expr) => match self.eval(expr)? {
                        Value::Null => None,
                        Value::String(cursor) => Some(cursor),
                        other => {
                            return Err(ExecError::BadInput(format!(
                                "a cursor must be a string, got {}",
                                other.type_name()
                            )));
                        }
                    },
                    None => None,
                };
                let scan = Scan {
                    index: index.as_deref(),
                    prefix,
                    limit,
                    order: *order,
                    range: range.as_deref(),
                };
                let (rows, cursor) = self.query(entity, &scan, after.as_deref())?;
                Ok(if *page {
                    Value::Object(BTreeMap::from([
                        ("items".to_owned(), Value::List(rows)),
                        (
                            "cursor".to_owned(),
                            cursor.map_or(Value::Null, Value::String),
                        ),
                    ]))
                } else {
                    Value::List(rows)
                })
            }
            Op::CounterGet { counter, key } => {
                let values = key.iter().map(|e| self.eval(e)).collect::<R<Vec<_>>>()?;
                let key = self.keys.counter(counter, &parts(&values)?);
                let bytes = self.tx.get(&key)?;
                self.meter.read(bytes.as_ref().map_or(0, Vec::len))?;
                Ok(Value::Int(
                    bytes.map(|b| decode_counter(&b)).transpose()?.unwrap_or(0),
                ))
            }
            Op::Map {
                list,
                item,
                body,
                result,
            } => {
                let mut out = Vec::new();
                for value in self.eval_list(list)? {
                    self.meter.op()?;
                    self.vars.insert(*item, value);
                    self.block(body, true)?;
                    out.push(self.eval(result)?);
                }
                Ok(Value::List(out))
            }
            Op::Filter {
                list,
                item,
                body,
                keep,
            } => {
                let mut out = Vec::new();
                for value in self.eval_list(list)? {
                    self.meter.op()?;
                    self.vars.insert(*item, value.clone());
                    self.block(body, true)?;
                    if self.eval_bool(keep)? {
                        out.push(value);
                    }
                }
                Ok(Value::List(out))
            }
            Op::Emit { event, payload } => {
                let manifest = self.manifest;
                let decl = manifest
                    .events
                    .get(event)
                    .ok_or_else(|| runtime(format!("unknown event {event}")))?;
                let payload = self.field_values(&decl.payload, payload, event)?;
                for handler in manifest
                    .handlers
                    .iter()
                    .filter(|h| matches!(&h.trigger, Trigger::Event { event: e } if e == event))
                {
                    self.release(
                        0,
                        OutboxItem::Run {
                            handler: handler.id.clone(),
                            payload: payload.clone(),
                            attempt: 0,
                        },
                    )?;
                }
                Ok(Value::Null)
            }
            Op::Enqueue {
                queue,
                payload,
                delay_ms,
            } => {
                const MAX_DELAY_MS: i64 = 30 * 24 * 60 * 60 * 1000;
                let manifest = self.manifest;
                let decl = manifest
                    .queues
                    .get(queue)
                    .ok_or_else(|| runtime(format!("unknown queue {queue}")))?;
                let payload = self.field_values(&decl.payload, payload, queue)?;
                let delay = match delay_ms {
                    Some(expr) => self.eval_int(expr)?,
                    None => 0,
                };
                if !(0..=MAX_DELAY_MS).contains(&delay) {
                    return Err(runtime(format!(
                        "enqueue delay must be between 0 and {MAX_DELAY_MS} ms, got {delay}"
                    )));
                }
                let handler = manifest
                    .handlers
                    .iter()
                    .find(|h| matches!(&h.trigger, Trigger::Queue { queue: q } if q == queue))
                    .ok_or_else(|| runtime(format!("queue {queue} has no handler")))?;
                self.release(
                    delay,
                    OutboxItem::Run {
                        handler: handler.id.clone(),
                        payload,
                        attempt: 0,
                    },
                )?;
                Ok(Value::Null)
            }
            Op::Publish {
                channel,
                key,
                message,
            } => {
                let manifest = self.manifest;
                let decl = manifest
                    .channels
                    .get(channel)
                    .ok_or_else(|| runtime(format!("unknown channel {channel}")))?;
                let key = self.field_values(&decl.key, key, channel)?;
                let message = self.field_values(&decl.message, message, channel)?;
                let data = serde_json::json!({
                    "key": Value::Object(key).to_json(),
                    "message": Value::Object(message).to_json(),
                });
                let delivery = Delivery {
                    kind: DeliveryKind::Publish,
                    target: channel.clone(),
                    operation: None,
                    data,
                };
                self.release(
                    0,
                    OutboxItem::Deliver {
                        delivery,
                        attempt: 0,
                    },
                )?;
                Ok(Value::Null)
            }
            Op::Send {
                integration,
                operation,
                input,
            } => {
                let decl = self.operation(integration, operation)?;
                let input =
                    Value::Object(self.field_values(&decl.input, input, operation)?).to_json();
                let delivery = Delivery {
                    kind: DeliveryKind::Send,
                    target: integration.clone(),
                    operation: Some(operation.clone()),
                    data: input,
                };
                self.release(
                    0,
                    OutboxItem::Deliver {
                        delivery,
                        attempt: 0,
                    },
                )?;
                Ok(Value::Null)
            }
            Op::Call {
                integration,
                operation,
                input,
            } => {
                let decl = self.operation(integration, operation)?;
                let input =
                    Value::Object(self.field_values(&decl.input, input, operation)?).to_json();
                self.meter.call()?;
                let fingerprint = format!("{integration}.{operation}:{input}");
                let result = match self.journal.replay(&fingerprint) {
                    Some(result) => result,
                    None => {
                        let result = self
                            .integrations
                            .call(self.tenant, integration, operation, &input)
                            .and_then(|output| {
                                validate_output(&decl.output, output, integration, operation)
                            });
                        self.journal.record(fingerprint, result.clone());
                        result
                    }
                };
                result.map_err(ExecError::CallFailed)
            }
            Op::Eval { expr } => self.eval(expr),
        }
    }

    /// Buffers an effect until commit.
    fn release(&mut self, delay_ms: i64, item: OutboxItem) -> R<()> {
        self.meter.effect()?;
        self.effects.push(Pending { delay_ms, item });
        Ok(())
    }

    /// Evaluates values for declared fields, filling defaults and validating each.
    fn field_values(
        &mut self,
        fields: &'a BTreeMap<String, Field>,
        given: &'a BTreeMap<String, Expr>,
        context: &str,
    ) -> R<BTreeMap<String, Value>> {
        let mut out = BTreeMap::new();
        for (name, field) in fields {
            let value = match given.get(name) {
                Some(expr) => self.eval(expr)?,
                None => missing(field, self.env, self.now)
                    .ok_or_else(|| runtime(format!("{context}.{name} is required")))?,
            };
            check_value(field, &value).map_err(|m| runtime(format!("{context}.{name}: {m}")))?;
            out.insert(name.clone(), value);
        }
        Ok(out)
    }

    fn operation(
        &self,
        integration: &str,
        operation: &str,
    ) -> R<&'a crate::manifest::IntegrationOp> {
        self.manifest
            .integrations
            .get(integration)
            .and_then(|i| i.operations.get(operation))
            .ok_or_else(|| runtime(format!("unknown operation {integration}.{operation}")))
    }

    /// Releases one effect per handler of a row change.
    fn changed(
        &mut self,
        entity: &str,
        change: Change,
        before: Option<&Row>,
        after: Option<&Row>,
    ) -> R<()> {
        let manifest = self.manifest;
        for handler in manifest.handlers.iter().filter(|h| {
            matches!(&h.trigger, Trigger::Entity { entity: e, change: c } if e == entity && *c == change)
        }) {
            let row = |r: Option<&Row>| r.map_or(Value::Null, |r| Value::Object(r.clone()));
            let payload = BTreeMap::from([("before".to_owned(), row(before)), ("after".to_owned(), row(after))]);
            self.release(0, OutboxItem::Run { handler: handler.id.clone(), payload, attempt: 0 })?;
        }
        Ok(())
    }

    fn entity(&self, name: &str) -> R<&'a Entity> {
        self.manifest
            .entities
            .get(name)
            .ok_or_else(|| runtime(format!("unknown entity {name}")))
    }

    /// Evaluates a key map into values in primary key order.
    fn key_values(&mut self, entity: &Entity, key: &'a BTreeMap<String, Expr>) -> R<Vec<Value>> {
        entity
            .key
            .iter()
            .map(|name| {
                let expr = key
                    .get(name)
                    .ok_or_else(|| runtime(format!("missing key field {name}")))?;
                self.eval(expr)
            })
            .collect()
    }

    fn read_row(&mut self, entity: &str, key: &[Value]) -> R<Option<(Key, Row)>> {
        let row_key = self.keys.row(entity, &parts(key)?);
        let bytes = self.tx.get(&row_key)?;
        self.meter.read(bytes.as_ref().map_or(0, Vec::len))?;
        bytes.map(|b| Ok((row_key, decode_row(&b)?))).transpose()
    }

    /// Builds a complete new row: given values, then defaults, then `null` for optional fields.
    fn new_row(&mut self, entity_name: &str, values: &'a BTreeMap<String, Expr>) -> R<Row> {
        let entity = self.entity(entity_name)?;
        let mut row = Row::new();
        for (name, field) in &entity.fields {
            let value = match values.get(name) {
                Some(expr) => self.eval(expr)?,
                None => missing(field, self.env, self.now)
                    .ok_or_else(|| runtime(format!("{entity_name}.{name} is required")))?,
            };
            check_value(field, &value)
                .map_err(|m| runtime(format!("{entity_name}.{name}: {m}")))?;
            row.insert(name.clone(), value);
        }
        Ok(row)
    }

    /// Inserts `row` unless its key exists. Returns whether it inserted.
    fn insert_row(&mut self, entity_name: &str, row: &Row) -> R<bool> {
        let entity = self.entity(entity_name)?;
        let key = row_key_values(entity, row);
        if self.read_row(entity_name, &key)?.is_some() {
            return Ok(false);
        }
        self.write_new_row(entity_name, &key, row)?;
        Ok(true)
    }

    /// Writes a row known to be absent, with its index entries and counter groups.
    fn write_new_row(&mut self, entity_name: &str, key: &[Value], row: &Row) -> R<()> {
        let entity = self.entity(entity_name)?;
        self.write_row(entity_name, key, row)?;
        for (index_name, index) in &entity.indexes {
            if let Some((index_key, pk)) = self.index_entry(entity_name, entity, index_name, row)? {
                self.put_index_entry(entity_name, index_name, index.unique, index_key, pk)?;
            }
        }
        self.maintain_counters(entity_name, None, Some(row))?;
        self.changed(entity_name, Change::Inserted, None, Some(row))
    }

    fn upsert(&mut self, entity_name: &str, values: &'a BTreeMap<String, Expr>) -> R<Value> {
        let entity = self.entity(entity_name)?;
        let mut given = BTreeMap::new();
        for (name, expr) in values {
            given.insert(name.clone(), self.eval(expr)?);
        }
        let key: Vec<Value> = entity
            .key
            .iter()
            .map(|f| {
                given
                    .get(f)
                    .cloned()
                    .ok_or_else(|| runtime(format!("upsert needs key field {f}")))
            })
            .collect::<R<_>>()?;
        match self.read_row(entity_name, &key)? {
            Some((_, old)) => {
                let mut new = old.clone();
                for (name, value) in given {
                    if entity.key.contains(&name) {
                        continue;
                    }
                    let field = entity
                        .fields
                        .get(&name)
                        .ok_or_else(|| runtime(format!("unknown field {entity_name}.{name}")))?;
                    check_value(field, &value)
                        .map_err(|m| runtime(format!("{entity_name}.{name}: {m}")))?;
                    new.insert(name, value);
                }
                self.replace_row(entity_name, &key, &old, &new)?;
                Ok(Value::Object(new))
            }
            None => {
                let mut row = Row::new();
                for (name, field) in &entity.fields {
                    let value = match given.remove(name) {
                        Some(value) => value,
                        None => missing(field, self.env, self.now)
                            .ok_or_else(|| runtime(format!("{entity_name}.{name} is required")))?,
                    };
                    check_value(field, &value)
                        .map_err(|m| runtime(format!("{entity_name}.{name}: {m}")))?;
                    row.insert(name.clone(), value);
                }
                // The read above already established the row is absent.
                self.write_new_row(entity_name, &key, &row)?;
                Ok(Value::Object(row))
            }
        }
    }

    fn update(
        &mut self,
        entity_name: &str,
        key: &'a BTreeMap<String, Expr>,
        set: &'a BTreeMap<String, Expr>,
    ) -> R<Value> {
        let entity = self.entity(entity_name)?;
        let key = self.key_values(entity, key)?;
        let Some((_, old)) = self.read_row(entity_name, &key)? else {
            return Err(ExecError::NotFound(entity_name.to_owned()));
        };
        let mut new = old.clone();
        for (name, expr) in set {
            let field = entity
                .fields
                .get(name)
                .ok_or_else(|| runtime(format!("unknown field {entity_name}.{name}")))?;
            let value = self.eval(expr)?;
            check_value(field, &value)
                .map_err(|m| runtime(format!("{entity_name}.{name}: {m}")))?;
            new.insert(name.clone(), value);
        }
        self.replace_row(entity_name, &key, &old, &new)?;
        Ok(Value::Object(new))
    }

    /// Writes an updated row and moves its index entries and counter groups.
    fn replace_row(&mut self, entity_name: &str, key: &[Value], old: &Row, new: &Row) -> R<()> {
        let entity = self.entity(entity_name)?;
        self.write_row(entity_name, key, new)?;
        for (index_name, index) in &entity.indexes {
            let old_entry = self.index_entry(entity_name, entity, index_name, old)?;
            let new_entry = self.index_entry(entity_name, entity, index_name, new)?;
            if old_entry.as_ref().map(|e| &e.0) == new_entry.as_ref().map(|e| &e.0) {
                continue;
            }
            if let Some((old_key, _)) = old_entry {
                self.tx.delete(old_key);
                self.meter.write(1)?;
            }
            if let Some((new_key, pk)) = new_entry {
                self.put_index_entry(entity_name, index_name, index.unique, new_key, pk)?;
            }
        }
        self.maintain_counters(entity_name, Some(old), Some(new))?;
        self.changed(entity_name, Change::Updated, Some(old), Some(new))
    }

    fn delete(&mut self, entity_name: &str, key: &'a BTreeMap<String, Expr>) -> R<Value> {
        let entity = self.entity(entity_name)?;
        let key = self.key_values(entity, key)?;
        let Some((row_key, old)) = self.read_row(entity_name, &key)? else {
            return Ok(Value::Null);
        };
        self.tx.delete(row_key);
        self.meter.write(1)?;
        for index_name in entity.indexes.keys() {
            if let Some((index_key, _)) = self.index_entry(entity_name, entity, index_name, &old)? {
                self.tx.delete(index_key);
                self.meter.write(1)?;
            }
        }
        self.maintain_counters(entity_name, Some(&old), None)?;
        self.changed(entity_name, Change::Deleted, Some(&old), None)?;
        Ok(Value::Null)
    }

    fn write_row(&mut self, entity: &str, key: &[Value], row: &Row) -> R<()> {
        let manifest = self.manifest;
        if let Some(declared) = manifest.entities.get(entity) {
            for (name, invariant) in &declared.invariants {
                if self.eval_on_row(invariant.var, &invariant.expr, row)? != Value::Bool(true) {
                    return Err(ExecError::Invariant(format!("{entity}.{name}")));
                }
            }
        }
        let bytes = serde_json::to_vec(row).map_err(|e| runtime(e.to_string()))?;
        self.tx.put(self.keys.row(entity, &parts(key)?), bytes);
        self.meter.write(1)?;
        Ok(())
    }

    /// The key of a row's entry in an index, and the stored value (the row's primary key).
    /// The key and value of a row's entry in an index, or `None` if the row isn't indexed (a
    /// partial index's predicate excludes it, or a unique index part is `null`). Non-unique
    /// entries append the primary key so they're distinct; unique ones don't, so a second row
    /// with the same values collides.
    fn index_entry(
        &mut self,
        entity_name: &str,
        entity: &'a Entity,
        index_name: &str,
        row: &Row,
    ) -> R<Option<(Key, Vec<u8>)>> {
        let index = &entity.indexes[index_name];
        if let Some(filter) = &index.filter
            && self.eval_on_row(filter.var, &filter.expr, row)? != Value::Bool(true)
        {
            return Ok(None);
        }
        let mut values = Vec::with_capacity(index.fields.len());
        for part in &index.fields {
            values.push(match part {
                IndexPart::Field(name) => row.get(name).cloned().unwrap_or_default(),
                IndexPart::Computed(part) => self.eval_on_row(part.var, &part.expr, row)?,
            });
        }
        if index.unique && values.iter().any(Value::is_null) {
            return Ok(None);
        }
        let pk = row_key_values(entity, row);
        let pk_parts = if index.unique {
            Vec::new()
        } else {
            parts(&pk)?
        };
        let key = self
            .keys
            .index_entry(entity_name, index_name, &parts(&values)?, &pk_parts);
        let pk_bytes = serde_json::to_vec(&pk).map_err(|e| runtime(e.to_string()))?;
        Ok(Some((key, pk_bytes)))
    }

    /// Writes an index entry, first checking that a unique index has no other row with it.
    fn put_index_entry(
        &mut self,
        entity_name: &str,
        index_name: &str,
        unique: bool,
        key: Key,
        pk: Vec<u8>,
    ) -> R<()> {
        if unique {
            let existing = self.tx.get(&key)?;
            self.meter.read(existing.as_ref().map_or(0, Vec::len))?;
            if existing.is_some_and(|other| other != pk) {
                return Err(ExecError::Unique(format!("{entity_name}.{index_name}")));
            }
        }
        self.tx.put(key, pk);
        self.meter.write(1)?;
        Ok(())
    }

    /// Moves a row between counter groups after a write. Counter maintenance is part of the
    /// write's cost: it charges writes, not reads or ops.
    fn maintain_counters(&mut self, entity: &str, old: Option<&Row>, new: Option<&Row>) -> R<()> {
        let manifest = self.manifest;
        for (name, counter) in manifest.counters.iter().filter(|(_, c)| c.of == entity) {
            let old_part = old
                .map(|row| self.group(counter, row))
                .transpose()?
                .flatten();
            let new_part = new
                .map(|row| self.group(counter, row))
                .transpose()?
                .flatten();
            match (old_part, new_part) {
                (Some((g1, a1)), Some((g2, a2))) if g1 == g2 => {
                    if a1 != a2 {
                        let delta = a2
                            .checked_sub(a1)
                            .ok_or_else(|| runtime("counter overflow"))?;
                        self.bump(name, &g1, delta)?;
                    }
                }
                (old_part, new_part) => {
                    if let Some((group, amount)) = old_part {
                        self.bump(
                            name,
                            &group,
                            amount
                                .checked_neg()
                                .ok_or_else(|| runtime("counter overflow"))?,
                        )?;
                    }
                    if let Some((group, amount)) = new_part {
                        self.bump(name, &group, amount)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Evaluates a row expression without metering: it's part of the write it belongs to.
    fn eval_on_row(&mut self, var: VarId, expr: &'a Expr, row: &Row) -> R<Value> {
        let saved = std::mem::take(&mut self.vars);
        self.vars.insert(var, Value::Object(row.clone()));
        self.metered = false;
        let value = self.eval(expr);
        self.metered = true;
        self.vars = saved;
        value
    }

    /// The counter group a row belongs to and its contribution (1, or the `sum` expression), or
    /// `None` if the counter's predicate excludes it.
    fn group(&mut self, counter: &'a Counter, row: &Row) -> R<Option<(Vec<Value>, i64)>> {
        if let Some(predicate) = &counter.filter
            && self.eval_on_row(predicate.var, &predicate.expr, row)? != Value::Bool(true)
        {
            return Ok(None);
        }
        let amount = match &counter.sum {
            None => 1,
            Some(sum) => match self.eval_on_row(sum.var, &sum.expr, row)? {
                Value::Int(i) => i,
                Value::Null => 0,
                other => {
                    return Err(runtime(format!(
                        "counter sum must be an int, got {}",
                        other.type_name()
                    )));
                }
            },
        };
        let group = counter
            .group_by
            .iter()
            .map(|f| row.get(f).cloned().unwrap_or_default())
            .collect();
        Ok(Some((group, amount)))
    }

    fn bump(&mut self, counter: &str, group: &[Value], delta: i64) -> R<()> {
        let key = self.keys.counter(counter, &parts(group)?);
        let current = self
            .tx
            .get(&key)?
            .map(|b| decode_counter(&b))
            .transpose()?
            .unwrap_or(0);
        let next = current
            .checked_add(delta)
            .ok_or_else(|| runtime("counter overflow"))?;
        if next == 0 {
            self.tx.delete(key);
        } else {
            self.tx.put(key, next.to_string().into_bytes());
        }
        self.meter.write(1)?;
        Ok(())
    }

    /// Scans a bounded range of rows, optionally resuming after a cursor. Returns the rows and,
    /// if the page is full, a cursor for the next one.
    fn query(
        &mut self,
        entity_name: &str,
        scan: &Scan<'a>,
        after: Option<&str>,
    ) -> R<(Vec<Value>, Option<String>)> {
        let values = scan
            .prefix
            .iter()
            .map(|e| self.eval(e))
            .collect::<R<Vec<_>>>()?;
        let limit = usize::try_from(self.eval_int(scan.limit)?.max(0)).unwrap_or(usize::MAX);
        let (direction, forward) = match scan.order {
            Order::Asc => (Direction::Forward, true),
            Order::Desc => (Direction::Reverse, false),
        };
        let full = match scan.index {
            None => self.keys.row_range(entity_name, &parts(&values)?),
            Some(index) => self.keys.index_range(entity_name, index, &parts(&values)?),
        };
        // Bounds on the next part narrow the scan; cursors stay relative to the full prefix range.
        let bounded = match scan.range {
            None => full.clone(),
            Some(range) => {
                // Like SQL, a comparison never matches `null`: with no lower bound, a range still
                // starts after the null values (which sort first).
                let lower = self
                    .bound(range.gt.as_ref(), range.gte.as_ref())?
                    .or(Some((Value::Null, false)));
                let upper = self.bound(range.lt.as_ref(), range.lte.as_ref())?;
                full.bounded(key_bound(lower.as_ref()), key_bound(upper.as_ref()))
            }
        };
        let range = match after {
            None => bounded.clone(),
            Some(cursor) => full
                .resume(cursor, forward)
                .ok_or_else(|| ExecError::BadInput("invalid cursor".into()))?
                .intersect(&bounded),
        };
        let entries = if range.start < range.end {
            self.tx.scan(&range, limit, direction)?
        } else {
            Vec::new()
        };
        let cursor = match entries.last() {
            Some((last, _)) if entries.len() == limit => full.cursor_after(last),
            _ => None,
        };
        let mut rows = Vec::with_capacity(entries.len());
        for (_, bytes) in entries {
            let bytes = match scan.index {
                None => bytes,
                Some(index) => {
                    let pk: Vec<Value> = serde_json::from_slice(&bytes)
                        .map_err(|e| runtime(format!("corrupt index entry: {e}")))?;
                    let row_key = self.keys.row(entity_name, &parts(&pk)?);
                    self.tx.get(&row_key)?.ok_or_else(|| {
                        runtime(format!("index {index} points at a missing {entity_name}"))
                    })?
                }
            };
            self.meter.read(bytes.len())?;
            rows.push(Value::Object(decode_row(&bytes)?));
        }
        Ok((rows, cursor))
    }

    /// Evaluates one side of a range: the exclusive bound if given, else the inclusive one.
    fn bound(
        &mut self,
        exclusive: Option<&'a Expr>,
        inclusive: Option<&'a Expr>,
    ) -> R<Option<(Value, bool)>> {
        let (expr, is_inclusive) = match (exclusive, inclusive) {
            (Some(e), _) => (e, false),
            (None, Some(e)) => (e, true),
            (None, None) => return Ok(None),
        };
        match self.eval(expr)? {
            Value::List(_) | Value::Object(_) => Err(runtime(
                "a range bound must be a string, int, bool, or null",
            )),
            value => Ok(Some((value, is_inclusive))),
        }
    }

    fn eval_bool(&mut self, expr: &'a Expr) -> R<bool> {
        match self.eval(expr)? {
            Value::Bool(b) => Ok(b),
            other => Err(runtime(format!("expected bool, got {}", other.type_name()))),
        }
    }

    /// Evaluates a string operand. `None` means `null`: string operations propagate it, so
    /// optional fields stay optional (and unique indexes over them skip rows without a value).
    fn eval_opt_string(&mut self, expr: &'a Expr) -> R<Option<String>> {
        match self.eval(expr)? {
            Value::String(s) => Ok(Some(s)),
            Value::Null => Ok(None),
            other => Err(runtime(format!(
                "expected string, got {}",
                other.type_name()
            ))),
        }
    }

    fn eval_int(&mut self, expr: &'a Expr) -> R<i64> {
        match self.eval(expr)? {
            Value::Int(i) => Ok(i),
            other => Err(runtime(format!("expected int, got {}", other.type_name()))),
        }
    }

    fn eval_list(&mut self, expr: &'a Expr) -> R<Vec<Value>> {
        match self.eval(expr)? {
            Value::List(items) => Ok(items),
            other => Err(runtime(format!("expected list, got {}", other.type_name()))),
        }
    }

    #[allow(clippy::too_many_lines, reason = "one arm per expression kind")]
    fn eval(&mut self, expr: &'a Expr) -> R<Value> {
        if self.metered {
            self.meter.op()?;
        }
        Ok(match expr {
            Expr::Lit { value } => Value::from(value),
            Expr::Var { id } => self
                .vars
                .get(id)
                .cloned()
                .ok_or_else(|| runtime(format!("variable {id} isn't defined")))?,
            Expr::Input { source, name } => {
                let inputs = match source {
                    InputSource::Params => &self.inputs.params,
                    InputSource::Query => &self.inputs.query,
                    InputSource::Body => &self.inputs.body,
                    InputSource::Payload => &self.inputs.payload,
                };
                inputs.get(name).cloned().unwrap_or_default()
            }
            Expr::Caller => self
                .caller
                .and_then(|c| c.user_id.clone())
                .map_or(Value::Null, Value::String),
            Expr::CallerRole { org } => {
                let org = self.eval(org)?;
                match (&org, self.caller) {
                    (Value::String(org), Some(caller)) => caller
                        .orgs
                        .get(org)
                        .cloned()
                        .map_or(Value::Null, Value::String),
                    _ => Value::Null,
                }
            }
            Expr::Now => Value::Int(self.now),
            Expr::Field { of, name } => match self.eval(of)? {
                Value::Object(mut fields) => fields.remove(name).unwrap_or_default(),
                // Reading a field of null is null, so optional rows can be inspected safely.
                Value::Null => Value::Null,
                other => {
                    return Err(runtime(format!(
                        "can't read field {name} of {}",
                        other.type_name()
                    )));
                }
            },
            Expr::IsNull { of } => Value::Bool(self.eval(of)?.is_null()),
            Expr::Not { arg } => Value::Bool(!self.eval_bool(arg)?),
            Expr::Neg { arg } => Value::Int(
                self.eval_int(arg)?
                    .checked_neg()
                    .ok_or_else(|| runtime("integer overflow"))?,
            ),
            Expr::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs)?,
            Expr::In { value, list } => {
                let value = self.eval(value)?;
                Value::Bool(list.iter().any(|l| Value::from(l) == value))
            }
            Expr::Select {
                cond,
                then,
                otherwise,
            } => {
                let branch = if self.eval_bool(cond)? {
                    then
                } else {
                    otherwise
                };
                self.eval(branch)?
            }
            Expr::Match { on, cases } => {
                let on = match self.eval(on)? {
                    Value::String(s) => s,
                    other => {
                        return Err(runtime(format!(
                            "can only match strings, got {}",
                            other.type_name()
                        )));
                    }
                };
                let case = cases
                    .get(&on)
                    .or_else(|| cases.get(crate::check::DEFAULT_CASE))
                    .ok_or_else(|| runtime(format!("no match case for {on:?}")))?;
                self.eval(case)?
            }
            Expr::Object { fields } => {
                let mut out = BTreeMap::new();
                for (name, value) in fields {
                    out.insert(name.clone(), self.eval(value)?);
                }
                Value::Object(out)
            }
            Expr::List { items } => {
                Value::List(items.iter().map(|i| self.eval(i)).collect::<R<_>>()?)
            }
            Expr::First { of } => match self.eval(of)? {
                Value::List(items) => items.into_iter().next().unwrap_or_default(),
                Value::Null => Value::Null,
                other => {
                    return Err(runtime(format!(
                        "can't take the first item of {}",
                        other.type_name()
                    )));
                }
            },
            Expr::Length { of } => {
                let len = match self.eval(of)? {
                    Value::List(items) => items.len(),
                    Value::String(s) => s.chars().count(),
                    other => {
                        return Err(runtime(format!(
                            "can't take the length of {}",
                            other.type_name()
                        )));
                    }
                };
                Value::Int(i64::try_from(len).unwrap_or(i64::MAX))
            }
            Expr::Str { op, of } => match self.eval_opt_string(of)? {
                None => Value::Null,
                Some(text) => Value::String(match op {
                    StrOp::Lower => text.to_lowercase(),
                    StrOp::Upper => text.to_uppercase(),
                    StrOp::Trim => text.trim().to_owned(),
                }),
            },
            Expr::StrTest { op, of, arg } => {
                match (self.eval_opt_string(of)?, self.eval_opt_string(arg)?) {
                    (Some(text), Some(other)) => Value::Bool(match op {
                        StrTest::StartsWith => text.starts_with(&other),
                        StrTest::EndsWith => text.ends_with(&other),
                        StrTest::Contains => text.contains(&other),
                    }),
                    _ => Value::Bool(false),
                }
            }
            Expr::Truncate { of, max } => match self.eval_opt_string(of)? {
                None => Value::Null,
                Some(text) => Value::String(text.chars().take(*max as usize).collect()),
            },
            Expr::Abs { arg } => Value::Int(
                self.eval_int(arg)?
                    .checked_abs()
                    .ok_or_else(|| runtime("integer overflow"))?,
            ),
            Expr::Concat { parts } => {
                let mut out = String::new();
                for part in parts {
                    out.push_str(&self.eval(part)?.to_string());
                }
                Value::String(out)
            }
        })
    }

    fn binary(&mut self, op: BinaryOp, lhs: &'a Expr, rhs: &'a Expr) -> R<Value> {
        // Short-circuit like every mainstream language. Evaluating less never exceeds the bound.
        match op {
            BinaryOp::And => return Ok(Value::Bool(self.eval_bool(lhs)? && self.eval_bool(rhs)?)),
            BinaryOp::Or => return Ok(Value::Bool(self.eval_bool(lhs)? || self.eval_bool(rhs)?)),
            _ => {}
        }
        let (a, b) = (self.eval(lhs)?, self.eval(rhs)?);
        if op.is_comparison() {
            let result = match op {
                BinaryOp::Eq => a == b,
                BinaryOp::Neq => a != b,
                BinaryOp::Lt => compare(&a, &b)? == Ordering::Less,
                BinaryOp::Lte => compare(&a, &b)? != Ordering::Greater,
                BinaryOp::Gt => compare(&a, &b)? == Ordering::Greater,
                _ => compare(&a, &b)? != Ordering::Less,
            };
            return Ok(Value::Bool(result));
        }
        let (Value::Int(x), Value::Int(y)) = (&a, &b) else {
            return Err(runtime(format!(
                "arithmetic needs ints, got {} and {}",
                a.type_name(),
                b.type_name()
            )));
        };
        let result = match op {
            BinaryOp::Add => x.checked_add(*y),
            BinaryOp::Sub => x.checked_sub(*y),
            BinaryOp::Mul => x.checked_mul(*y),
            BinaryOp::Div if *y == 0 => return Err(runtime("division by zero")),
            BinaryOp::Div => x.checked_div(*y),
            BinaryOp::Min => Some(*x.min(y)),
            _ => Some(*x.max(y)),
        };
        result
            .map(Value::Int)
            .ok_or_else(|| runtime("integer overflow"))
    }
}

fn compare(a: &Value, b: &Value) -> R<Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        _ => Err(runtime(format!(
            "can't order {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn parts(values: &[Value]) -> R<Vec<KeyPart<'_>>> {
    values
        .iter()
        .map(|v| {
            KeyPart::from_value(v)
                .ok_or_else(|| runtime(format!("a {} can't be part of a key", v.type_name())))
        })
        .collect()
}

fn row_key_values(entity: &Entity, row: &Row) -> Vec<Value> {
    entity
        .key
        .iter()
        .map(|f| row.get(f).cloned().unwrap_or_default())
        .collect()
}

/// A range bound as a key part (lists and objects aren't orderable, so they bound nothing).
fn key_bound(bound: Option<&(Value, bool)>) -> Option<(KeyPart<'_>, bool)> {
    let (value, inclusive) = bound?;
    Some((KeyPart::from_value(value)?, *inclusive))
}

fn decode_row(bytes: &[u8]) -> R<Row> {
    serde_json::from_slice(bytes).map_err(|e| runtime(format!("corrupt row: {e}")))
}

fn decode_counter(bytes: &[u8]) -> R<i64> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| runtime("corrupt counter value"))
}

/// Validates an integration's raw output against the declared output fields.
fn validate_output(
    fields: &BTreeMap<String, Field>,
    output: serde_json::Value,
    integration: &str,
    operation: &str,
) -> Result<Value, String> {
    let serde_json::Value::Object(mut object) = output else {
        return Err(format!(
            "{integration}.{operation} returned something other than an object"
        ));
    };
    let mut out = BTreeMap::new();
    for (name, field) in fields {
        let value = match object.remove(name) {
            Some(json) => Value::from_json(json)
                .map_err(|e| format!("{integration}.{operation} output {name}: {e}"))?,
            None => Value::Null,
        };
        check_value(field, &value)
            .map_err(|e| format!("{integration}.{operation} output {name}: {e}"))?;
        out.insert(name.clone(), value);
    }
    Ok(Value::Object(out))
}
