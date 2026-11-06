//! Durable workflows: instances that move through steps, each step one bounded transaction.
//!
//! An instance is one record, keyed by the workflow and a key the starter chose. Every step run
//! reads it, runs the step's plan, applies the step's transition (next, wait, complete), and
//! writes it back together with the next outbox entry, all in one commit. A crash between steps
//! loses nothing: the entry that runs the next step is already durable. `seq` numbers every
//! scheduled step, so a stale entry (a timeout after the signal came, a step of a cancelled
//! instance) is recognized and dropped.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::manifest::WorkflowDecl;
use crate::value::Value;

/// Where an instance is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Status {
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

impl Status {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// What a waiting instance waits for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Waiting {
    pub(crate) signal: String,
    pub(crate) then: String,
    pub(crate) on_timeout: String,
}

/// A stored workflow instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Instance {
    pub(crate) status: Status,
    pub(crate) step: String,
    pub(crate) input: BTreeMap<String, Value>,
    pub(crate) state: BTreeMap<String, Value>,
    /// Signals that arrived before the instance waited for them (the latest of each name).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) signals: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) waiting: Option<Waiting>,
    #[serde(default)]
    pub(crate) result: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    /// Steps scheduled so far (at most `max_steps`).
    pub(crate) steps: u32,
    /// The number of the scheduled step; entries with another number are stale.
    pub(crate) seq: u64,
}

impl Instance {
    /// A new instance at its start step, with every state field `null`.
    pub(crate) fn new(decl: &WorkflowDecl, input: BTreeMap<String, Value>) -> Self {
        Self {
            status: Status::Running,
            step: decl.start.clone(),
            input,
            state: decl
                .state
                .keys()
                .map(|k| (k.clone(), Value::Null))
                .collect(),
            signals: BTreeMap::new(),
            waiting: None,
            result: Value::Null,
            error: None,
            steps: 1,
            seq: 1,
        }
    }

    /// What `getWorkflow` returns.
    pub(crate) fn view(&self, key: &str) -> Value {
        Value::Object(BTreeMap::from([
            ("instanceId".to_owned(), Value::String(key.to_owned())),
            (
                "status".to_owned(),
                Value::String(self.status.as_str().to_owned()),
            ),
            ("step".to_owned(), Value::String(self.step.clone())),
            ("state".to_owned(), Value::Object(self.state.clone())),
            ("result".to_owned(), self.result.clone()),
            (
                "error".to_owned(),
                self.error.clone().map_or(Value::Null, Value::String),
            ),
            ("steps".to_owned(), Value::Int(i64::from(self.steps))),
        ]))
    }

    /// The payload a step's plan reads.
    pub(crate) fn payload(
        &self,
        key: &str,
        decl: &WorkflowDecl,
        signal: Option<&(String, Value)>,
        error: Option<&str>,
    ) -> BTreeMap<String, Value> {
        let signals = decl
            .signals
            .keys()
            .map(|name| {
                let value = match signal {
                    Some((s, payload)) if s == name => payload.clone(),
                    _ => Value::Null,
                };
                (name.clone(), value)
            })
            .collect();
        BTreeMap::from([
            ("instanceId".to_owned(), Value::String(key.to_owned())),
            ("input".to_owned(), Value::Object(self.input.clone())),
            ("state".to_owned(), Value::Object(self.state.clone())),
            ("signals".to_owned(), Value::Object(signals)),
            (
                "error".to_owned(),
                error.map_or(Value::Null, |e| Value::String(e.to_owned())),
            ),
        ])
    }

    /// Counts one more step; fails the instance if it would exceed `max_steps`.
    pub(crate) fn advance(&mut self, decl: &WorkflowDecl) -> bool {
        self.seq += 1;
        if self.steps >= decl.max_steps {
            self.status = Status::Failed;
            self.waiting = None;
            self.error = Some(format!(
                "the workflow ran {} steps, its maxSteps",
                decl.max_steps
            ));
            return false;
        }
        self.steps += 1;
        true
    }
}

/// How a step ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Transition {
    Next {
        step: String,
        delay_ms: i64,
    },
    WaitFor {
        signal: String,
        timeout_ms: i64,
        then: String,
        on_timeout: String,
    },
    Complete {
        result: Value,
    },
}

/// The longest delay or wait, in milliseconds: 30 days.
pub(crate) const MAX_WAIT_MS: i64 = 30 * 24 * 60 * 60 * 1000;
