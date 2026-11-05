//! The plan interpreter. Runs one attempt of one route inside one transaction, charging the meter
//! for everything it does.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use super::env::Env;
use super::input::{Inputs, check_value, missing};
use super::meter::{Meter, OverBudget};
use crate::keys::{Key, KeyPart, Keyspace};
use crate::manifest::{
    BinaryOp, Counter, Entity, Expr, InputSource, Manifest, Op, Order, Route, Stmt, VarId,
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
}

fn runtime(message: impl Into<String>) -> ExecError {
    ExecError::Runtime(message.into())
}

pub(crate) struct Exec<'a> {
    pub(crate) manifest: &'a Manifest,
    pub(crate) route: &'a Route,
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
    /// Runs the route's plan and evaluates its output.
    pub(crate) fn run(&mut self) -> R<Value> {
        let route = self.route;
        self.block(&route.plan.body, false)?;
        self.eval(&route.plan.output)
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
            Op::Eval { expr } => self.eval(expr),
        }
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
        for index_name in entity.indexes.keys() {
            let (index_key, pk) = self.index_entry(entity_name, entity, index_name, row)?;
            self.tx.put(index_key, pk);
            self.meter.write(1)?;
        }
        self.maintain_counters(entity_name, None, Some(row))
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
        for index_name in entity.indexes.keys() {
            let (old_key, _) = self.index_entry(entity_name, entity, index_name, old)?;
            let (new_key, pk) = self.index_entry(entity_name, entity, index_name, new)?;
            if old_key != new_key {
                self.tx.delete(old_key);
                self.tx.put(new_key, pk);
                self.meter.write(2)?;
            }
        }
        self.maintain_counters(entity_name, Some(old), Some(new))?;
        Ok(())
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
            let (index_key, _) = self.index_entry(entity_name, entity, index_name, &old)?;
            self.tx.delete(index_key);
            self.meter.write(1)?;
        }
        self.maintain_counters(entity_name, Some(&old), None)?;
        Ok(Value::Null)
    }

    fn write_row(&mut self, entity: &str, key: &[Value], row: &Row) -> R<()> {
        let bytes = serde_json::to_vec(row).map_err(|e| runtime(e.to_string()))?;
        self.tx.put(self.keys.row(entity, &parts(key)?), bytes);
        self.meter.write(1)?;
        Ok(())
    }

    /// The key of a row's entry in an index, and the stored value (the row's primary key).
    fn index_entry(
        &self,
        entity_name: &str,
        entity: &Entity,
        index_name: &str,
        row: &Row,
    ) -> R<(Key, Vec<u8>)> {
        let index = &entity.indexes[index_name];
        let values: Vec<Value> = index
            .fields
            .iter()
            .map(|f| row.get(f).cloned().unwrap_or_default())
            .collect();
        let pk = row_key_values(entity, row);
        let key = self
            .keys
            .index_entry(entity_name, index_name, &parts(&values)?, &parts(&pk)?);
        let pk_bytes = serde_json::to_vec(&pk).map_err(|e| runtime(e.to_string()))?;
        Ok((key, pk_bytes))
    }

    /// Moves a row between counter groups after a write. Counter maintenance is part of the
    /// write's cost: it charges writes, not reads or ops.
    fn maintain_counters(&mut self, entity: &str, old: Option<&Row>, new: Option<&Row>) -> R<()> {
        let manifest = self.manifest;
        for (name, counter) in manifest.counters.iter().filter(|(_, c)| c.of == entity) {
            let old_group = old
                .map(|row| self.group(counter, row))
                .transpose()?
                .flatten();
            let new_group = new
                .map(|row| self.group(counter, row))
                .transpose()?
                .flatten();
            if old_group == new_group {
                continue;
            }
            if let Some(group) = old_group {
                self.bump(name, &group, -1)?;
            }
            if let Some(group) = new_group {
                self.bump(name, &group, 1)?;
            }
        }
        Ok(())
    }

    /// The counter group a row belongs to, or `None` if the counter's predicate excludes it.
    fn group(&mut self, counter: &'a Counter, row: &Row) -> R<Option<Vec<Value>>> {
        if let Some(predicate) = &counter.filter {
            let saved = std::mem::take(&mut self.vars);
            self.vars.insert(predicate.var, Value::Object(row.clone()));
            self.metered = false;
            let included = self.eval_bool(&predicate.expr);
            self.metered = true;
            self.vars = saved;
            if !included? {
                return Ok(None);
            }
        }
        Ok(Some(
            counter
                .group_by
                .iter()
                .map(|f| row.get(f).cloned().unwrap_or_default())
                .collect(),
        ))
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
        let range = match after {
            None => full.clone(),
            Some(cursor) => full
                .resume(cursor, forward)
                .ok_or_else(|| ExecError::BadInput("invalid cursor".into()))?,
        };
        let entries = self.tx.scan(&range, limit, direction)?;
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

    fn eval_bool(&mut self, expr: &'a Expr) -> R<bool> {
        match self.eval(expr)? {
            Value::Bool(b) => Ok(b),
            other => Err(runtime(format!("expected bool, got {}", other.type_name()))),
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

fn decode_row(bytes: &[u8]) -> R<Row> {
    serde_json::from_slice(bytes).map_err(|e| runtime(format!("corrupt row: {e}")))
}

fn decode_counter(bytes: &[u8]) -> R<i64> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| runtime("corrupt counter value"))
}
