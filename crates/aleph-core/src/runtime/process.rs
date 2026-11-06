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
use super::{Engine, Env};
use crate::keys::{Key, KeyPart, Keyspace, TenantId};
use crate::limits::Limits;
use crate::manifest::{CronScan, Handler, QueueDecl, Trigger};
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
        let id = env.new_id(now);
        let bytes =
            serde_json::to_vec(&pending.item).map_err(|e| ExecError::Runtime(e.to_string()))?;
        tx.put(
            keys.outbox(now.saturating_add(pending.delay_ms), &id),
            bytes,
        );
    }
    Ok(())
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
            } = &handler.trigger
            else {
                continue;
            };
            let (every, offset) = (i64::from(*every_ms), i64::from(*offset_ms));
            if now < offset {
                continue;
            }
            let tick = (now - offset) / every * every + offset;
            let mut tx = self.store.begin();
            let state = keys.schedule(&handler.id);
            let last = tx
                .get(&state)
                .ok()
                .flatten()
                .and_then(|b| String::from_utf8(b).ok())
                .and_then(|s| s.parse::<i64>().ok());
            if last.is_some_and(|last| last >= tick) {
                continue;
            }
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
            OutboxItem::Run {
                handler: id,
                payload,
                attempt,
            } => {
                let Some(handler) = self.handler(&id) else {
                    self.remove(key);
                    report.failures.push(format!("unknown handler {id}"));
                    return;
                };
                match self.run_handler(keys, now, key, handler, &payload, None) {
                    Ok(()) => report.ran += 1,
                    Err(error) => {
                        let (retries, base, max) = retry_policy(self, handler);
                        if attempt < retries {
                            let next = OutboxItem::Run {
                                handler: id,
                                payload,
                                attempt: attempt + 1,
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
                ) {
                    Ok(()) => report.ran += 1,
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
    fn run_handler(
        &self,
        keys: &Keyspace,
        now: i64,
        entry: &Key,
        handler: &Handler,
        payload: &BTreeMap<String, Value>,
        scan: Option<(&CronScan, Option<&str>, i64)>,
    ) -> Result<(), ExecError> {
        let env = self.env.as_ref();
        let mut journal = Journal::default();
        for _ in 0..=Limits::JOB.max_conflict_retries {
            let mut tx = self.store.begin();
            journal.rewind();
            // Another worker may already have handled this entry.
            if tx.get(entry)?.is_none() {
                return Ok(());
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
                };
                exec.run()?;
                std::mem::take(&mut exec.effects)
            };
            if let Some(item) = next {
                effects.push(Pending { delay_ms: 0, item });
            }
            write_outbox(&mut *tx, keys, env, now, effects)?;
            tx.delete(entry.clone());
            match tx.commit() {
                Ok(()) => return Ok(()),
                Err(CommitError::Conflict) => {}
                Err(CommitError::Store(error)) => return Err(ExecError::Store(error)),
            }
        }
        Err(ExecError::Runtime(
            "the handler's transaction kept conflicting".into(),
        ))
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
        tx.put(keys.outbox(at, &self.env.new_id(at)), bytes);
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
