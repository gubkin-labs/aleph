//! Trigger cycles. Each handler run is bounded by the job limits, but a handler can set off
//! another (by writing a row, emitting an event, enqueueing a job, or starting a workflow), and a
//! chain of them that leads back to itself would never stop. This check builds the graph of which
//! handler can set off which, and rejects every cycle unless each handler in it declares
//! `maxDepth`, which the runtime then enforces (see `limits::MAX_CHAIN_DEPTH`). A queue job that
//! re-enqueues its own queue to work through data in batches is a continuation, not a cycle: the
//! runtime bounds it separately (`limits::MAX_CONTINUATIONS`).

use std::collections::{BTreeSet, VecDeque};

use crate::diagnostics::{Diagnostic, DiagnosticCode as Code};
use crate::limits::MAX_CHAIN_DEPTH;
use crate::manifest::{Change, Handler, Manifest, Op, Stmt, Trigger};

/// What a plan can set off.
#[derive(Default)]
struct Fires<'a> {
    changes: BTreeSet<(&'a str, Change)>,
    events: BTreeSet<&'a str>,
    queues: BTreeSet<&'a str>,
    workflows: BTreeSet<&'a str>,
}

impl<'a> Fires<'a> {
    fn of(handler: &'a Handler) -> Self {
        let mut fires = Self::default();
        fires.stmts(&handler.plan.body);
        fires
    }

    fn stmts(&mut self, stmts: &'a [Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::Let { op, .. } => self.op(op),
                Stmt::If {
                    then, otherwise, ..
                } => {
                    self.stmts(then);
                    self.stmts(otherwise);
                }
                Stmt::ForEach { body, .. } => self.stmts(body),
                _ => {}
            }
        }
    }

    fn op(&mut self, op: &'a Op) {
        match op {
            Op::Insert { entity, .. } | Op::InsertIfAbsent { entity, .. } => {
                self.changes.insert((entity, Change::Inserted));
            }
            Op::Upsert { entity, .. } => {
                self.changes.insert((entity, Change::Inserted));
                self.changes.insert((entity, Change::Updated));
            }
            Op::Update { entity, .. } => {
                self.changes.insert((entity, Change::Updated));
            }
            Op::Delete { entity, .. } => {
                self.changes.insert((entity, Change::Deleted));
            }
            Op::Emit { event, .. } => {
                self.events.insert(event);
            }
            Op::Enqueue { queue, .. } => {
                self.queues.insert(queue);
            }
            Op::StartWorkflow { workflow, .. } => {
                self.workflows.insert(workflow);
            }
            Op::Map { body, .. } | Op::Filter { body, .. } | Op::Loop { body, .. } => {
                self.stmts(body);
            }
            _ => {}
        }
    }

    /// Whether running this plan can trigger `target`. The second value says the path goes
    /// through a new workflow instance, whose steps the runtime doesn't count as hops.
    fn triggers(&self, target: &Trigger) -> Option<bool> {
        match target {
            Trigger::Entity { entity, change } => self
                .changes
                .contains(&(entity.as_str(), *change))
                .then_some(false),
            Trigger::Event { event } => self.events.contains(event.as_str()).then_some(false),
            Trigger::Queue { queue } => self.queues.contains(queue.as_str()).then_some(false),
            Trigger::Workflow { workflow, .. } => {
                self.workflows.contains(workflow.as_str()).then_some(true)
            }
            Trigger::DeadLetter { .. } | Trigger::Cron { .. } | Trigger::Upload { .. } => None,
        }
    }
}

/// Reports every trigger cycle, and every invalid `maxDepth`.
pub(crate) fn check(m: &Manifest, diags: &mut Vec<Diagnostic>) {
    let handlers = &m.handlers;
    for (i, handler) in handlers.iter().enumerate() {
        if let Some(depth) = handler.max_depth
            && !(1..=MAX_CHAIN_DEPTH).contains(&depth)
        {
            diags.push(Diagnostic::new(
                Code::InvalidHandler,
                format!("handlers[{i}].maxDepth"),
                format!("maxDepth must be between 1 and {MAX_CHAIN_DEPTH}, got {depth}"),
            ));
        }
    }

    // edges[i]: the handlers a run of handler i can set off, and whether via a workflow start.
    let fires: Vec<Fires<'_>> = handlers.iter().map(Fires::of).collect();
    let edges: Vec<Vec<(usize, bool)>> = handlers
        .iter()
        .enumerate()
        .map(|(i, from)| {
            handlers
                .iter()
                .enumerate()
                .filter_map(|(j, to)| {
                    // A job that fails every retry runs its queue's dead-letter handler.
                    if let (Trigger::Queue { queue }, Trigger::DeadLetter { queue: q }) =
                        (&from.trigger, &to.trigger)
                        && queue == q
                    {
                        return Some((j, false));
                    }
                    // A queue job that enqueues its own queue continues its work in another
                    // batch: the same hop, bounded by `MAX_CONTINUATIONS`, not a cycle.
                    if i == j && matches!(from.trigger, Trigger::Queue { .. }) {
                        return None;
                    }
                    fires[i].triggers(&to.trigger).map(|via| (j, via))
                })
                .collect()
        })
        .collect();

    let mut reported = vec![false; handlers.len()];
    for start in 0..handlers.len() {
        if reported[start] {
            continue;
        }
        let Some(cycle) = shortest_cycle(&edges, start) else {
            continue;
        };
        let members = component(&edges, start);
        for &member in &members {
            reported[member] = true;
        }
        let via_workflow = members
            .iter()
            .any(|&i| edges[i].iter().any(|&(j, via)| via && members.contains(&j)));
        let undeclared: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&i| handlers[i].max_depth.is_none())
            .collect();
        if undeclared.is_empty() && !via_workflow {
            continue;
        }
        let names: Vec<String> = cycle
            .iter()
            .chain(cycle.first())
            .map(|&i| format!("{:?}", handlers[i].id))
            .collect();
        let at = undeclared.first().copied().unwrap_or(start);
        let fix = if via_workflow {
            "It starts a new workflow instance on every turn, so it can't be allowed with \
             maxDepth: break the cycle, for example by starting the workflow from a route."
        } else {
            "Break it (write a different entity, emit a different event, or use a different \
             queue), or, if a guard stops the chain on its own, declare maxDepth on every \
             handler in it."
        };
        diags.push(Diagnostic::new(
            Code::TriggerCycle,
            format!("handlers[{at}]"),
            format!(
                "handlers {} form a trigger cycle: each run can set off the next, without end. \
                 {fix}",
                names.join(" → ")
            ),
        ));
    }
}

/// The handlers on a shortest path from `start` back to itself, in order, starting with `start`.
fn shortest_cycle(edges: &[Vec<(usize, bool)>], start: usize) -> Option<Vec<usize>> {
    let mut parent = vec![None; edges.len()];
    let mut queue = VecDeque::from([start]);
    let mut seen = vec![false; edges.len()];
    while let Some(node) = queue.pop_front() {
        for &(next, _) in &edges[node] {
            if next == start {
                let mut path = vec![node];
                let mut at = node;
                while let Some(p) = parent[at] {
                    path.push(p);
                    at = p;
                }
                path.reverse();
                return Some(path);
            }
            if !seen[next] {
                seen[next] = true;
                parent[next] = Some(node);
                queue.push_back(next);
            }
        }
    }
    None
}

/// The handlers on some cycle through `start`: those it reaches that also reach it.
fn component(edges: &[Vec<(usize, bool)>], start: usize) -> Vec<usize> {
    let reach = |from: usize| {
        let mut seen = vec![false; edges.len()];
        let mut queue = VecDeque::from([from]);
        while let Some(node) = queue.pop_front() {
            for &(next, _) in &edges[node] {
                if !seen[next] {
                    seen[next] = true;
                    queue.push_back(next);
                }
            }
        }
        seen
    };
    let forward = reach(start);
    (0..edges.len())
        .filter(|&i| forward[i] && reach(i)[start])
        .collect()
}
