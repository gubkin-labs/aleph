//! Processing the outbox: firing cron ticks, running handlers, and delivering messages, with
//! retries, exponential backoff, and dead-lettering.
//!
//! Each outbox entry is handled in its own transaction, which also deletes the entry. A handler's
//! writes, its new effects, and the removal of the entry that ran it commit together, so an
//! entry is never half-processed. If a worker crashes before committing, the entry stays and runs
//! again: delivery is at-least-once.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use super::effects::{Journal, OutboxItem, Pending, backoff_ms};
use super::exec::{Exec, ExecError};
use super::input::Inputs;
use super::meter::Meter;
use super::workflow::{Instance, Status, Transition, Waiting};
use super::{Engine, Env};
use crate::keys::{Key, KeyPart, Keyspace, TenantId};
use crate::limits::{Limits, MAX_CHAIN_DEPTH, MAX_CONTINUATIONS};
use crate::manifest::{CronScan, Handler, QueueDecl, Trigger, WorkflowDecl};
use crate::store::{CommitError, Direction, Transaction};
use crate::value::Value;

/// What one call to [`Engine::process`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProcessReport {
    /// Cron ticks fired.
    pub fired: u32,
    /// Handler runs that committed.
    pub ran: u32,
    /// Messages handed to the sink.
    pub delivered: u32,
    /// Failed attempts rescheduled with backoff.
    pub retried: u32,
    /// Jobs moved to their queue's dead-letter handler after their last retry.
    pub dead_lettered: u32,
    /// Work dropped after its last retry, with the reason.
    pub failures: Vec<String>,
    /// Whether due work remains (`max_items` was reached).
    pub remaining: bool,
    /// What each committed handler run actually used. Always within the handler's bounds.
    pub runs: Vec<HandlerRun>,
}

/// One committed handler run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HandlerRun {
    /// The handler id.
    pub handler: String,
    /// What the run used.
    pub usage: crate::limits::Cost,
}

impl ProcessReport {
    fn record(&mut self, handler: &str, usage: Option<crate::limits::Cost>) {
        if let Some(usage) = usage {
            self.ran += 1;
            self.runs.push(HandlerRun {
                handler: handler.to_owned(),
                usage,
            });
        }
    }
}

/// Writes buffered effects to the outbox, inside the transaction that produced them.
pub(crate) fn write_outbox(
    tx: &mut dyn Transaction,
    keys: &Keyspace,
    env: &dyn Env,
    now: i64,
    effects: Vec<Pending>,
) -> Result<(), ExecError> {
    for pending in effects {
        let job = match &pending.item {
            OutboxItem::Run { job: Some(job), .. } | OutboxItem::Cancel { job } => Some(job),
            _ => None,
        };
        // A keyed job replaces (or a cancel removes) the pending job with the same key.
        if let Some(job) = job {
            let pointer = keys.job(&job.queue, &job.key);
            if let Some(entry) = tx.get(&pointer)? {
                let entry = Key::from_bytes(entry);
                if keys.all().contains(&entry) {
                    tx.delete(entry);
                }
            }
            tx.delete(pointer);
        }
        if matches!(pending.item, OutboxItem::Cancel { .. }) {
            continue;
        }
        let id = env.new_id(now);
        let bytes =
            serde_json::to_vec(&pending.item).map_err(|e| ExecError::Runtime(e.to_string()))?;
        let entry = keys.outbox(now.saturating_add(pending.delay_ms), &id);
        if let Some(job) = job {
            tx.put(keys.job(&job.queue, &job.key), entry.as_bytes().to_vec());
        }
        tx.put(entry, bytes);
    }
    Ok(())
}

/// Which scheduled step of which instance.
struct StepRef<'a> {
    workflow: &'a str,
    key: &'a str,
    seq: u64,
}

fn step_item(
    workflow: &str,
    key: &str,
    seq: u64,
    attempt: u32,
    signal: Option<(String, Value)>,
    error: Option<String>,
) -> OutboxItem {
    OutboxItem::Step {
        workflow: workflow.to_owned(),
        key: key.to_owned(),
        seq,
        attempt,
        signal,
        error,
    }
}

/// Retry settings: (retries, base backoff, max backoff).
fn retry_policy(engine: &Engine, handler: &Handler) -> (u32, u32, u32) {
    match &handler.trigger {
        Trigger::Queue { queue } => engine
            .manifest
            .manifest()
            .queues
            .get(queue)
            .map_or(QueueDecl::HANDLER_DEFAULTS, |q| {
                (q.retries, q.backoff_ms, q.max_backoff_ms)
            }),
        _ => QueueDecl::HANDLER_DEFAULTS,
    }
}

impl Engine {
    /// Does the work that's due for `tenant`: fires cron ticks, then handles due outbox entries
    /// in due-time order (running handlers and delivering messages), at most `max_items` of them.
    ///
    /// Call it after requests, and periodically: the platform runs it continuously, and the dev
    /// engine runs it when told to.
    pub fn process(&self, tenant: &TenantId, max_items: u32) -> ProcessReport {
        let keys = Keyspace::new(tenant);
        let now = self.env.now_ms();
        let mut report = ProcessReport::default();
        self.fire_crons(&keys, now, &mut report);
        for _ in 0..max_items {
            let Some((key, bytes)) = self.next_due(&keys, now, &mut report) else {
                return report;
            };
            match serde_json::from_slice::<OutboxItem>(&bytes) {
                Ok(item) => self.handle_item(&keys, now, &key, item, &mut report),
                Err(error) => {
                    self.remove(&key);
                    report
                        .failures
                        .push(format!("corrupt outbox entry dropped: {error}"));
                }
            }
        }
        report.remaining = self.next_due(&keys, now, &mut report).is_some();
        report
    }

    fn next_due(
        &self,
        keys: &Keyspace,
        now: i64,
        report: &mut ProcessReport,
    ) -> Option<(Key, Vec<u8>)> {
        let mut tx = self.store.begin();
        match tx.scan(&keys.outbox_due(now), 1, Direction::Forward) {
            Ok(mut entries) => entries.pop(),
            Err(error) => {
                report.failures.push(error.to_string());
                None
            }
        }
    }

    fn fire_crons(&self, keys: &Keyspace, now: i64, report: &mut ProcessReport) {
        for handler in &self.manifest.manifest().handlers {
            let Trigger::Cron {
                every_ms,
                offset_ms,
                scan,
                schedule,
            } = &handler.trigger
            else {
                continue;
            };
            let mut tx = self.store.begin();
            let state = keys.schedule(&handler.id);
            let last = tx
                .get(&state)
                .ok()
                .flatten()
                .and_then(|b| String::from_utf8(b).ok())
                .and_then(|s| s.parse::<i64>().ok());
            let tick = match schedule {
                Some(schedule) => {
                    let (Ok(cron), Some(tz)) = (
                        super::cron::Cron::parse(&schedule.cron),
                        super::time::zone(schedule.tz.as_deref().unwrap_or("UTC")),
                    ) else {
                        continue;
                    };
                    // The first time a schedule is seen, it starts from now (no back-fill).
                    let Some(last) = last else {
                        tx.put(state, now.to_string().into_bytes());
                        let _ = tx.commit();
                        continue;
                    };
                    // The latest scheduled time that's due; missed earlier ones are skipped.
                    let mut due = None;
                    let mut at = last;
                    for _ in 0..10_000 {
                        match cron.next_after(at, &tz) {
                            Some(next) if next <= now => {
                                due = Some(next);
                                at = next;
                            }
                            _ => break,
                        }
                    }
                    let Some(tick) = due else {
                        continue;
                    };
                    tick
                }
                None => {
                    let (every, offset) = (i64::from(*every_ms), i64::from(*offset_ms));
                    if now < offset || every == 0 {
                        continue;
                    }
                    let tick = (now - offset) / every * every + offset;
                    if last.is_some_and(|last| last >= tick) {
                        continue;
                    }
                    tick
                }
            };
            tx.put(state, tick.to_string().into_bytes());
            let item = match scan {
                Some(_) => OutboxItem::Scan {
                    handler: handler.id.clone(),
                    fired_at: tick,
                    cursor: None,
                    attempt: 0,
                },
                None => OutboxItem::Run {
                    handler: handler.id.clone(),
                    payload: BTreeMap::from([("firedAt".to_owned(), Value::Int(tick))]),
                    attempt: 0,
                    job: None,
                    chain: 0,
                    turns: 0,
                },
            };
            let pending = vec![Pending { delay_ms: 0, item }];
            if write_outbox(&mut *tx, keys, self.env.as_ref(), now, pending).is_ok()
                && tx.commit().is_ok()
            {
                report.fired += 1;
            }
        }
    }

    fn handler(&self, id: &str) -> Option<&Handler> {
        self.manifest
            .manifest()
            .handlers
            .iter()
            .find(|h| h.id == id)
    }

    #[allow(clippy::too_many_lines, reason = "one arm per outbox entry kind")]
    fn handle_item(
        &self,
        keys: &Keyspace,
        now: i64,
        key: &Key,
        item: OutboxItem,
        report: &mut ProcessReport,
    ) {
        match item {
            OutboxItem::Step {
                workflow,
                key: instance,
                seq,
                attempt,
                signal,
                error,
            } => {
                let step = StepRef {
                    workflow: &workflow,
                    key: &instance,
                    seq,
                };
                self.handle_step(keys, now, key, &step, attempt, signal, error, report);
            }
            OutboxItem::Signal {
                workflow,
                key: instance,
                signal,
                payload,
            } => {
                let decl = self.manifest.manifest().workflows.get(&workflow);
                self.update_instance(
                    keys,
                    now,
                    key,
                    &workflow,
                    &instance,
                    report,
                    |inst, next| {
                        if inst.status.finished() {
                            return;
                        }
                        let resumes = inst.status == Status::Waiting
                            && inst.waiting.as_ref().is_some_and(|w| w.signal == signal);
                        if !resumes {
                            // Not waiting for it (yet): keep the latest of each signal.
                            inst.signals.insert(signal.clone(), payload.clone());
                            return;
                        }
                        let then = inst.waiting.take().map(|w| w.then).unwrap_or_default();
                        inst.status = Status::Running;
                        if let Some(decl) = decl
                            && inst.advance(decl)
                        {
                            inst.step = then;
                            next.push(step_item(
                                &workflow,
                                &instance,
                                inst.seq,
                                0,
                                Some((signal.clone(), payload.clone())),
                                None,
                            ));
                        }
                    },
                );
            }
            OutboxItem::Timeout {
                workflow,
                key: instance,
                seq,
            } => {
                let decl = self.manifest.manifest().workflows.get(&workflow);
                self.update_instance(
                    keys,
                    now,
                    key,
                    &workflow,
                    &instance,
                    report,
                    |inst, next| {
                        if inst.status != Status::Waiting || inst.seq != seq {
                            return; // the signal came, or the instance moved on
                        }
                        let on_timeout = inst
                            .waiting
                            .take()
                            .map(|w| w.on_timeout)
                            .unwrap_or_default();
                        inst.status = Status::Running;
                        if let Some(decl) = decl
                            && inst.advance(decl)
                        {
                            inst.step = on_timeout;
                            next.push(step_item(&workflow, &instance, inst.seq, 0, None, None));
                        }
                    },
                );
            }
            OutboxItem::CancelWorkflow {
                workflow,
                key: instance,
            } => {
                self.update_instance(keys, now, key, &workflow, &instance, report, |inst, _| {
                    if !inst.status.finished() {
                        inst.status = Status::Cancelled;
                        inst.waiting = None;
                        inst.seq += 1; // anything already scheduled is now stale
                    }
                });
            }
            OutboxItem::Deliver { delivery, attempt } => {
                match self.sink.deliver(keys.tenant(), &delivery) {
                    Ok(()) => {
                        self.remove(key);
                        report.delivered += 1;
                    }
                    Err(error) => {
                        let (retries, base, max) = QueueDecl::HANDLER_DEFAULTS;
                        if attempt < retries {
                            let next = OutboxItem::Deliver {
                                delivery,
                                attempt: attempt + 1,
                            };
                            self.reschedule(keys, key, now + backoff_ms(attempt, base, max), &next);
                            report.retried += 1;
                        } else {
                            self.remove(key);
                            report
                                .failures
                                .push(format!("delivery to {}: {error}", delivery.target));
                        }
                    }
                }
            }
            OutboxItem::Cancel { .. } => self.remove(key),
            OutboxItem::Run {
                handler: id,
                payload,
                attempt,
                job,
                chain,
                turns,
            } => {
                let Some(handler) = self.handler(&id) else {
                    self.remove(key);
                    report.failures.push(format!("unknown handler {id}"));
                    return;
                };
                let limit = handler
                    .max_depth
                    .map_or(MAX_CHAIN_DEPTH, |d| d.min(MAX_CHAIN_DEPTH));
                if chain > limit {
                    // A trigger cycle: refuse the run, which ends the chain. Retrying can't help.
                    self.remove(key);
                    report.failures.push(format!(
                        "{id}: trigger chain too deep: this run would be {chain} handler hops \
                         after the request or schedule that started the chain, over the limit \
                         of {limit}, so it didn't run"
                    ));
                    return;
                }
                if turns > MAX_CONTINUATIONS {
                    self.remove(key);
                    report.failures.push(format!(
                        "{id}: the job re-enqueued itself {turns} times in a row, over the limit \
                         of {MAX_CONTINUATIONS}, so it didn't run"
                    ));
                    return;
                }
                match self.run_handler(keys, now, key, handler, &payload, None, (chain, turns)) {
                    Ok(usage) => report.record(&id, usage),
                    Err(error) => {
                        let (retries, base, max) = retry_policy(self, handler);
                        if attempt < retries {
                            let next = OutboxItem::Run {
                                handler: id,
                                payload,
                                attempt: attempt + 1,
                                job,
                                chain,
                                turns,
                            };
                            self.reschedule(keys, key, now + backoff_ms(attempt, base, max), &next);
                            report.retried += 1;
                            return;
                        }
                        let dead_letter = match &handler.trigger {
                            Trigger::Queue { queue } => self.manifest.manifest().handlers.iter().find(|h| {
                                matches!(&h.trigger, Trigger::DeadLetter { queue: q } if q == queue)
                            }),
                            _ => None,
                        };
                        match dead_letter {
                            Some(dead_letter) => {
                                let mut payload = payload;
                                payload.insert("error".into(), Value::String(error.to_string()));
                                payload
                                    .insert("attempts".into(), Value::Int(i64::from(attempt) + 1));
                                let next = OutboxItem::Run {
                                    handler: dead_letter.id.clone(),
                                    payload,
                                    attempt: 0,
                                    job: None,
                                    chain,
                                    turns: 0,
                                };
                                self.reschedule(keys, key, now, &next);
                                report.dead_lettered += 1;
                            }
                            None => {
                                self.remove(key);
                                report.failures.push(format!("{id}: {error}"));
                            }
                        }
                    }
                }
            }
            OutboxItem::Scan {
                handler: id,
                fired_at,
                cursor,
                attempt,
            } => {
                let Some(handler) = self.handler(&id) else {
                    self.remove(key);
                    report.failures.push(format!("unknown handler {id}"));
                    return;
                };
                let Trigger::Cron {
                    scan: Some(scan), ..
                } = &handler.trigger
                else {
                    self.remove(key);
                    report.failures.push(format!("handler {id} doesn't scan"));
                    return;
                };
                let payload = BTreeMap::from([("firedAt".to_owned(), Value::Int(fired_at))]);
                match self.run_handler(
                    keys,
                    now,
                    key,
                    handler,
                    &payload,
                    Some((scan, cursor.as_deref(), fired_at)),
                    (0, 0),
                ) {
                    Ok(usage) => report.record(&id, usage),
                    Err(error) => {
                        let (retries, base, max) = QueueDecl::HANDLER_DEFAULTS;
                        if attempt < retries {
                            let next = OutboxItem::Scan {
                                handler: id,
                                fired_at,
                                cursor,
                                attempt: attempt + 1,
                            };
                            self.reschedule(keys, key, now + backoff_ms(attempt, base, max), &next);
                            report.retried += 1;
                        } else {
                            self.remove(key);
                            report.failures.push(format!("{id}: {error}"));
                        }
                    }
                }
            }
        }
    }

    /// Runs a handler for one outbox entry: its writes, its new effects, and the entry's removal
    /// commit together. Conflicts are retried; anything else is returned as a failed attempt.
    #[allow(clippy::too_many_arguments)]
    fn run_handler(
        &self,
        keys: &Keyspace,
        now: i64,
        entry: &Key,
        handler: &Handler,
        payload: &BTreeMap<String, Value>,
        scan: Option<(&CronScan, Option<&str>, i64)>,
        (chain, turns): (u32, u32),
    ) -> Result<Option<crate::limits::Cost>, ExecError> {
        let env = self.env.as_ref();
        let mut journal = Journal::default();
        for _ in 0..=Limits::JOB.max_conflict_retries {
            let mut tx = self.store.begin();
            journal.rewind();
            // Another worker may already have handled this entry.
            if tx.get(entry)?.is_none() {
                return Ok(None);
            }
            let mut meter = Meter::new(Limits::JOB);
            let mut payload = payload.clone();
            let mut next = None;
            if let Some((scan, cursor, fired_at)) = &scan {
                let (rows, cursor) = read_batch(&mut *tx, keys, &mut meter, scan, *cursor)?;
                payload.insert("batch".into(), Value::List(rows));
                if let Some(cursor) = cursor {
                    meter.effect()?;
                    next = Some(OutboxItem::Scan {
                        handler: handler.id.clone(),
                        fired_at: *fired_at,
                        cursor: Some(cursor),
                        attempt: 0,
                    });
                }
            }
            let inputs = Inputs {
                payload,
                ..Inputs::default()
            };
            let mut effects = {
                let mut exec = Exec {
                    manifest: self.manifest.manifest(),
                    plan: &handler.plan,
                    errors: &handler.errors,
                    tenant: keys.tenant(),
                    integrations: self.integrations.as_ref(),
                    journal: &mut journal,
                    effects: Vec::new(),
                    tx: &mut *tx,
                    keys,
                    meter: &mut meter,
                    env,
                    now,
                    caller: None,
                    inputs: &inputs,
                    vars: HashMap::new(),
                    metered: true,
                    meta: super::exec::ResponseMeta::default(),
                    workflow: None,
                    state_updates: BTreeMap::new(),
                    chain: super::exec::Chain {
                        hops: chain,
                        continuing: match &handler.trigger {
                            Trigger::Queue { .. } => Some((handler.id.as_str(), turns)),
                            _ => None,
                        },
                    },
                };
                match exec.run() {
                    Ok(_) => std::mem::take(&mut exec.effects),
                    // A failure decided on stale reads isn't trusted: re-run it right away.
                    Err(_) if tx.validate() == Err(CommitError::Conflict) => continue,
                    Err(error) => return Err(error),
                }
            };
            if let Some(item) = next {
                effects.push(Pending { delay_ms: 0, item });
            }
            write_outbox(&mut *tx, keys, env, now, effects)?;
            tx.delete(entry.clone());
            match tx.commit() {
                Ok(()) => return Ok(Some(meter.used())),
                Err(CommitError::Conflict) => {}
                Err(CommitError::Store(error)) => return Err(ExecError::Store(error)),
            }
        }
        Err(ExecError::Runtime(
            "the handler's transaction kept conflicting".into(),
        ))
    }

    /// Runs one workflow step, with retries; after the last retry, the step's `onFail` step
    /// runs (compensation) or the instance fails.
    #[allow(clippy::too_many_arguments)]
    fn handle_step(
        &self,
        keys: &Keyspace,
        now: i64,
        entry: &Key,
        step: &StepRef<'_>,
        attempt: u32,
        signal: Option<(String, Value)>,
        error: Option<String>,
        report: &mut ProcessReport,
    ) {
        let manifest = self.manifest.manifest();
        let Some(decl) = manifest.workflows.get(step.workflow) else {
            self.remove(entry);
            report
                .failures
                .push(format!("unknown workflow {}", step.workflow));
            return;
        };
        match self.run_step(
            keys,
            now,
            entry,
            step,
            decl,
            signal.as_ref(),
            error.as_deref(),
        ) {
            Ok(Some((handler, usage))) => report.record(&handler, Some(usage)),
            Ok(None) => {}
            Err((handler, failure)) => {
                if attempt < decl.retries {
                    let next = OutboxItem::Step {
                        workflow: step.workflow.to_owned(),
                        key: step.key.to_owned(),
                        seq: step.seq,
                        attempt: attempt + 1,
                        signal,
                        error,
                    };
                    let at = now + backoff_ms(attempt, decl.backoff_ms, 60_000);
                    self.reschedule(keys, entry, at, &next);
                    report.retried += 1;
                    return;
                }
                let message = failure.to_string();
                report.failures.push(format!(
                    "{}: {message}",
                    handler.as_deref().unwrap_or(step.workflow)
                ));
                let on_fail =
                    handler.as_deref().and_then(|id| {
                        manifest.handlers.iter().find(|h| h.id == id).and_then(|h| {
                            match &h.trigger {
                                Trigger::Workflow { on_fail, .. } => on_fail.clone(),
                                _ => None,
                            }
                        })
                    });
                let seq = step.seq;
                self.update_instance(
                    keys,
                    now,
                    entry,
                    step.workflow,
                    step.key,
                    report,
                    |inst, next| {
                        if inst.seq != seq || inst.status.finished() {
                            return;
                        }
                        match &on_fail {
                            Some(compensate) => {
                                if inst.advance(decl) {
                                    inst.step.clone_from(compensate);
                                    next.push(step_item(
                                        step.workflow,
                                        step.key,
                                        inst.seq,
                                        0,
                                        None,
                                        Some(message.clone()),
                                    ));
                                }
                            }
                            None => {
                                inst.status = Status::Failed;
                                inst.error = Some(message.clone());
                                inst.seq += 1;
                            }
                        }
                    },
                );
            }
        }
    }

    /// One attempt at a workflow step, in one transaction: read the instance, run the step's
    /// plan, apply its transition, and write the instance and the next entry. `Ok(None)` if the
    /// entry was stale.
    #[allow(
        clippy::too_many_arguments,
        clippy::type_complexity,
        clippy::too_many_lines,
        reason = "one transaction: read, run, transition, write"
    )]
    fn run_step(
        &self,
        keys: &Keyspace,
        now: i64,
        entry: &Key,
        step: &StepRef<'_>,
        decl: &WorkflowDecl,
        signal: Option<&(String, Value)>,
        error: Option<&str>,
    ) -> Result<Option<(String, crate::limits::Cost)>, (Option<String>, ExecError)> {
        let manifest = self.manifest.manifest();
        let env = self.env.as_ref();
        let mut journal = Journal::default();
        let instance_key = keys.workflow(step.workflow, step.key);
        let fail = |e: ExecError| (None, e);
        for _ in 0..=Limits::JOB.max_conflict_retries {
            let mut tx = self.store.begin();
            journal.rewind();
            if tx.get(entry).map_err(|e| fail(e.into()))?.is_none() {
                return Ok(None);
            }
            let mut meter = Meter::new(Limits::JOB);
            let bytes = tx.get(&instance_key).map_err(|e| fail(e.into()))?;
            meter
                .read(bytes.as_ref().map_or(0, Vec::len))
                .map_err(|e| fail(e.into()))?;
            let instance: Option<Instance> = bytes.and_then(|b| serde_json::from_slice(&b).ok());
            let Some(mut instance) =
                instance.filter(|i| i.status == Status::Running && i.seq == step.seq)
            else {
                // Stale: cancelled, already past this step, or gone.
                tx.delete(entry.clone());
                let _ = tx.commit();
                return Ok(None);
            };
            let Some(handler) = manifest.handlers.iter().find(|h| {
                matches!(&h.trigger, Trigger::Workflow { workflow, step: s, .. }
                    if workflow == step.workflow && *s == instance.step)
            }) else {
                return Err(fail(ExecError::Runtime(format!(
                    "workflow {} has no step {}",
                    step.workflow, instance.step
                ))));
            };
            let with_handler = |e: ExecError| (Some(handler.id.clone()), e);
            let inputs = Inputs {
                payload: instance.payload(step.key, decl, signal, error),
                ..Inputs::default()
            };
            let (mut effects, transition, updates) = {
                let mut exec = Exec {
                    manifest,
                    plan: &handler.plan,
                    errors: &handler.errors,
                    tenant: keys.tenant(),
                    integrations: self.integrations.as_ref(),
                    journal: &mut journal,
                    effects: Vec::new(),
                    tx: &mut *tx,
                    keys,
                    meter: &mut meter,
                    env,
                    now,
                    caller: None,
                    inputs: &inputs,
                    vars: HashMap::new(),
                    metered: true,
                    meta: super::exec::ResponseMeta::default(),
                    workflow: Some(decl),
                    state_updates: BTreeMap::new(),
                    chain: super::exec::Chain::default(),
                };
                match exec.run() {
                    Ok(_) => (
                        std::mem::take(&mut exec.effects),
                        exec.meta.transition.take(),
                        std::mem::take(&mut exec.state_updates),
                    ),
                    Err(_) if tx.validate() == Err(CommitError::Conflict) => continue,
                    Err(e) => return Err(with_handler(e)),
                }
            };
            instance.state.extend(updates);
            let transition = transition.unwrap_or(Transition::Complete {
                result: Value::Null,
            });
            match transition {
                Transition::Next {
                    step: next,
                    delay_ms,
                } => {
                    if instance.advance(decl) {
                        instance.step = next;
                        meter.effect().map_err(|e| with_handler(e.into()))?;
                        effects.push(Pending {
                            delay_ms,
                            item: step_item(step.workflow, step.key, instance.seq, 0, None, None),
                        });
                    }
                }
                Transition::WaitFor {
                    signal: name,
                    timeout_ms,
                    then,
                    on_timeout,
                } => {
                    if let Some(payload) = instance.signals.remove(&name) {
                        // It already arrived.
                        if instance.advance(decl) {
                            instance.step = then;
                            meter.effect().map_err(|e| with_handler(e.into()))?;
                            effects.push(Pending {
                                delay_ms: 0,
                                item: step_item(
                                    step.workflow,
                                    step.key,
                                    instance.seq,
                                    0,
                                    Some((name, payload)),
                                    None,
                                ),
                            });
                        }
                    } else {
                        instance.status = Status::Waiting;
                        instance.seq += 1;
                        instance.waiting = Some(Waiting {
                            signal: name,
                            then,
                            on_timeout,
                        });
                        meter.effect().map_err(|e| with_handler(e.into()))?;
                        effects.push(Pending {
                            delay_ms: timeout_ms,
                            item: OutboxItem::Timeout {
                                workflow: step.workflow.to_owned(),
                                key: step.key.to_owned(),
                                seq: instance.seq,
                            },
                        });
                    }
                }
                Transition::Complete { result } => {
                    instance.status = Status::Completed;
                    instance.result = result;
                    instance.seq += 1;
                }
            }
            let bytes = serde_json::to_vec(&instance)
                .map_err(|e| with_handler(ExecError::Runtime(e.to_string())))?;
            if bytes.len() as u64 > Limits::JOB.max_row_bytes {
                return Err(with_handler(ExecError::Runtime(
                    "the workflow's state and result are too large to store".into(),
                )));
            }
            tx.put(instance_key.clone(), bytes);
            meter.write(1).map_err(|e| with_handler(e.into()))?;
            write_outbox(&mut *tx, keys, env, now, effects).map_err(with_handler)?;
            tx.delete(entry.clone());
            match tx.commit() {
                Ok(()) => return Ok(Some((handler.id.clone(), meter.used()))),
                Err(CommitError::Conflict) => {}
                Err(CommitError::Store(e)) => return Err(with_handler(ExecError::Store(e))),
            }
        }
        Err(fail(ExecError::Runtime(
            "the workflow step's transaction kept conflicting".into(),
        )))
    }

    /// Changes a workflow instance outside a step (signals, timeouts, cancels, failures) and
    /// removes the outbox entry that asked for it, in one transaction.
    #[allow(clippy::too_many_arguments)]
    fn update_instance(
        &self,
        keys: &Keyspace,
        now: i64,
        entry: &Key,
        workflow: &str,
        key: &str,
        report: &mut ProcessReport,
        mut change: impl FnMut(&mut Instance, &mut Vec<OutboxItem>),
    ) {
        let instance_key = keys.workflow(workflow, key);
        for _ in 0..=Limits::JOB.max_conflict_retries {
            let mut tx = self.store.begin();
            if !matches!(tx.get(entry), Ok(Some(_))) {
                return;
            }
            let instance: Option<Instance> = tx
                .get(&instance_key)
                .ok()
                .flatten()
                .and_then(|b| serde_json::from_slice(&b).ok());
            let mut next = Vec::new();
            if let Some(mut instance) = instance {
                change(&mut instance, &mut next);
                if let Ok(bytes) = serde_json::to_vec(&instance) {
                    tx.put(instance_key.clone(), bytes);
                }
            }
            let pending = next
                .into_iter()
                .map(|item| Pending { delay_ms: 0, item })
                .collect();
            if write_outbox(&mut *tx, keys, self.env.as_ref(), now, pending).is_err() {
                return;
            }
            tx.delete(entry.clone());
            match tx.commit() {
                Ok(()) => return,
                Err(CommitError::Conflict) => {}
                Err(CommitError::Store(e)) => {
                    report.failures.push(e.to_string());
                    return;
                }
            }
        }
    }

    fn remove(&self, key: &Key) {
        let mut tx = self.store.begin();
        tx.delete(key.clone());
        // A conflict means another worker changed the entry; it will be seen again.
        let _ = tx.commit();
    }

    fn reschedule(&self, keys: &Keyspace, key: &Key, at: i64, item: &OutboxItem) {
        let Ok(bytes) = serde_json::to_vec(item) else {
            return;
        };
        let mut tx = self.store.begin();
        tx.delete(key.clone());
        let entry = keys.outbox(at, &self.env.new_id(at));
        // A retried keyed job stays cancellable: its pointer follows it.
        if let OutboxItem::Run { job: Some(job), .. } = item {
            tx.put(keys.job(&job.queue, &job.key), entry.as_bytes().to_vec());
        }
        tx.put(entry, bytes);
        let _ = tx.commit();
    }
}

/// Reads one scan batch, metered, and the cursor for the next one if the batch was full.
fn read_batch(
    tx: &mut dyn Transaction,
    keys: &Keyspace,
    meter: &mut Meter,
    scan: &CronScan,
    cursor: Option<&str>,
) -> Result<(Vec<Value>, Option<String>), ExecError> {
    let prefix: Vec<Value> = scan.prefix.iter().map(Value::from).collect();
    let parts: Vec<KeyPart<'_>> = prefix.iter().filter_map(KeyPart::from_value).collect();
    let full = match &scan.index {
        None => keys.row_range(&scan.entity, &parts),
        Some(index) => keys.index_range(&scan.entity, index, &parts),
    };
    let range = match cursor {
        None => full.clone(),
        Some(cursor) => full
            .resume(cursor, true)
            .ok_or_else(|| ExecError::BadInput("invalid scan cursor".into()))?,
    };
    let limit = scan.batch as usize;
    let entries = tx.scan(&range, limit, Direction::Forward)?;
    let next = match entries.last() {
        Some((last, _)) if entries.len() == limit => full.cursor_after(last),
        _ => None,
    };
    let mut rows = Vec::with_capacity(entries.len());
    for (_, bytes) in entries {
        let bytes = match &scan.index {
            None => bytes,
            Some(_) => {
                let pk: Vec<Value> = serde_json::from_slice(&bytes)
                    .map_err(|e| ExecError::Runtime(format!("corrupt index entry: {e}")))?;
                let pk_parts: Vec<KeyPart<'_>> =
                    pk.iter().filter_map(KeyPart::from_value).collect();
                tx.get(&keys.row(&scan.entity, &pk_parts))?
                    .ok_or_else(|| ExecError::Runtime("index points at a missing row".into()))?
            }
        };
        meter.read(bytes.len())?;
        rows.push(Value::Object(
            serde_json::from_slice(&bytes)
                .map_err(|e| ExecError::Runtime(format!("corrupt row: {e}")))?,
        ));
    }
    Ok((rows, next))
}
