//! The plan interpreter. Runs one attempt of one plan (a route's or a handler's) inside one
//! transaction, charging the meter for everything it does.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use super::blobs::{BlobAccess, BlobGrant};
use super::builtins;
use super::effects::{Delivery, DeliveryKind, Integrations, Journal, OutboxItem, Pending};
use super::env::Env;
use super::input::{Inputs, conform, missing};
use super::meter::{Meter, OverBudget};
use crate::keys::{Key, KeyPart, Keyspace, TenantId};
use crate::manifest::{
    AggOp, BinaryOp, BucketDecl, Change, Counter, Entity, Expr, Field, FieldType, IndexPart,
    InputSource, Manifest, Op, Order, Plan, QueryRange, Stmt, StrOp, StrTest, Trigger, VarId,
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
    Failed(String, Option<Value>),
    /// The plan returned early (`return`). Not an error: the transaction commits.
    #[error("returned")]
    Returned(Value),
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
    /// A conditional update found a different version (`Entity`).
    #[error("{0} was changed by someone else")]
    VersionConflict(String),
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

/// One side of an aggregate counter change: (group, value, primary key).
type AggSide = (Vec<Value>, Value, Vec<Value>);

/// Scanned rows with their keys, and the cursor for the next page.
type ScanPage = (Vec<(Key, Value)>, Option<String>);

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
    /// The errors the plan may fail with (its route's or handler's).
    pub(crate) errors: &'a BTreeMap<String, crate::manifest::ErrorDecl>,
    pub(crate) tenant: &'a TenantId,
    pub(crate) integrations: &'a dyn Integrations,
    /// Signs blob URLs; `None` if the runtime has no file storage.
    pub(crate) blobs: Option<&'a super::BlobSigner>,
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
    /// Response headers, status, cookies, and logs the plan set.
    pub(crate) meta: ResponseMeta,
    /// The workflow whose step this is, for state updates.
    pub(crate) workflow: Option<&'a crate::manifest::WorkflowDecl>,
    /// State fields a workflow step set.
    pub(crate) state_updates: BTreeMap<String, Value>,
    /// Where this execution sits in a chain of handlers.
    pub(crate) chain: Chain<'a>,
}

/// Where an execution sits in a chain of handlers set off by one request or schedule.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Chain<'a> {
    /// Handler hops that led here (0 for a request, a schedule, or a workflow step). Handler
    /// runs this execution releases are one hop further.
    pub(crate) hops: u32,
    /// For a queue job: its handler and how many times in a row it re-enqueued itself.
    /// Enqueueing the same queue again is a continuation: the same hop, one more turn.
    pub(crate) continuing: Option<(&'a str, u32)>,
}

/// What a plan set about its response besides the body.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResponseMeta {
    pub(crate) status: Option<u16>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) cookies: Vec<String>,
    pub(crate) logs: Vec<LogEntry>,
    /// How a workflow step ended.
    pub(crate) transition: Option<super::workflow::Transition>,
}

/// One `log` statement's output.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct LogEntry {
    /// The label.
    pub label: String,
    /// The value.
    #[cfg_attr(feature = "codegen", ts(type = "unknown"))]
    pub value: serde_json::Value,
}

impl<'a> Exec<'a> {
    /// Runs the plan and evaluates its output (or the value of an early `return`).
    pub(crate) fn run(&mut self) -> R<Value> {
        let plan = self.plan;
        let value = match self.block(&plan.body, false) {
            Ok(()) => self.eval(&plan.output)?,
            Err(ExecError::Returned(value)) => value,
            Err(e) => return Err(e),
        };
        self.meter.memory(value.size())?;
        Ok(value)
    }

    /// The error a failing `fail` or `ensure` raises, with its evaluated details.
    fn failure(&mut self, error: &str, details: &'a BTreeMap<String, Expr>) -> R<ExecError> {
        let declared = self.errors.get(error);
        if details.is_empty() {
            return Ok(ExecError::Failed(error.to_owned(), None));
        }
        let mut out = BTreeMap::new();
        for (name, expr) in details {
            let value = self.eval(expr)?;
            let value = match declared.and_then(|d| d.details.get(name)) {
                Some(field) => conform(field, value)
                    .map_err(|m| runtime(format!("{error} details.{name}: {m}")))?,
                None => value,
            };
            out.insert(name.clone(), value);
        }
        Ok(ExecError::Failed(
            error.to_owned(),
            Some(Value::Object(out)),
        ))
    }

    fn block(&mut self, stmts: &'a [Stmt], pure: bool) -> R<()> {
        for stmt in stmts {
            self.stmt(stmt, pure)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines, reason = "one arm per statement kind")]
    fn stmt(&mut self, stmt: &'a Stmt, pure: bool) -> R<()> {
        self.meter.op()?;
        match stmt {
            Stmt::Let { var, op } => {
                let value = self.op(op, pure)?;
                self.meter.memory(value.size())?;
                self.vars.insert(*var, value);
            }
            Stmt::Ensure {
                cond,
                error,
                details,
            } => {
                if !self.eval_bool(cond)? {
                    return Err(self.failure(error, details)?);
                }
            }
            Stmt::Fail { error, details } => return Err(self.failure(error, details)?),
            Stmt::Return { value } => return Err(ExecError::Returned(self.eval(value)?)),
            Stmt::SetHeader { name, value } => {
                let value = match self.eval(value)? {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                if value.contains(['\r', '\n']) {
                    return Err(runtime("header values can't contain line breaks"));
                }
                if name == "set-cookie" {
                    self.meta.cookies.push(value);
                } else {
                    self.meta.headers.insert(name.clone(), value);
                }
            }
            Stmt::SetStatus { status } => self.meta.status = Some(*status),
            Stmt::Next {
                step,
                delay_ms,
                state,
            } => {
                self.update_state(state)?;
                let delay_ms = match delay_ms {
                    Some(expr) => self.eval_int(expr)?,
                    None => 0,
                };
                check_wait(delay_ms)?;
                self.meta.transition = Some(super::workflow::Transition::Next {
                    step: step.clone(),
                    delay_ms,
                });
                return Err(ExecError::Returned(Value::Null));
            }
            Stmt::WaitFor {
                signal,
                timeout_ms,
                then,
                on_timeout,
                state,
            } => {
                self.update_state(state)?;
                let timeout_ms = self.eval_int(timeout_ms)?;
                check_wait(timeout_ms)?;
                self.meta.transition = Some(super::workflow::Transition::WaitFor {
                    signal: signal.clone(),
                    timeout_ms,
                    then: then.clone(),
                    on_timeout: on_timeout.clone(),
                });
                return Err(ExecError::Returned(Value::Null));
            }
            Stmt::Complete { result, state } => {
                self.update_state(state)?;
                let result = match result {
                    Some(expr) => self.eval(expr)?,
                    None => Value::Null,
                };
                self.meta.transition = Some(super::workflow::Transition::Complete { result });
                return Err(ExecError::Returned(Value::Null));
            }
            Stmt::Log { label, value } => {
                let value = self.eval(value)?;
                self.meta.logs.push(LogEntry {
                    label: label.clone(),
                    value: value.to_json(),
                });
            }
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
                    self.meter.memory(value.size())?;
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
                Ok(match self.read_row(entity, &key)? {
                    Some((_, row)) if !self.expired(e, &row) => Value::Object(row),
                    _ => Value::Null,
                })
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
            Op::Update {
                entity,
                key,
                set,
                if_version,
            } => self.update(entity, key, set, if_version.as_deref()),
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
                any_of,
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
                let (rows, cursor) = match any_of {
                    Some(values) => (self.query_any(entity, &scan, values)?, None),
                    None => self.query(entity, &scan, after.as_deref())?,
                };
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
                self.counter_value(counter, &values)
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
                    self.meter.memory(value.size())?;
                    self.vars.insert(*item, value);
                    self.block(body, true)?;
                    out.push(self.eval(result)?);
                }
                Ok(Value::List(out))
            }
            Op::Loop {
                max,
                state,
                var,
                cond,
                body,
                next,
            } => {
                let mut current = BTreeMap::new();
                for (name, expr) in state {
                    current.insert(name.clone(), self.eval(expr)?);
                }
                for _ in 0..*max {
                    let value = Value::Object(current.clone());
                    self.meter.op()?;
                    self.meter.memory(value.size())?;
                    self.vars.insert(*var, value.clone());
                    if let Some(cond) = cond
                        && !self.eval_bool(cond)?
                    {
                        return Ok(value);
                    }
                    self.block(body, true)?;
                    current = BTreeMap::new();
                    for (name, expr) in next {
                        current.insert(name.clone(), self.eval(expr)?);
                    }
                }
                Ok(Value::Object(current))
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
                    self.meter.memory(value.size())?;
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
                            job: None,
                            chain: 0,
                            turns: 0,
                        },
                    )?;
                }
                Ok(Value::Null)
            }
            Op::Enqueue {
                queue,
                payload,
                delay_ms,
                key,
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
                let job = match key {
                    Some(expr) => Some(self.job_ref(queue, expr)?),
                    None => None,
                };
                self.release(
                    delay,
                    OutboxItem::Run {
                        handler: handler.id.clone(),
                        payload,
                        attempt: 0,
                        job,
                        chain: 0,
                        turns: 0,
                    },
                )?;
                Ok(Value::Null)
            }
            Op::StartWorkflow {
                workflow,
                key,
                input,
            } => {
                let decl = self.workflow_decl(workflow)?;
                let key = self.workflow_key(key)?;
                if self.read_instance(workflow, &key)?.is_some() {
                    return Ok(Value::Bool(false));
                }
                let input = self.field_values(&decl.input, input, workflow)?;
                let instance = super::workflow::Instance::new(decl, input);
                let bytes = serde_json::to_vec(&instance).map_err(|e| runtime(e.to_string()))?;
                if bytes.len() as u64 > self.meter.limits().max_row_bytes {
                    return Err(runtime("the workflow's input is too large to store"));
                }
                self.tx.put(self.keys.workflow(workflow, &key), bytes);
                self.meter.write(1)?;
                self.release(
                    0,
                    OutboxItem::Step {
                        workflow: workflow.clone(),
                        key,
                        seq: instance.seq,
                        attempt: 0,
                        signal: None,
                        error: None,
                    },
                )?;
                Ok(Value::Bool(true))
            }
            Op::SignalWorkflow {
                workflow,
                key,
                signal,
                payload,
            } => {
                let decl = self.workflow_decl(workflow)?;
                let key = self.workflow_key(key)?;
                let fields = decl
                    .signals
                    .get(signal)
                    .ok_or_else(|| runtime(format!("unknown signal {signal}")))?;
                let payload = Value::Object(self.field_values(fields, payload, signal)?);
                self.release(
                    0,
                    OutboxItem::Signal {
                        workflow: workflow.clone(),
                        key,
                        signal: signal.clone(),
                        payload,
                    },
                )?;
                Ok(Value::Null)
            }
            Op::GetWorkflow { workflow, key } => {
                let key = self.workflow_key(key)?;
                Ok(self
                    .read_instance(workflow, &key)?
                    .map_or(Value::Null, |i| i.view(&key)))
            }
            Op::CancelWorkflow { workflow, key } => {
                let key = self.workflow_key(key)?;
                self.release(
                    0,
                    OutboxItem::CancelWorkflow {
                        workflow: workflow.clone(),
                        key,
                    },
                )?;
                Ok(Value::Null)
            }
            Op::Cancel { queue, key } => {
                let job = self.job_ref(queue, key)?;
                self.release(0, OutboxItem::Cancel { job })?;
                Ok(Value::Null)
            }
            Op::GetUser { user_id } => {
                let Some(id) = self.eval_opt_string(user_id)? else {
                    return Ok(Value::Null);
                };
                let bytes = self.tx.get(&self.keys.user(&id))?;
                self.meter.read(bytes.as_ref().map_or(0, Vec::len))?;
                match bytes {
                    None => Ok(Value::Null),
                    Some(bytes) => {
                        let mut profile = decode_row(&bytes)?;
                        profile.insert("userId".into(), Value::String(id));
                        Ok(Value::Object(profile))
                    }
                }
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
            Op::UploadUrl {
                bucket,
                key,
                content_type,
                expires_s,
            } => {
                let max_bytes = self.bucket(bucket)?.max_bytes;
                let key = self.blob_key(key)?;
                let content_type = self
                    .eval_opt_string(content_type)?
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                if content_type.len() > BucketDecl::MAX_CONTENT_TYPE_LEN as usize
                    || !self.bucket(bucket)?.allows(&content_type)
                {
                    return Err(ExecError::BadInput(format!(
                        "bucket {bucket:?} doesn't accept {content_type:?} files"
                    )));
                }
                let grant = BlobGrant {
                    access: BlobAccess::Upload,
                    tenant: self.tenant.as_str().to_owned(),
                    bucket: bucket.clone(),
                    key: key.clone(),
                    content_type: Some(content_type),
                    max_bytes: Some(max_bytes),
                    expires_at: self.expires_at(*expires_s),
                };
                let url = self.sign_blob(&grant)?;
                Ok(Value::Object(BTreeMap::from([
                    ("url".to_owned(), Value::String(url)),
                    ("key".to_owned(), Value::String(key)),
                    ("expiresAt".to_owned(), Value::Int(grant.expires_at)),
                ])))
            }
            Op::DownloadUrl {
                bucket,
                key,
                expires_s,
            } => {
                self.bucket(bucket)?;
                let key = self.blob_key(key)?;
                let grant = BlobGrant {
                    access: BlobAccess::Download,
                    tenant: self.tenant.as_str().to_owned(),
                    bucket: bucket.clone(),
                    key,
                    content_type: None,
                    max_bytes: None,
                    expires_at: self.expires_at(*expires_s),
                };
                let url = self.sign_blob(&grant)?;
                Ok(Value::Object(BTreeMap::from([
                    ("url".to_owned(), Value::String(url)),
                    ("expiresAt".to_owned(), Value::Int(grant.expires_at)),
                ])))
            }
            Op::GetBlob { bucket, key } => {
                self.bucket(bucket)?;
                let key = self.blob_key(key)?;
                let found = super::blobs::read_info(&mut *self.tx, self.keys, bucket, &key)?;
                self.meter.read(found.as_ref().map_or(0, |(_, len)| *len))?;
                Ok(found.map_or(Value::Null, |(info, _)| info.to_value()))
            }
            Op::DeleteBlob { bucket, key } => {
                self.bucket(bucket)?;
                let key = self.blob_key(key)?;
                self.tx.delete(self.keys.blob(bucket, &key));
                self.meter.write(1)?;
                let delivery = Delivery {
                    kind: DeliveryKind::DeleteBlob,
                    target: bucket.clone(),
                    operation: None,
                    data: serde_json::json!({ "key": key }),
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

    /// Evaluates state updates of a workflow step into `self.state_updates`.
    fn update_state(&mut self, state: &'a BTreeMap<String, Expr>) -> R<()> {
        for (name, expr) in state {
            let value = self.eval(expr)?;
            let field = self
                .workflow
                .and_then(|w| w.state.get(name))
                .ok_or_else(|| runtime(format!("unknown workflow state field {name}")))?;
            let optional = Field {
                optional: true,
                ..field.clone()
            };
            let value =
                conform(&optional, value).map_err(|m| runtime(format!("state.{name}: {m}")))?;
            self.state_updates.insert(name.clone(), value);
        }
        Ok(())
    }

    /// Evaluates a workflow instance key.
    fn workflow_key(&mut self, key: &'a Expr) -> R<String> {
        match self.eval(key)? {
            Value::String(key) if !key.is_empty() && key.len() <= 256 => Ok(key),
            other => Err(runtime(format!(
                "a workflow key must be a string of 1-256 bytes, got {}",
                other.to_json()
            ))),
        }
    }

    fn workflow_decl(&self, workflow: &str) -> R<&'a crate::manifest::WorkflowDecl> {
        self.manifest
            .workflows
            .get(workflow)
            .ok_or_else(|| runtime(format!("unknown workflow {workflow}")))
    }

    /// Reads a workflow instance.
    fn read_instance(&mut self, workflow: &str, key: &str) -> R<Option<super::workflow::Instance>> {
        let bytes = self.tx.get(&self.keys.workflow(workflow, key))?;
        self.meter.read(bytes.as_ref().map_or(0, Vec::len))?;
        bytes
            .map(|b| {
                serde_json::from_slice(&b)
                    .map_err(|e| runtime(format!("corrupt workflow instance: {e}")))
            })
            .transpose()
    }

    /// A keyed job's reference: its queue and evaluated key.
    fn job_ref(&mut self, queue: &str, key: &'a Expr) -> R<super::effects::JobRef> {
        match self.eval(key)? {
            Value::String(key) if !key.is_empty() && key.len() <= 256 => {
                Ok(super::effects::JobRef {
                    queue: queue.to_owned(),
                    key,
                })
            }
            other => Err(runtime(format!(
                "a job key must be a string of 1-256 bytes, got {}",
                other.to_json()
            ))),
        }
    }

    /// Buffers an effect until commit.
    fn release(&mut self, delay_ms: i64, mut item: OutboxItem) -> R<()> {
        if let OutboxItem::Run {
            handler,
            chain,
            turns,
            ..
        } = &mut item
        {
            match self.chain.continuing {
                Some((own, done)) if own == handler.as_str() => {
                    *chain = self.chain.hops;
                    *turns = done + 1;
                }
                _ => *chain = self.chain.hops + 1,
            }
        }
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
            let value =
                conform(field, value).map_err(|m| runtime(format!("{context}.{name}: {m}")))?;
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
            self.release(0, OutboxItem::Run { handler: handler.id.clone(), payload, attempt: 0, job: None, chain: 0, turns: 0 })?;
        }
        Ok(())
    }

    fn bucket(&self, name: &str) -> R<&'a BucketDecl> {
        self.manifest
            .buckets
            .get(name)
            .ok_or_else(|| runtime(format!("unknown bucket {name}")))
    }

    /// Evaluates a file key, rejecting one no file could have.
    fn blob_key(&mut self, key: &'a Expr) -> R<String> {
        let key = self.eval_opt_string(key)?.unwrap_or_default();
        super::blobs::check_key(&key).map_err(ExecError::BadInput)?;
        Ok(key)
    }

    /// When a signed URL lasting `expires_s` seconds (or the default) expires.
    fn expires_at(&self, expires_s: Option<u32>) -> i64 {
        let seconds = expires_s
            .unwrap_or(BucketDecl::DEFAULT_EXPIRES_S)
            .clamp(1, BucketDecl::MAX_EXPIRES_S);
        self.now.saturating_add(i64::from(seconds) * 1000)
    }

    fn sign_blob(&self, grant: &BlobGrant) -> R<String> {
        let signer = self
            .blobs
            .ok_or_else(|| runtime("this runtime has no file storage configured"))?;
        let url = signer.sign(grant);
        if url.len() > BucketDecl::MAX_URL_LEN as usize {
            return Err(runtime("the signed URL is longer than its static bound"));
        }
        Ok(url)
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
                let value = self.eval(expr)?;
                Ok(key_value(entity.fields.get(name).map(|f| &f.ty), value))
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
            let value =
                conform(field, value).map_err(|m| runtime(format!("{entity_name}.{name}: {m}")))?;
            row.insert(name.clone(), value);
        }
        if entity.versioned {
            row.insert("version".into(), Value::Int(1));
        }
        Ok(row)
    }

    /// Inserts a row given as JSON (seed data): fields are validated, and missing ones take their
    /// defaults. Indexes, counters, and invariants are maintained; change handlers run as usual.
    pub(crate) fn seed_row(&mut self, entity_name: &str, json: &serde_json::Value) -> R<bool> {
        let entity = self.entity(entity_name)?;
        let serde_json::Value::Object(given) = json else {
            return Err(runtime(format!("{entity_name} seed rows must be objects")));
        };
        if let Some(unknown) = given.keys().find(|k| !entity.fields.contains_key(*k)) {
            return Err(runtime(format!("{entity_name} has no field {unknown}")));
        }
        let mut row = Row::new();
        for (name, field) in &entity.fields {
            let value = match given.get(name) {
                Some(json) => Value::from_json(json.clone()).map_err(runtime)?,
                None => missing(field, self.env, self.now)
                    .ok_or_else(|| runtime(format!("{entity_name}.{name} is required")))?,
            };
            let value =
                conform(field, value).map_err(|m| runtime(format!("{entity_name}.{name}: {m}")))?;
            row.insert(name.clone(), value);
        }
        if entity.versioned {
            row.insert("version".into(), Value::Int(1));
        }
        self.insert_row(entity_name, &row)
    }

    /// Inserts `row` unless its key exists. Returns whether it inserted.
    fn insert_row(&mut self, entity_name: &str, row: &Row) -> R<bool> {
        let entity = self.entity(entity_name)?;
        let key = row_key_values(entity, row);
        if let Some((row_key, old)) = self.read_row(entity_name, &key)? {
            if !self.expired(entity, &old) {
                return Ok(false);
            }
            // An expired row is gone as far as plans can tell: replace it.
            self.purge_row(entity_name, row_key, &old, false)?;
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
            let field = entity
                .fields
                .get(name)
                .ok_or_else(|| runtime(format!("unknown field {entity_name}.{name}")))?;
            let value = self.eval(expr)?;
            let value =
                conform(field, value).map_err(|m| runtime(format!("{entity_name}.{name}: {m}")))?;
            given.insert(name.clone(), value);
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
        let existing = match self.read_row(entity_name, &key)? {
            Some((row_key, old)) if self.expired(entity, &old) => {
                self.purge_row(entity_name, row_key, &old, false)?;
                None
            }
            other => other,
        };
        match existing {
            Some((_, old)) => {
                let mut new = old.clone();
                for (name, value) in given {
                    if entity.key.contains(&name) || entity.versioned && name == "version" {
                        continue;
                    }
                    new.insert(name, value);
                }
                if entity.versioned {
                    let version = next_version(&old);
                    new.insert("version".into(), version);
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
                    row.insert(name.clone(), value);
                }
                if entity.versioned {
                    row.insert("version".into(), Value::Int(1));
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
        if_version: Option<&'a Expr>,
    ) -> R<Value> {
        let entity = self.entity(entity_name)?;
        let key = self.key_values(entity, key)?;
        let old = match self.read_row(entity_name, &key)? {
            Some((_, old)) if !self.expired(entity, &old) => old,
            _ => return Err(ExecError::NotFound(entity_name.to_owned())),
        };
        if let Some(expected) = if_version {
            let expected = self.eval(expected)?;
            if !expected.is_null() && old.get("version") != Some(&expected) {
                return Err(ExecError::VersionConflict(entity_name.to_owned()));
            }
        }
        let mut new = old.clone();
        for (name, expr) in set {
            let field = entity
                .fields
                .get(name)
                .ok_or_else(|| runtime(format!("unknown field {entity_name}.{name}")))?;
            let value = self.eval(expr)?;
            let value =
                conform(field, value).map_err(|m| runtime(format!("{entity_name}.{name}: {m}")))?;
            new.insert(name.clone(), value);
        }
        if entity.versioned {
            let version = next_version(&old);
            new.insert("version".into(), version);
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
        let notify = !self.expired(entity, &old);
        self.purge_row(entity_name, row_key, &old, notify)?;
        Ok(Value::Null)
    }

    /// Removes a row with its index entries and counter contributions. With `notify`, delete
    /// handlers run.
    pub(crate) fn purge_row(
        &mut self,
        entity_name: &str,
        row_key: Key,
        old: &Row,
        notify: bool,
    ) -> R<()> {
        let entity = self.entity(entity_name)?;
        self.tx.delete(row_key);
        self.meter.write(1)?;
        for index_name in entity.indexes.keys() {
            if let Some((index_key, _)) = self.index_entry(entity_name, entity, index_name, old)? {
                self.tx.delete(index_key);
                self.meter.write(1)?;
            }
        }
        self.maintain_counters(entity_name, Some(old), None)?;
        if notify {
            self.changed(entity_name, Change::Deleted, Some(old), None)?;
        }
        Ok(())
    }

    /// Whether a row of a TTL entity has expired.
    pub(crate) fn expired(&self, entity: &Entity, row: &Row) -> bool {
        let Some(ttl) = &entity.ttl else {
            return false;
        };
        match row.get(&ttl.field) {
            Some(Value::Int(at)) => {
                at.saturating_add(i64::try_from(ttl.ms).unwrap_or(i64::MAX)) <= self.now
            }
            _ => false,
        }
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
            if let Some(agg) = &counter.agg {
                self.maintain_aggregate(name, counter, agg, entity, old, new)?;
                continue;
            }
            let old_part = old
                .map(|row| self.group(counter, row))
                .transpose()?
                .flatten();
            let new_part = new
                .map(|row| self.group(counter, row))
                .transpose()?
                .flatten();
            let shard = |row: Option<&Row>| -> Option<i64> {
                let shards = counter.shards?;
                let entity = manifest.entities.get(entity)?;
                Some(shard_of(&row_key_values(entity, row?), shards))
            };
            let (old_shard, new_shard) = (shard(old), shard(new));
            match (old_part, new_part) {
                (Some((g1, a1)), Some((g2, a2))) if g1 == g2 && old_shard == new_shard => {
                    if a1 != a2 {
                        let delta = a2
                            .checked_sub(a1)
                            .ok_or_else(|| runtime("counter overflow"))?;
                        self.bump(name, &g1, old_shard, delta)?;
                    }
                }
                (old_part, new_part) => {
                    if let Some((group, amount)) = old_part {
                        let delta = amount
                            .checked_neg()
                            .ok_or_else(|| runtime("counter overflow"))?;
                        self.bump(name, &group, old_shard, delta)?;
                    }
                    if let Some((group, amount)) = new_part {
                        self.bump(name, &group, new_shard, amount)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Keeps a min/max or distinct counter up to date for one row change.
    fn maintain_aggregate(
        &mut self,
        name: &str,
        counter: &'a Counter,
        agg: &crate::manifest::CounterAgg,
        entity: &str,
        old: Option<&Row>,
        new: Option<&Row>,
    ) -> R<()> {
        let manifest = self.manifest;
        let entity = manifest
            .entities
            .get(entity)
            .ok_or_else(|| runtime(format!("unknown entity {entity}")))?;
        // (group, value, primary key) of each side, if the row counts and has a value.
        let mut side = |row: Option<&Row>| -> R<Option<AggSide>> {
            let Some(row) = row else { return Ok(None) };
            let Some((group, _)) = self.group(counter, row)? else {
                return Ok(None);
            };
            let value = row.get(&agg.field).cloned().unwrap_or_default();
            Ok((!value.is_null()).then(|| (group, value, row_key_values(entity, row))))
        };
        let (before, after) = (side(old)?, side(new)?);
        if before == after {
            return Ok(());
        }
        match agg.kind {
            crate::manifest::AggKind::Min | crate::manifest::AggKind::Max => {
                // One ordered entry per row: (group, value, pk) → value.
                if let Some((group, value, pk)) = before {
                    let key = self.agg_entry(name, &group, &value, &pk)?;
                    self.tx.delete(key);
                    self.meter.write(1)?;
                }
                if let Some((group, value, pk)) = after {
                    let key = self.agg_entry(name, &group, &value, &pk)?;
                    let bytes = serde_json::to_vec(&value).map_err(|e| runtime(e.to_string()))?;
                    self.tx.put(key, bytes);
                    self.meter.write(1)?;
                }
            }
            crate::manifest::AggKind::Distinct => {
                // A refcount per (group, value), and the number of values with a refcount.
                if let Some((group, value, _)) = before {
                    self.distinct(name, &group, &value, -1)?;
                }
                if let Some((group, value, _)) = after {
                    self.distinct(name, &group, &value, 1)?;
                }
            }
        }
        Ok(())
    }

    fn agg_entry(&self, name: &str, group: &[Value], value: &Value, pk: &[Value]) -> R<Key> {
        let mut all: Vec<Value> = group.to_vec();
        all.push(value.clone());
        all.extend(pk.iter().cloned());
        Ok(self.keys.counter(name, &parts(&all)?))
    }

    fn distinct(&mut self, name: &str, group: &[Value], value: &Value, delta: i64) -> R<()> {
        let mut with_value = group.to_vec();
        with_value.push(value.clone());
        let refcount_key = self.keys.counter(name, &parts(&with_value)?);
        let current = self
            .tx
            .get(&refcount_key)?
            .map(|b| decode_counter(&b))
            .transpose()?
            .unwrap_or(0);
        let next = current.saturating_add(delta).max(0);
        if next == 0 {
            self.tx.delete(refcount_key);
        } else {
            self.tx.put(refcount_key, next.to_string().into_bytes());
        }
        self.meter.write(1)?;
        if (current == 0) != (next == 0) {
            self.bump(name, group, None, if next == 0 { -1 } else { 1 })?;
        }
        Ok(())
    }

    /// Reads a counter: a count or sum (summing shards), a min/max value, or a distinct count.
    fn counter_value(&mut self, name: &str, group: &[Value]) -> R<Value> {
        let manifest = self.manifest;
        let counter = manifest
            .counters
            .get(name)
            .ok_or_else(|| runtime(format!("unknown counter {name}")))?;
        let types: Vec<Option<&FieldType>> = counter
            .group_by
            .iter()
            .map(|part| match part {
                IndexPart::Field(f) => manifest
                    .entities
                    .get(&counter.of)
                    .and_then(|e| e.fields.get(f))
                    .map(|f| &f.ty),
                IndexPart::Computed(_) => None,
            })
            .collect();
        let group: Vec<Value> = group
            .iter()
            .enumerate()
            .map(|(i, v)| key_value(types.get(i).copied().flatten(), v.clone()))
            .collect();
        match counter.agg.as_ref().map(|a| a.kind) {
            Some(kind @ (crate::manifest::AggKind::Min | crate::manifest::AggKind::Max)) => {
                let range = self.keys.counter_range(name, &parts(&group)?);
                let direction = if kind == crate::manifest::AggKind::Min {
                    Direction::Forward
                } else {
                    Direction::Reverse
                };
                let entries = self.tx.scan(&range, 1, direction)?;
                let Some((_, bytes)) = entries.into_iter().next() else {
                    self.meter.read(0)?;
                    return Ok(Value::Null);
                };
                self.meter.read(bytes.len())?;
                serde_json::from_slice(&bytes).map_err(|e| runtime(format!("corrupt counter: {e}")))
            }
            _ => {
                let mut keys = Vec::new();
                match counter.shards {
                    Some(n) => {
                        for shard in 0..i64::from(n) {
                            let mut with_shard = group.clone();
                            with_shard.push(Value::Int(shard));
                            keys.push(self.keys.counter(name, &parts(&with_shard)?));
                        }
                    }
                    None => keys.push(self.keys.counter(name, &parts(&group)?)),
                }
                let mut total: i64 = 0;
                for key in keys {
                    let bytes = self.tx.get(&key)?;
                    self.meter.read(bytes.as_ref().map_or(0, Vec::len))?;
                    let n = bytes.map(|b| decode_counter(&b)).transpose()?.unwrap_or(0);
                    total = total
                        .checked_add(n)
                        .ok_or_else(|| runtime("counter overflow"))?;
                }
                Ok(Value::Int(total))
            }
        }
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
        let mut group = Vec::with_capacity(counter.group_by.len());
        for part in &counter.group_by {
            group.push(match part {
                IndexPart::Field(f) => row.get(f).cloned().unwrap_or_default(),
                IndexPart::Computed(part) => self.eval_on_row(part.var, &part.expr, row)?,
            });
        }
        Ok(Some((group, amount)))
    }

    fn bump(&mut self, counter: &str, group: &[Value], shard: Option<i64>, delta: i64) -> R<()> {
        let mut group = group.to_vec();
        if let Some(shard) = shard {
            group.push(Value::Int(shard));
        }
        let key = self.keys.counter(counter, &parts(&group)?);
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

    /// Scans the ranges for each `anyOf` value (each up to the limit) and merges them in key
    /// order, keeping the first `limit` rows.
    fn query_any(
        &mut self,
        entity_name: &str,
        scan: &Scan<'a>,
        any_of: &'a [Expr],
    ) -> R<Vec<Value>> {
        let entity = self.entity(entity_name)?;
        let types = scanned_types(entity, scan.index);
        let mut prefix = Vec::with_capacity(scan.prefix.len() + 1);
        for (i, expr) in scan.prefix.iter().enumerate() {
            let value = self.eval(expr)?;
            prefix.push(key_value(types.get(i).copied().flatten(), value));
        }
        let any_type = types.get(prefix.len()).copied().flatten();
        let limit = usize::try_from(self.eval_int(scan.limit)?.max(0)).unwrap_or(usize::MAX);
        let mut merged: Vec<(Vec<u8>, Value)> = Vec::new();
        for expr in any_of {
            let value = key_value(any_type, self.eval(expr)?);
            let mut values = prefix.clone();
            values.push(value);
            let start_len = match scan.index {
                None => self.keys.row_range(entity_name, &parts(&values)?),
                Some(index) => self.keys.index_range(entity_name, index, &parts(&values)?),
            }
            .start
            .as_bytes()
            .len();
            let one = Scan {
                index: scan.index,
                prefix: &[],
                limit: scan.limit,
                order: scan.order,
                range: scan.range,
            };
            for (key, row) in self.scan_entries(entity_name, &one, &values, None)?.0 {
                // Merge on the key after the anyOf part.
                merged.push((key.as_bytes()[start_len..].to_vec(), row));
            }
        }
        merged.sort_by(|(a, _), (b, _)| a.cmp(b));
        if scan.order == Order::Desc {
            merged.reverse();
        }
        merged.truncate(limit);
        Ok(merged.into_iter().map(|(_, row)| row).collect())
    }

    /// Scans a bounded range of rows, optionally resuming after a cursor. Returns the rows and,
    /// if the page is full, a cursor for the next one.
    fn query(
        &mut self,
        entity_name: &str,
        scan: &Scan<'a>,
        after: Option<&str>,
    ) -> R<(Vec<Value>, Option<String>)> {
        let types = scanned_types(self.entity(entity_name)?, scan.index);
        let mut values = Vec::with_capacity(scan.prefix.len());
        for (i, expr) in scan.prefix.iter().enumerate() {
            let value = self.eval(expr)?;
            values.push(key_value(types.get(i).copied().flatten(), value));
        }
        let (rows, cursor) = self.scan_entries(entity_name, scan, &values, after)?;
        Ok((rows.into_iter().map(|(_, row)| row).collect(), cursor))
    }

    /// Scans rows whose scanned key starts with `values` (evaluated prefix), with the scan's
    /// range on the next part. Expired rows are skipped (they still count as reads).
    fn scan_entries(
        &mut self,
        entity_name: &str,
        scan: &Scan<'a>,
        values: &[Value],
        after: Option<&str>,
    ) -> R<ScanPage> {
        let entity = self.entity(entity_name)?;
        let types = scanned_types(entity, scan.index);
        let next_type = types.get(values.len()).copied().flatten();
        let limit = usize::try_from(self.eval_int(scan.limit)?.max(0)).unwrap_or(usize::MAX);
        let (direction, forward) = match scan.order {
            Order::Asc => (Direction::Forward, true),
            Order::Desc => (Direction::Reverse, false),
        };
        let full = match scan.index {
            None => self.keys.row_range(entity_name, &parts(values)?),
            Some(index) => self.keys.index_range(entity_name, index, &parts(values)?),
        };
        // Bounds on the next part narrow the scan; cursors stay relative to the full prefix range.
        let bounded = match scan.range {
            None => full.clone(),
            Some(range) => {
                // Like SQL, a comparison never matches `null`: with no lower bound, a range still
                // starts after the null values (which sort first).
                let lower = self
                    .bound(range.gt.as_ref(), range.gte.as_ref())?
                    .map(|(v, inclusive)| (key_value(next_type, v), inclusive))
                    .or(Some((Value::Null, false)));
                let upper = self
                    .bound(range.lt.as_ref(), range.lte.as_ref())?
                    .map(|(v, inclusive)| (key_value(next_type, v), inclusive));
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
        for (key, bytes) in entries {
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
            let row = decode_row(&bytes)?;
            if !self.expired(entity, &row) {
                rows.push((key, Value::Object(row)));
            }
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

    /// Evaluates a list operand. `null` (an optional list, or a field of a missing row) is empty.
    fn eval_list(&mut self, expr: &'a Expr) -> R<Vec<Value>> {
        match self.eval(expr)? {
            Value::List(items) => Ok(items),
            Value::Null => Ok(Vec::new()),
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
            Expr::Decimal { value } => Value::Decimal(
                crate::decimal::Decimal::parse(value)
                    .ok_or_else(|| runtime(format!("{value:?} isn't a decimal")))?,
            ),
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
            Expr::CallerProfile { field } => {
                let declared = self.manifest.auth.profile.get(field);
                let raw = self.caller.and_then(|c| c.profile.get(field)).cloned();
                match (declared, raw) {
                    // A value the platform passed that doesn't fit its declaration reads as null.
                    (Some(decl), Some(json)) => Value::from_json(json)
                        .ok()
                        .and_then(|v| conform(decl, v).ok())
                        .unwrap_or_default(),
                    _ => Value::Null,
                }
            }
            Expr::Now => Value::Int(self.now),
            // A field path from a variable (`row.price`, `order.shipTo.city`) reads in place
            // instead of copying the whole row; it charges the same ops as evaluating each node.
            Expr::Field { of, name } => match self.field_path(expr) {
                Some((value, nodes)) => {
                    if self.metered {
                        self.meter.charge(crate::limits::Cost::ops(nodes))?;
                    }
                    value
                }
                None => match self.eval(of)? {
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
            },
            Expr::IsNull { of } => Value::Bool(self.eval(of)?.is_null()),
            Expr::Not { arg } => Value::Bool(!self.eval_bool(arg)?),
            Expr::Neg { arg } => match self.eval(arg)? {
                Value::Null => Value::Null,
                Value::Decimal(d) => Value::Decimal(d.negate().ok_or_else(|| runtime("overflow"))?),
                Value::Int(i) => {
                    Value::Int(i.checked_neg().ok_or_else(|| runtime("integer overflow"))?)
                }
                other => return Err(runtime(format!("can't negate {}", other.type_name()))),
            },
            Expr::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs)?,
            Expr::In { value, list } => {
                let value = self.eval(value)?;
                Value::Bool(
                    list.iter()
                        .any(|l| builtins::equal(&Value::from(l), &value)),
                )
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
            Expr::Abs { arg } => match self.eval(arg)? {
                Value::Null => Value::Null,
                Value::Decimal(d) => Value::Decimal(d.abs().ok_or_else(|| runtime("overflow"))?),
                Value::Int(i) => {
                    Value::Int(i.checked_abs().ok_or_else(|| runtime("integer overflow"))?)
                }
                other => {
                    return Err(runtime(format!(
                        "can't take the absolute value of {}",
                        other.type_name()
                    )));
                }
            },
            Expr::Concat { parts } => {
                let mut out = String::new();
                for part in parts {
                    out.push_str(&self.eval(part)?.to_string());
                }
                Value::String(out)
            }
            Expr::Agg {
                op,
                list,
                item,
                value,
            } => self.aggregate(*op, list, *item, value.as_deref())?,
            Expr::Sort {
                list,
                item,
                by,
                desc,
            } => self.sort(list, *item, by, *desc)?,
            Expr::Slice { of, start, end } => {
                let target = self.eval(of)?;
                let start = self.eval_int(start)?;
                let end = match end {
                    Some(end) => self.eval_opt_int(end)?,
                    None => None,
                };
                match target {
                    Value::Null => Value::Null,
                    Value::List(items) => {
                        let (s, e) = builtins::slice_range(items.len(), start, end);
                        Value::List(items[s..e].to_vec())
                    }
                    Value::String(text) => {
                        let chars: Vec<char> = text.chars().collect();
                        let (s, e) = builtins::slice_range(chars.len(), start, end);
                        Value::String(chars[s..e].iter().collect())
                    }
                    other => {
                        return Err(runtime(format!("can't slice {}", other.type_name())));
                    }
                }
            }
            Expr::Distinct { of } => {
                let items = self.eval_list(of)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for item in items {
                    self.meter.op()?;
                    if seen.insert(item.to_json().to_string()) {
                        out.push(item);
                    }
                }
                Value::List(out)
            }
            Expr::ListConcat { parts } => {
                let mut out = Vec::new();
                for part in parts {
                    match self.eval(part)? {
                        Value::Null => {}
                        Value::List(items) => out.extend(items),
                        other => {
                            return Err(runtime(format!(
                                "can only concatenate lists, got {}",
                                other.type_name()
                            )));
                        }
                    }
                }
                Value::List(out)
            }
            Expr::Includes { list, value } => {
                let items = match self.eval(list)? {
                    Value::Null => Vec::new(),
                    Value::List(items) => items,
                    other => {
                        return Err(runtime(format!("expected list, got {}", other.type_name())));
                    }
                };
                let value = self.eval(value)?;
                let mut found = false;
                for item in &items {
                    self.meter.op()?;
                    if builtins::equal(item, &value) {
                        found = true;
                        break;
                    }
                }
                Value::Bool(found)
            }
            Expr::Split { of, sep, max } => {
                match (self.eval_opt_string(of)?, self.eval_opt_string(sep)?) {
                    (Some(text), Some(sep)) => {
                        Value::List(builtins::split(&text, &sep, *max).map_err(runtime)?)
                    }
                    _ => Value::Null,
                }
            }
            Expr::Replace { of, find, with } => {
                let (text, find, with) = (
                    self.eval_opt_string(of)?,
                    self.eval_opt_string(find)?,
                    self.eval_opt_string(with)?,
                );
                match (text, find) {
                    (Some(text), Some(find)) => Value::String(
                        builtins::replace(&text, &find, &with.unwrap_or_default())
                            .map_err(runtime)?,
                    ),
                    _ => Value::Null,
                }
            }
            Expr::Pad {
                of,
                length,
                fill,
                end,
            } => match self.eval(of)? {
                Value::Null => Value::Null,
                other => Value::String(builtins::pad(&other.to_string(), *length, fill, *end)),
            },
            Expr::Matches { of, pattern } => match self.eval_opt_string(of)? {
                None => Value::Bool(false),
                Some(text) => {
                    let regex = regex_lite::Regex::new(pattern)
                        .map_err(|e| runtime(format!("invalid pattern: {e}")))?;
                    Value::Bool(regex.is_match(&text))
                }
            },
            Expr::FormatNumber {
                of,
                decimals,
                grouping,
            } => {
                let value = self.eval(of)?;
                builtins::format_number(&value, *decimals, *grouping).map_err(runtime)?
            }
            Expr::Round { of, scale, mode } => {
                let value = self.eval(of)?;
                builtins::round(&value, *scale, *mode).map_err(runtime)?
            }
            Expr::ToInt { of, mode } => {
                let value = self.eval(of)?;
                builtins::to_int(&value, *mode).map_err(runtime)?
            }
            Expr::StartOf { of, unit, tz } => {
                let at = self.eval_opt_int(of)?;
                let tz = self.time_zone(tz.as_deref())?;
                match at {
                    None => Value::Null,
                    Some(ms) => Value::Int(super::time::start_of(ms, *unit, &tz).map_err(runtime)?),
                }
            }
            Expr::AddTime {
                of,
                amount,
                unit,
                tz,
            } => {
                let at = self.eval_opt_int(of)?;
                let amount = self.eval_int(amount)?;
                let tz = self.time_zone(tz.as_deref())?;
                match at {
                    None => Value::Null,
                    Some(ms) => {
                        Value::Int(super::time::add(ms, amount, *unit, &tz).map_err(runtime)?)
                    }
                }
            }
            Expr::TimePart { of, part, tz } => {
                let at = self.eval_opt_int(of)?;
                let tz = self.time_zone(tz.as_deref())?;
                match at {
                    None => Value::Null,
                    Some(ms) => Value::Int(super::time::part(ms, *part, &tz).map_err(runtime)?),
                }
            }
            Expr::FormatTime { of, pattern, tz } => {
                let at = self.eval_opt_int(of)?;
                let tz = self.time_zone(tz.as_deref())?;
                match at {
                    None => Value::Null,
                    Some(ms) => Value::String(
                        super::time::format(ms, pattern.as_deref(), &tz).map_err(runtime)?,
                    ),
                }
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
        // Like SQL, `null` propagates through arithmetic, and an ordering comparison with `null`
        // is false (equality still compares it).
        let has_null = a.is_null() || b.is_null();
        if has_null && !matches!(op, BinaryOp::Eq | BinaryOp::Neq) {
            return Ok(if op.is_comparison() {
                Value::Bool(false)
            } else {
                Value::Null
            });
        }
        if op.is_comparison() {
            let result = match op {
                BinaryOp::Eq => builtins::equal(&a, &b),
                BinaryOp::Neq => !builtins::equal(&a, &b),
                BinaryOp::Lt => compare(&a, &b)? == Ordering::Less,
                BinaryOp::Lte => compare(&a, &b)? != Ordering::Greater,
                BinaryOp::Gt => compare(&a, &b)? == Ordering::Greater,
                _ => compare(&a, &b)? != Ordering::Less,
            };
            return Ok(Value::Bool(result));
        }
        builtins::arithmetic(op, &a, &b).map_err(runtime)
    }

    /// Reads a chain of fields rooted at a variable without copying the variable's value.
    /// Returns the value and how many more nodes it covers than the outer one (already charged).
    fn field_path(&self, expr: &Expr) -> Option<(Value, u64)> {
        let mut names = Vec::new();
        let mut node = expr;
        while let Expr::Field { of, name } = node {
            names.push(name.as_str());
            node = of;
        }
        let Expr::Var { id } = node else {
            return None;
        };
        let mut value = self.vars.get(id)?;
        for name in names.iter().rev() {
            value = match value {
                Value::Object(fields) => fields.get(*name).unwrap_or(&Value::Null),
                Value::Null => &Value::Null,
                _ => return None,
            };
        }
        Some((value.clone(), names.len() as u64))
    }

    /// Evaluates `expr` once per item, with the item bound to `var`, charging one op per item.
    fn per_item(&mut self, items: &[Value], var: VarId, expr: &'a Expr) -> R<Vec<Value>> {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            self.meter.op()?;
            self.vars.insert(var, item.clone());
            out.push(self.eval(expr)?);
        }
        Ok(out)
    }

    fn aggregate(
        &mut self,
        op: AggOp,
        list: &'a Expr,
        item: VarId,
        value: Option<&'a Expr>,
    ) -> R<Value> {
        let items = self.eval_list(list)?;
        let values = match value {
            Some(expr) => self.per_item(&items, item, expr)?,
            None => {
                for _ in &items {
                    self.meter.op()?;
                }
                items
            }
        };
        let truthy = |v: &Value| matches!(v, Value::Bool(true));
        Ok(match op {
            AggOp::Some => Value::Bool(values.iter().any(truthy)),
            AggOp::Every => Value::Bool(values.iter().all(truthy)),
            AggOp::Count => {
                let n = if value.is_some() {
                    values.iter().filter(|v| truthy(v)).count()
                } else {
                    values.len()
                };
                Value::Int(i64::try_from(n).unwrap_or(i64::MAX))
            }
            AggOp::Sum => {
                let mut total = Value::Int(0);
                for v in values.iter().filter(|v| !v.is_null()) {
                    total = builtins::arithmetic(BinaryOp::Add, &total, v).map_err(runtime)?;
                }
                total
            }
            AggOp::Min | AggOp::Max => {
                let mut best: Option<Value> = None;
                for v in values.into_iter().filter(|v| !v.is_null()) {
                    let better = match &best {
                        None => true,
                        Some(b) => {
                            let order = compare(&v, b)?;
                            if op == AggOp::Min {
                                order == Ordering::Less
                            } else {
                                order == Ordering::Greater
                            }
                        }
                    };
                    if better {
                        best = Some(v);
                    }
                }
                best.unwrap_or(Value::Null)
            }
        })
    }

    fn sort(&mut self, list: &'a Expr, item: VarId, by: &'a Expr, desc: bool) -> R<Value> {
        let items = self.eval_list(list)?;
        let keys = self.per_item(&items, item, by)?;
        // Charge comparisons: n * ceil(log2 n), matching the static bound.
        self.meter
            .charge(crate::limits::Cost::ops(sort_comparisons(
                items.len() as u64
            )))?;
        let mut pairs: Vec<(Value, Value)> = keys.into_iter().zip(items).collect();
        let mut failure = None;
        pairs.sort_by(|(a, _), (b, _)| {
            let order = match (a, b) {
                (Value::Null, Value::Null) => Ordering::Equal,
                (Value::Null, _) => Ordering::Less,
                (_, Value::Null) => Ordering::Greater,
                (a, b) => builtins::compare(a, b).unwrap_or_else(|e| {
                    failure.get_or_insert(e);
                    Ordering::Equal
                }),
            };
            if desc { order.reverse() } else { order }
        });
        if let Some(message) = failure {
            return Err(runtime(message));
        }
        Ok(Value::List(pairs.into_iter().map(|(_, v)| v).collect()))
    }

    /// Evaluates an optional time zone expression (`null` or absent means UTC).
    fn time_zone(&mut self, tz: Option<&'a Expr>) -> R<jiff::tz::TimeZone> {
        let Some(expr) = tz else {
            return Ok(jiff::tz::TimeZone::UTC);
        };
        match self.eval(expr)? {
            Value::Null => Ok(jiff::tz::TimeZone::UTC),
            Value::String(name) => super::time::zone(&name)
                .ok_or_else(|| ExecError::BadInput(format!("{name:?} isn't an IANA time zone"))),
            other => Err(runtime(format!(
                "a time zone must be a string, got {}",
                other.type_name()
            ))),
        }
    }

    /// Evaluates a timestamp operand; `None` for `null`.
    fn eval_opt_int(&mut self, expr: &'a Expr) -> R<Option<i64>> {
        match self.eval(expr)? {
            Value::Int(i) => Ok(Some(i)),
            Value::Null => Ok(None),
            other => Err(runtime(format!("expected int, got {}", other.type_name()))),
        }
    }
}

fn check_wait(ms: i64) -> R<()> {
    if (0..=super::workflow::MAX_WAIT_MS).contains(&ms) {
        Ok(())
    } else {
        Err(runtime(format!(
            "workflow delays and timeouts are 0 to {} ms, got {ms}",
            super::workflow::MAX_WAIT_MS
        )))
    }
}

/// The next version of a versioned row.
fn next_version(old: &Row) -> Value {
    match old.get("version") {
        Some(Value::Int(v)) => Value::Int(v.saturating_add(1)),
        _ => Value::Int(1),
    }
}

/// The shard a row's counter contribution goes to: a hash of its primary key.
fn shard_of(pk: &[Value], shards: u8) -> i64 {
    let bytes = serde_json::to_vec(pk).unwrap_or_default();
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    });
    i64::try_from(hash % u64::from(shards.max(1))).unwrap_or(0)
}

/// Comparisons a sort of `n` items is charged: `n * ceil(log2 n)`.
pub(crate) fn sort_comparisons(n: u64) -> u64 {
    if n < 2 {
        return 0;
    }
    n * u64::from(64 - (n - 1).leading_zeros())
}

fn compare(a: &Value, b: &Value) -> R<Ordering> {
    builtins::compare(a, b).map_err(runtime)
}

/// A key value in the stored form of its field: ints given for decimal fields become decimals,
/// so they encode (and compare) like the stored values. Anything else is left alone.
fn key_value(ty: Option<&FieldType>, value: Value) -> Value {
    match (ty, value) {
        (Some(FieldType::Decimal { scale, .. }), Value::Int(i)) => {
            crate::decimal::Decimal::from_int(i)
                .exactly_at(*scale)
                .map_or(Value::Int(i), Value::Decimal)
        }
        (_, value) => value,
    }
}

/// The field types of the parts a query scans (`None` for computed index parts).
fn scanned_types<'e>(entity: &'e Entity, index: Option<&str>) -> Vec<Option<&'e FieldType>> {
    let field = |name: &str| entity.fields.get(name).map(|f| &f.ty);
    match index.and_then(|name| entity.indexes.get(name)) {
        None => entity.key.iter().map(|name| field(name)).collect(),
        Some(ix) => ix
            .fields
            .iter()
            .map(|part| match part {
                IndexPart::Field(name) => field(name),
                IndexPart::Computed(_) => None,
            })
            .collect(),
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
        let value = conform(field, value)
            .map_err(|e| format!("{integration}.{operation} output {name}: {e}"))?;
        out.insert(name.clone(), value);
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::sort_comparisons;

    #[test]
    fn sorts_are_charged_n_times_ceil_log2_n_comparisons() {
        let expected = [
            (0, 0),
            (1, 0),
            (2, 2),
            (3, 6),
            (4, 8),
            (5, 15),
            (8, 24),
            (9, 36),
        ];
        for (n, comparisons) in expected {
            assert_eq!(sort_comparisons(n), comparisons, "{n} items");
        }
    }
}
