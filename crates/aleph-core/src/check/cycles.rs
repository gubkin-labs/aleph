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
            Trigger::DeadLetter { .. } | Trigger::Cron { .. } => None,
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

#[cfg(test)]
#[allow(clippy::needless_pass_by_value)]
mod tests {
    use serde_json::{Value as Json, json};

    use super::*;

    const NULL: Json = Json::Null;

    fn lit(value: Json) -> Json {
        json!({ "kind": "lit", "value": value })
    }

    fn let_(op: Json) -> Json {
        json!({ "kind": "let", "var": 0, "op": op })
    }

    fn handler(id: &str, trigger: Json, body: Vec<Json>, max_depth: Option<u32>) -> Json {
        let mut h = json!({
            "id": id,
            "trigger": trigger,
            "plan": { "body": body, "output": lit(NULL) },
        });
        if let Some(depth) = max_depth {
            h["maxDepth"] = json!(depth);
        }
        h
    }

    fn on_event(event: &str) -> Json {
        json!({ "kind": "event", "event": event })
    }

    fn on_entity(entity: &str, change: &str) -> Json {
        json!({ "kind": "entity", "entity": entity, "change": change })
    }

    fn on_queue(queue: &str) -> Json {
        json!({ "kind": "queue", "queue": queue })
    }

    fn emit(event: &str) -> Json {
        let_(json!({ "op": "emit", "event": event, "payload": {} }))
    }

    fn enqueue(queue: &str) -> Json {
        let_(json!({ "op": "enqueue", "queue": queue, "payload": {} }))
    }

    /// The cycle check's diagnostics for these handlers (nothing else is validated).
    fn check_handlers(handlers: Vec<Json>) -> Vec<Diagnostic> {
        let mut base: Json = serde_json::from_str(include_str!(
            "../../../../conformance/invalid/trigger-cycle.json"
        ))
        .unwrap();
        let mut manifest = base["manifest"].take();
        manifest["handlers"] = Json::Array(handlers);
        let manifest: Manifest = serde_json::from_value(manifest).unwrap();
        let mut diags = Vec::new();
        check(&manifest, &mut diags);
        diags
    }

    fn cycles(handlers: Vec<Json>) -> Vec<Diagnostic> {
        let diags = check_handlers(handlers);
        assert!(
            diags.iter().all(|d| d.code == Code::TriggerCycle),
            "{diags:#?}"
        );
        diags
    }

    #[test]
    fn every_kind_of_write_sets_off_its_change_trigger() {
        let cases = [
            (
                json!({ "op": "insert", "entity": "Post", "values": {} }),
                "inserted",
            ),
            (
                json!({ "op": "insertIfAbsent", "entity": "Post", "values": {} }),
                "inserted",
            ),
            (
                json!({ "op": "upsert", "entity": "Post", "values": {} }),
                "inserted",
            ),
            (
                json!({ "op": "upsert", "entity": "Post", "values": {} }),
                "updated",
            ),
            (
                json!({ "op": "update", "entity": "Post", "key": {}, "set": {} }),
                "updated",
            ),
            (
                json!({ "op": "delete", "entity": "Post", "key": {} }),
                "deleted",
            ),
        ];
        for (op, change) in cases {
            let diags = cycles(vec![handler(
                "onPost",
                on_entity("Post", change),
                vec![let_(op.clone())],
                None,
            )]);
            assert_eq!(diags.len(), 1, "{op} on {change}: {diags:#?}");
            assert_eq!(diags[0].path, "handlers[0]");
        }
    }

    #[test]
    fn a_write_only_sets_off_triggers_for_its_own_change_and_entity() {
        let cases = [
            (
                json!({ "op": "insert", "entity": "Post", "values": {} }),
                on_entity("Post", "updated"),
            ),
            (
                json!({ "op": "insert", "entity": "Other", "values": {} }),
                on_entity("Post", "inserted"),
            ),
            (
                json!({ "op": "update", "entity": "Post", "key": {}, "set": {} }),
                on_entity("Post", "deleted"),
            ),
            (
                json!({ "op": "delete", "entity": "Post", "key": {} }),
                on_entity("Post", "inserted"),
            ),
            (
                json!({ "op": "emit", "event": "pong", "payload": {} }),
                on_event("ping"),
            ),
        ];
        for (op, trigger) in cases {
            let diags = cycles(vec![handler(
                "h",
                trigger.clone(),
                vec![let_(op.clone())],
                None,
            )]);
            assert!(diags.is_empty(), "{op} under {trigger}: {diags:#?}");
        }
    }

    #[test]
    fn writes_nested_in_branches_and_loops_still_count() {
        let inner = emit("ping");
        let list = json!({ "kind": "input", "source": "payload", "name": "items" });
        let nested = [
            json!({ "kind": "if", "cond": lit(json!(true)), "then": [inner], "else": [] }),
            json!({ "kind": "if", "cond": lit(json!(true)), "then": [], "else": [inner] }),
            json!({ "kind": "forEach", "list": list, "item": 1, "body": [inner] }),
            let_(
                json!({ "op": "map", "list": list, "item": 1, "body": [inner], "yield": lit(NULL) }),
            ),
            let_(
                json!({ "op": "filter", "list": list, "item": 1, "body": [inner], "keep": lit(json!(true)) }),
            ),
            let_(
                json!({ "op": "loop", "max": 2, "state": {}, "var": 1, "body": [inner], "next": {} }),
            ),
        ];
        for stmt in nested {
            let diags = cycles(vec![handler(
                "onPing",
                on_event("ping"),
                vec![stmt.clone()],
                None,
            )]);
            assert_eq!(diags.len(), 1, "{stmt}: {diags:#?}");
        }
    }

    #[test]
    fn a_two_handler_cycle_is_named_in_order() {
        let diags = cycles(vec![
            handler("onPing", on_event("ping"), vec![emit("pong")], None),
            handler("onPong", on_event("pong"), vec![emit("ping")], None),
        ]);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(
            diags[0]
                .message
                .contains(r#"handlers "onPing" → "onPong" → "onPing" form a trigger cycle"#),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0].message.contains("declare maxDepth"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn the_shortest_cycle_is_reported() {
        // a → b → c → a and a → c → a: the report names the two-hop one.
        let diags = cycles(vec![
            handler("a", on_event("a"), vec![emit("b"), emit("c")], None),
            handler("b", on_event("b"), vec![emit("c")], None),
            handler("c", on_event("c"), vec![emit("a")], None),
        ]);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(
            diags[0]
                .message
                .contains(r#"handlers "a" → "c" → "a" form"#),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn chains_without_a_way_back_are_not_cycles() {
        let diags = cycles(vec![
            handler("a", on_event("a"), vec![emit("b")], None),
            handler("b", on_event("b"), vec![emit("c")], None),
            handler("c", on_event("c"), vec![], None),
        ]);
        assert!(diags.is_empty(), "{diags:#?}");
    }

    #[test]
    fn queues_cycle_through_each_other_but_may_continue_themselves() {
        let diags = cycles(vec![
            handler("jobA", on_queue("a"), vec![enqueue("b")], None),
            handler("jobB", on_queue("b"), vec![enqueue("a")], None),
        ]);
        assert_eq!(diags.len(), 1, "{diags:#?}");

        let diags = cycles(vec![handler(
            "batch",
            on_queue("a"),
            vec![enqueue("a")],
            None,
        )]);
        assert!(diags.is_empty(), "a continuation isn't a cycle: {diags:#?}");
    }

    #[test]
    fn a_dead_letter_handler_that_re_enqueues_its_queue_is_a_cycle() {
        let dead = |queue: &str| json!({ "kind": "deadLetter", "queue": queue });
        let diags = cycles(vec![
            handler("job", on_queue("a"), vec![], None),
            handler("failed", dead("a"), vec![enqueue("a")], None),
        ]);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].path, "handlers[0]");

        // Another queue's dead letters don't lead back.
        let diags = cycles(vec![
            handler("job", on_queue("a"), vec![], None),
            handler("failed", dead("b"), vec![enqueue("a")], None),
        ]);
        assert!(diags.is_empty(), "{diags:#?}");
    }

    #[test]
    fn max_depth_allows_a_cycle_only_when_every_member_declares_it() {
        let both = cycles(vec![
            handler("onPing", on_event("ping"), vec![emit("pong")], Some(3)),
            handler("onPong", on_event("pong"), vec![emit("ping")], Some(3)),
        ]);
        assert!(both.is_empty(), "{both:#?}");

        let one = cycles(vec![
            handler("onPing", on_event("ping"), vec![emit("pong")], Some(3)),
            handler("onPong", on_event("pong"), vec![emit("ping")], None),
        ]);
        assert_eq!(one.len(), 1, "{one:#?}");
        assert_eq!(
            one[0].path, "handlers[1]",
            "points at the undeclared handler"
        );
    }

    #[test]
    fn handlers_off_the_cycle_need_no_max_depth() {
        // ping ↔ pong declare maxDepth; `audit` hangs off the cycle and `kick` leads into it,
        // but neither is on it, so neither needs one.
        let diags = cycles(vec![
            handler(
                "onPing",
                on_event("ping"),
                vec![emit("pong"), emit("audit")],
                Some(3),
            ),
            handler("onPong", on_event("pong"), vec![emit("ping")], Some(3)),
            handler("audit", on_event("audit"), vec![], None),
            handler("kick", on_event("kick"), vec![emit("ping")], None),
        ]);
        assert!(diags.is_empty(), "{diags:#?}");
    }

    #[test]
    fn a_cycle_through_a_workflow_start_is_rejected_even_with_max_depth() {
        let step = handler(
            "step",
            json!({ "kind": "workflow", "workflow": "flow", "step": "start" }),
            vec![let_(json!({
                "op": "startWorkflow", "workflow": "flow", "key": lit(json!("k")), "input": {}
            }))],
            Some(3),
        );
        let diags = cycles(vec![step]);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(
            diags[0].message.contains("new workflow instance"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn max_depth_must_be_within_the_chain_limit() {
        let ok = |depth| check_handlers(vec![handler("h", on_event("ping"), vec![], Some(depth))]);
        assert_eq!(ok(1), []);
        assert_eq!(ok(MAX_CHAIN_DEPTH), []);
        for depth in [0, MAX_CHAIN_DEPTH + 1] {
            let diags = ok(depth);
            assert_eq!(diags.len(), 1, "{depth}: {diags:#?}");
            assert_eq!(diags[0].code, Code::InvalidHandler);
            assert_eq!(diags[0].path, "handlers[0].maxDepth");
        }
    }
}
