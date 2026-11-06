//! Checking asynchronous work: event, queue, channel, and integration declarations, and the
//! handlers that react to them. Handlers are bounded like routes, against the job limits.

use std::collections::{BTreeMap, HashSet};

use super::{
    EntityInfo, RouteReport, Ty, check_field, check_name, is_keyable, plan, unknown_entity,
};
use crate::diagnostics::{Diagnostic, DiagnosticCode as Code};
use crate::limits::{Cost, Limits, Resource};
use crate::manifest::{CronScan, Field, Handler, Manifest, OpMode, QueueDecl, Trigger};

/// Checks every declaration and handler, and records handler bounds in `reports`.
pub(crate) fn check_all(
    m: &Manifest,
    infos: &BTreeMap<String, EntityInfo>,
    reports: &mut BTreeMap<String, RouteReport>,
    diags: &mut Vec<Diagnostic>,
) {
    check_declarations(m, diags);
    let mut ids = HashSet::new();
    let mut queue_handlers = HashSet::new();
    for (i, handler) in m.handlers.iter().enumerate() {
        let path = format!("handlers[{i}]");
        check_name(&handler.id, &format!("{path}.id"), diags);
        if !ids.insert(&handler.id) {
            diags.push(Diagnostic::new(
                Code::InvalidHandler,
                format!("{path}.id"),
                format!("handler id {:?} is used twice", handler.id),
            ));
        }
        if let Trigger::Queue { queue } | Trigger::DeadLetter { queue } = &handler.trigger
            && !queue_handlers.insert((matches!(handler.trigger, Trigger::Queue { .. }), queue))
        {
            diags.push(Diagnostic::new(
                Code::InvalidHandler,
                format!("{path}.trigger"),
                format!(
                    "queue {queue:?} already has a handler of this kind; each queue has one job \
                     handler and at most one dead-letter handler"
                ),
            ));
        }
        for (name, error) in &handler.errors {
            check_name(name, &format!("{path}.errors.{name}"), diags);
            if !(400..=599).contains(&error.status) {
                diags.push(Diagnostic::new(
                    Code::InvalidErrorStatus,
                    format!("{path}.errors.{name}"),
                    "error status must be between 400 and 599",
                ));
            }
        }
        let Some((payload, extra)) = trigger_payload(m, infos, handler, &path, diags) else {
            continue;
        };
        let bounds = plan::check_handler(m, infos, handler, payload, &path, diags) + extra;
        for resource in Resource::ALL {
            let (bound, limit) = (bounds.get(resource), resource.limit(&Limits::JOB));
            if bound > limit {
                diags.push(Diagnostic::new(
                    Code::OverBudget,
                    format!("{path}.plan"),
                    format!(
                        "handler {:?} can use up to {bound} {resource} in the worst case, over the \
                         job limit of {limit}. Lower the bounds that drive it or split the work.",
                        handler.id
                    ),
                ));
            }
        }
        reports.insert(
            handler.id.clone(),
            RouteReport {
                bounds,
                output: None,
            },
        );
    }
    for queue in m.queues.keys() {
        let handled = m
            .handlers
            .iter()
            .any(|h| matches!(&h.trigger, Trigger::Queue { queue: q } if q == queue));
        if !handled {
            diags.push(Diagnostic::new(
                Code::InvalidDeclaration,
                format!("queues.{queue}"),
                format!("queue {queue:?} has no job handler, so its jobs would never run"),
            ));
        }
    }
}

fn payload_types(fields: &BTreeMap<String, Field>) -> BTreeMap<String, Ty> {
    fields
        .iter()
        .map(|(n, f)| (n.clone(), Ty::from_field(f)))
        .collect()
}

/// The payload a handler's trigger provides, and the cost the trigger itself adds (reading a
/// scan batch and scheduling the next one). `None` if the trigger is invalid.
#[allow(clippy::too_many_lines, reason = "one arm per trigger kind")]
fn trigger_payload(
    m: &Manifest,
    infos: &BTreeMap<String, EntityInfo>,
    handler: &Handler,
    path: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<(BTreeMap<String, Ty>, Cost)> {
    let trigger_path = format!("{path}.trigger");
    let mut missing = |what: &str, name: &str| {
        diags.push(Diagnostic::new(
            Code::InvalidHandler,
            &trigger_path,
            format!("unknown {what} {name:?}"),
        ));
        None
    };
    match &handler.trigger {
        Trigger::Event { event } => match m.events.get(event) {
            Some(decl) => Some((payload_types(&decl.payload), Cost::ZERO)),
            None => missing("event", event),
        },
        Trigger::Queue { queue } => match m.queues.get(queue) {
            Some(decl) => Some((payload_types(&decl.payload), Cost::ZERO)),
            None => missing("queue", queue),
        },
        Trigger::DeadLetter { queue } => match m.queues.get(queue) {
            Some(decl) => {
                let mut payload = payload_types(&decl.payload);
                payload.insert("error".into(), Ty::string(4096));
                payload.insert("attempts".into(), Ty::Int { max: None });
                Some((payload, Cost::ZERO))
            }
            None => missing("queue", queue),
        },
        Trigger::Entity { entity, .. } => {
            if !m.entities.contains_key(entity) {
                return missing("entity", entity);
            }
            let row = Ty::OptRow(entity.clone());
            let payload = BTreeMap::from([("before".into(), row.clone()), ("after".into(), row)]);
            Some((payload, Cost::ZERO))
        }
        Trigger::Workflow {
            workflow,
            step,
            on_fail,
        } => {
            let Some(decl) = m.workflows.get(workflow) else {
                return missing("workflow", workflow);
            };
            check_name(step, &format!("{trigger_path}.step"), diags);
            if let Some(target) = on_fail
                && !plan::workflow_steps(m, workflow).any(|s| s == target)
            {
                diags.push(Diagnostic::new(
                    Code::InvalidHandler,
                    format!("{trigger_path}.onFail"),
                    format!("workflow {workflow:?} has no step {target:?}"),
                ));
            }
            let instance = plan::workflow_instance_ty(m, workflow, decl);
            let state = match &instance {
                Ty::Object(fields) => fields.get("state").cloned().unwrap_or(Ty::Unknown),
                _ => Ty::Unknown,
            };
            let signals = decl
                .signals
                .iter()
                .map(|(n, fields)| (n.clone(), Ty::Object(payload_types(fields)).nullable()))
                .collect();
            let payload = BTreeMap::from([
                ("instanceId".to_owned(), Ty::string(256)),
                ("input".to_owned(), Ty::Object(payload_types(&decl.input))),
                ("state".to_owned(), state),
                ("signals".to_owned(), Ty::Object(signals)),
                ("error".to_owned(), Ty::string(4096).nullable()),
            ]);
            // Reading and writing the instance, and scheduling the next step and a timeout.
            let extra = Cost {
                reads: 1,
                writes: 1,
                effects: 2,
                bytes_read: Limits::JOB.max_row_bytes,
                ..Cost::ZERO
            };
            Some((payload, extra))
        }
        Trigger::Cron {
            every_ms,
            offset_ms,
            scan,
            schedule,
        } => {
            const MAX_EVERY_MS: u32 = 30 * 24 * 60 * 60 * 1000;
            if let Some(schedule) = schedule {
                if let Err(e) = crate::runtime::parse_cron(&schedule.cron) {
                    diags.push(Diagnostic::new(
                        Code::InvalidHandler,
                        format!("{trigger_path}.schedule.cron"),
                        e,
                    ));
                }
                if let Some(tz) = &schedule.tz
                    && !crate::runtime::time_zone_exists(tz)
                {
                    diags.push(Diagnostic::new(
                        Code::InvalidHandler,
                        format!("{trigger_path}.schedule.tz"),
                        format!("{tz:?} isn't an IANA time zone"),
                    ));
                }
                if *every_ms != 0 || *offset_ms != 0 {
                    diags.push(Diagnostic::new(
                        Code::InvalidHandler,
                        &trigger_path,
                        "a cron has either a schedule or an interval (everyMs), not both",
                    ));
                }
            } else if !(1_000..=MAX_EVERY_MS).contains(every_ms) || offset_ms >= every_ms {
                diags.push(Diagnostic::new(
                    Code::InvalidHandler,
                    &trigger_path,
                    format!(
                        "cron everyMs must be between 1000 and {MAX_EVERY_MS}, and offsetMs below it"
                    ),
                ));
            }
            let mut payload = BTreeMap::from([("firedAt".to_owned(), Ty::Int { max: None })]);
            let Some(scan) = scan else {
                return Some((payload, Cost::ZERO));
            };
            let batch = check_scan(m, scan, &format!("{trigger_path}.scan"), diags)?;
            let row_bytes = infos.get(&scan.entity).map_or(0, |info| info.row_bytes);
            let rows = Ty::List {
                item: Box::new(Ty::Row(scan.entity.clone())),
                max: batch,
            };
            payload.insert("batch".into(), rows);
            let extra = Cost {
                reads: batch,
                bytes_read: batch.saturating_mul(row_bytes),
                effects: 1,
                ..Cost::ZERO
            };
            Some((payload, extra))
        }
    }
}

fn check_scan(
    m: &Manifest,
    scan: &CronScan,
    path: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<u64> {
    let Some(entity) = m.entities.get(&scan.entity) else {
        diags.push(unknown_entity(&format!("{path}.entity"), &scan.entity, m));
        return None;
    };
    let fields = match &scan.index {
        None => entity.key.len(),
        Some(index) => {
            if let Some(ix) = entity.indexes.get(index) {
                ix.fields.len()
            } else {
                diags.push(Diagnostic::new(
                    Code::UnknownIndex,
                    format!("{path}.index"),
                    format!("unknown index {index:?} on {}", scan.entity),
                ));
                return None;
            }
        }
    };
    if scan.prefix.len() > fields {
        diags.push(Diagnostic::new(
            Code::InvalidPrefix,
            format!("{path}.prefix"),
            "prefix is longer than the scanned key",
        ));
    }
    if !(1..=CronScan::MAX_BATCH).contains(&scan.batch) {
        diags.push(Diagnostic::new(
            Code::InvalidHandler,
            format!("{path}.batch"),
            format!("batch must be between 1 and {}", CronScan::MAX_BATCH),
        ));
    }
    Some(u64::from(scan.batch))
}

fn check_fields(fields: &BTreeMap<String, Field>, path: &str, diags: &mut Vec<Diagnostic>) {
    for (name, field) in fields {
        let field_path = format!("{path}.{name}");
        check_name(name, &field_path, diags);
        check_field(field, &field_path, diags);
    }
}

/// Validates events, queues, channels, and integrations.
fn check_workflows(m: &Manifest, diags: &mut Vec<Diagnostic>) {
    for (name, decl) in &m.workflows {
        let path = format!("workflows.{name}");
        check_name(name, &path, diags);
        check_fields(&decl.input, &format!("{path}.input"), diags);
        check_fields(&decl.state, &format!("{path}.state"), diags);
        for (signal, fields) in &decl.signals {
            check_name(signal, &format!("{path}.signals.{signal}"), diags);
            check_fields(fields, &format!("{path}.signals.{signal}"), diags);
        }
        let mut problem = |at: String, message: String| {
            diags.push(Diagnostic::new(Code::InvalidDeclaration, at, message));
        };
        if !(1..=1000).contains(&decl.max_steps) {
            problem(format!("{path}.maxSteps"), "maxSteps must be 1-1000".into());
        }
        if decl.retries > QueueDecl::MAX_RETRIES {
            problem(
                format!("{path}.retries"),
                "a step retries at most 10 times".into(),
            );
        }
        let steps: Vec<&str> = plan::workflow_steps(m, name).collect();
        if !steps.contains(&decl.start.as_str()) {
            problem(
                format!("{path}.start"),
                format!(
                    "start step {:?} has no handler (steps: {steps:?})",
                    decl.start
                ),
            );
        }
        let mut seen = HashSet::new();
        for step in &steps {
            if !seen.insert(*step) {
                problem(path.clone(), format!("step {step:?} is declared twice"));
            }
        }
    }
}

fn check_declarations(m: &Manifest, diags: &mut Vec<Diagnostic>) {
    check_workflows(m, diags);
    for (name, event) in &m.events {
        let path = format!("events.{name}");
        check_name(name, &path, diags);
        check_fields(&event.payload, &format!("{path}.payload"), diags);
    }
    for (name, queue) in &m.queues {
        let path = format!("queues.{name}");
        check_name(name, &path, diags);
        check_fields(&queue.payload, &format!("{path}.payload"), diags);
        if queue.retries > QueueDecl::MAX_RETRIES
            || queue.backoff_ms == 0
            || queue.max_backoff_ms < queue.backoff_ms
        {
            diags.push(Diagnostic::new(
                Code::InvalidDeclaration,
                &path,
                format!(
                    "retries must be at most {}, backoffMs positive, and maxBackoffMs at least backoffMs",
                    QueueDecl::MAX_RETRIES
                ),
            ));
        }
    }
    for (name, channel) in &m.channels {
        let path = format!("channels.{name}");
        check_name(name, &path, diags);
        check_fields(&channel.key, &format!("{path}.key"), diags);
        check_fields(&channel.message, &format!("{path}.message"), diags);
        if channel
            .key
            .values()
            .any(|f| !is_keyable(&f.ty) || f.optional)
        {
            diags.push(Diagnostic::new(
                Code::InvalidDeclaration,
                format!("{path}.key"),
                "channel key fields must be required scalars",
            ));
        }
    }
    for (name, integration) in &m.integrations {
        let path = format!("integrations.{name}");
        check_name(name, &path, diags);
        if integration.provider.is_empty() {
            diags.push(Diagnostic::new(
                Code::InvalidDeclaration,
                format!("{path}.provider"),
                "an integration needs a provider",
            ));
        }
        for (op_name, op) in &integration.operations {
            let op_path = format!("{path}.operations.{op_name}");
            check_name(op_name, &op_path, diags);
            check_fields(&op.input, &format!("{op_path}.input"), diags);
            check_fields(&op.output, &format!("{op_path}.output"), diags);
            if op.mode == OpMode::Effect && !op.output.is_empty() {
                diags.push(Diagnostic::new(
                    Code::InvalidDeclaration,
                    &op_path,
                    "effect operations have no output; make it a call to read a response",
                ));
            }
        }
    }
}
