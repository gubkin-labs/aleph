//! Effects: the outbox, integration calls, and delivery to the outside world.
//!
//! A plan never performs an effect directly. Events, jobs, change triggers, channel messages,
//! and integration sends are buffered while it runs and written to the tenant's outbox in the
//! same commit as its writes. A rolled-back or retried attempt therefore releases nothing, and a
//! committed one releases everything exactly once into the outbox. Delivery from the outbox is
//! at-least-once, with retries and backoff.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::keys::TenantId;
use crate::value::Value;

/// An outbox entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum OutboxItem {
    /// Run a handler with a payload.
    Run {
        handler: String,
        payload: BTreeMap<String, Value>,
        attempt: u32,
        /// The job's key, for keyed jobs (which can be replaced and cancelled).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job: Option<JobRef>,
    },
    /// Cancel a keyed job. Only ever pending: it's applied when the outbox is written.
    Cancel { job: JobRef },
    /// Run one batch of a scanning cron, starting after `cursor`.
    Scan {
        handler: String,
        fired_at: i64,
        cursor: Option<String>,
        attempt: u32,
    },
    /// Hand a message to the platform's sink.
    Deliver { delivery: Delivery, attempt: u32 },
}

/// A keyed job: its queue and the key the plan gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JobRef {
    pub(crate) queue: String,
    pub(crate) key: String,
}

/// An effect produced by a plan, waiting for its transaction to commit.
#[derive(Debug, Clone)]
pub(crate) struct Pending {
    pub(crate) delay_ms: i64,
    pub(crate) item: OutboxItem,
}

/// What a delivery is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DeliveryKind {
    /// An integration effect (`send`).
    Send,
    /// A realtime channel message (`publish`).
    Publish,
}

/// A message for the outside world: an integration effect or a channel message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    /// Send or publish.
    pub kind: DeliveryKind,
    /// The integration or channel.
    pub target: String,
    /// The integration operation (sends only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub operation: Option<String>,
    /// The validated input (sends) or `{ key, message }` (publishes).
    #[cfg_attr(feature = "codegen", ts(type = "unknown"))]
    pub data: serde_json::Value,
}

/// Where deliveries go: the platform's integration providers and realtime gateway.
pub trait Sink: Send + Sync {
    /// Delivers one message. An error is retried with backoff.
    fn deliver(&self, tenant: &TenantId, delivery: &Delivery) -> Result<(), String>;
}

/// Accepts and discards every delivery.
#[derive(Debug, Default)]
pub struct NullSink;

impl Sink for NullSink {
    fn deliver(&self, _: &TenantId, _: &Delivery) -> Result<(), String> {
        Ok(())
    }
}

/// Records deliveries, for tests and the dev engine.
#[derive(Debug, Default)]
pub struct MemorySink {
    delivered: Mutex<Vec<(TenantId, Delivery)>>,
}

impl MemorySink {
    /// An empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Removes and returns everything delivered so far.
    pub fn take(&self) -> Vec<(TenantId, Delivery)> {
        std::mem::take(
            &mut *self
                .delivered
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }
}

impl Sink for MemorySink {
    fn deliver(&self, tenant: &TenantId, delivery: &Delivery) -> Result<(), String> {
        self.delivered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((tenant.clone(), delivery.clone()));
        Ok(())
    }
}

/// Runs integration calls: the platform's providers.
pub trait Integrations: Send + Sync {
    /// Calls `integration.operation` with a validated input and returns its raw output.
    fn call(
        &self,
        tenant: &TenantId,
        integration: &str,
        operation: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

/// No providers: every call fails.
#[derive(Debug, Default)]
pub struct NoIntegrations;

impl Integrations for NoIntegrations {
    fn call(
        &self,
        _: &TenantId,
        integration: &str,
        operation: &str,
        _: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err(format!(
            "no provider is configured for {integration}.{operation}"
        ))
    }
}

/// One recorded integration call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RecordedCall {
    /// The integration.
    pub integration: String,
    /// The operation.
    pub operation: String,
    /// The input.
    #[cfg_attr(feature = "codegen", ts(type = "unknown"))]
    pub input: serde_json::Value,
}

/// Canned responses, for tests and the dev engine. Records every call it answers.
#[derive(Debug, Default)]
pub struct MockIntegrations {
    responses: Mutex<BTreeMap<(String, String), Result<serde_json::Value, String>>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl MockIntegrations {
    /// No responses yet: every call fails until one is set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers `integration.operation` with `response` (or fails it with an error).
    pub fn respond(
        &self,
        integration: &str,
        operation: &str,
        response: Result<serde_json::Value, String>,
    ) {
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((integration.to_owned(), operation.to_owned()), response);
    }

    /// Removes and returns every call answered so far.
    pub fn take_calls(&self) -> Vec<RecordedCall> {
        std::mem::take(&mut *self.calls.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Integrations for MockIntegrations {
    fn call(
        &self,
        _: &TenantId,
        integration: &str,
        operation: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                integration: integration.to_owned(),
                operation: operation.to_owned(),
                input: input.clone(),
            });
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(integration.to_owned(), operation.to_owned()))
            .cloned()
            .unwrap_or_else(|| Err(format!("no mock response for {integration}.{operation}")))
    }
}

/// The calls one execution made, so a conflict retry replays them instead of calling again.
#[derive(Debug, Default)]
pub(crate) struct Journal {
    entries: Vec<(String, Result<Value, String>)>,
    position: usize,
}

impl Journal {
    /// Starts a new attempt: replay from the beginning.
    pub(crate) fn rewind(&mut self) {
        self.position = 0;
    }

    /// The recorded result of the next call, if it's the same call (same operation and input).
    pub(crate) fn replay(&mut self, fingerprint: &str) -> Option<Result<Value, String>> {
        let (recorded, result) = self.entries.get(self.position)?;
        if recorded != fingerprint {
            // The attempt diverged (it read different data): forget the rest of the journal.
            self.entries.truncate(self.position);
            return None;
        }
        self.position += 1;
        Some(result.clone())
    }

    /// Records the result of a new call.
    pub(crate) fn record(&mut self, fingerprint: String, result: Result<Value, String>) {
        self.entries.truncate(self.position);
        self.entries.push((fingerprint, result));
        self.position += 1;
    }
}

/// Exponential backoff: `base`, `2 * base`, `4 * base`, ... capped at `max`.
pub(crate) fn backoff_ms(attempt: u32, base: u32, max: u32) -> i64 {
    let factor = 1_u64.checked_shl(attempt.min(30)).unwrap_or(u64::MAX);
    i64::try_from(u64::from(base).saturating_mul(factor).min(u64::from(max))).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let delays: Vec<i64> = (0..6).map(|a| backoff_ms(a, 1_000, 10_000)).collect();
        assert_eq!(delays, [1_000, 2_000, 4_000, 8_000, 10_000, 10_000]);
    }

    #[test]
    fn journal_replays_matching_calls_and_drops_diverged_ones() {
        let mut journal = Journal::default();
        journal.record("a".into(), Ok(Value::Int(1)));
        journal.record("b".into(), Ok(Value::Int(2)));
        journal.rewind();
        assert_eq!(journal.replay("a"), Some(Ok(Value::Int(1))));
        assert_eq!(journal.replay("x"), None); // diverged: "b" is forgotten
        journal.record("x".into(), Ok(Value::Int(3)));
        journal.rewind();
        assert_eq!(journal.replay("a"), Some(Ok(Value::Int(1))));
        assert_eq!(journal.replay("x"), Some(Ok(Value::Int(3))));
    }
}
