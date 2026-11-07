//! Plan checking: scoping, types, and worst-case cost.
//!
//! Every operation has a known cost and every list has a known maximum length, so the worst case
//! is a fold over the plan tree: sequences add, branches take the maximum, and loops multiply by
//! their bound. There's no recursion or unbounded looping in the format, so this always
//! terminates.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::types::{CALLER_ID_MAX, CURSOR_MAX};
use super::{EntityInfo, Ty, unknown_entity, unknown_field};
use crate::diagnostics::{Diagnostic, DiagnosticCode as Code};
use crate::limits::Cost;
use crate::manifest::{
    AggOp, BinaryOp, BucketDecl, Change, Entity, ErrorDecl, Expr, Field, FieldType, Handler, Input,
    InputSource, Manifest, Op, OpMode, Route, RowPredicate, Stmt, StrOp, Trigger, VarId,
    WorkflowDecl,
};

/// The match case that catches every other value.
pub(crate) const DEFAULT_CASE: &str = "_";

/// What checking a route's plan found.
pub(crate) struct RouteCheck {
    /// Worst-case cost.
    pub(crate) cost: Cost,
    /// The type of every response the plan can give (early returns, then the output), with the
    /// manifest path of each.
    pub(crate) outputs: Vec<(Ty, String)>,
}

/// Checks a route's plan and returns its worst-case cost and response types.
pub(crate) fn check_route(
    m: &Manifest,
    infos: &BTreeMap<String, EntityInfo>,
    route: &Route,
    path: &str,
    diags: &mut Vec<Diagnostic>,
) -> RouteCheck {
    let mut checker = Checker::new(m, infos, diags);
    checker.errors = Some(&route.errors);
    checker.input = Some(&route.input);
    let plan_path = format!("{path}.plan");
    checker.scopes.push(HashMap::new());
    let mut cost = Cost::default();
    for (i, stmt) in route.plan.body.iter().enumerate() {
        cost += checker.stmt(stmt, &format!("{plan_path}.body[{i}]"), false);
    }
    let output_path = format!("{plan_path}.output");
    let (output, output_cost) = checker.expr(&route.plan.output, &output_path);
    let mut outputs = std::mem::take(&mut checker.outputs);
    outputs.push((output, output_path));
    // The response value, whichever path produced it.
    let response_memory = outputs
        .iter()
        .map(|(ty, _)| checker.memory(ty))
        .fold(Cost::ZERO, Cost::max);
    RouteCheck {
        cost: cost + output_cost + response_memory,
        outputs,
    }
}

/// Checks a handler's plan against its payload and returns its worst-case cost.
pub(crate) fn check_handler(
    m: &Manifest,
    infos: &BTreeMap<String, EntityInfo>,
    handler: &Handler,
    payload: BTreeMap<String, Ty>,
    path: &str,
    diags: &mut Vec<Diagnostic>,
) -> Cost {
    let mut checker = Checker::new(m, infos, diags);
    checker.errors = Some(&handler.errors);
    checker.payload = Some(payload);
    if let Trigger::Workflow { workflow, .. } = &handler.trigger {
        checker.workflow = m.workflows.get(workflow).map(|d| (workflow.as_str(), d));
    }
    let plan_path = format!("{path}.plan");
    checker.scopes.push(HashMap::new());
    let mut cost = Cost::default();
    for (i, stmt) in handler.plan.body.iter().enumerate() {
        cost += checker.stmt(stmt, &format!("{plan_path}.body[{i}]"), false);
    }
    let (output, output_cost) = checker.expr(&handler.plan.output, &format!("{plan_path}.output"));
    let outputs = std::mem::take(&mut checker.outputs);
    let response_memory = outputs
        .iter()
        .map(|(ty, _)| ty)
        .chain(std::iter::once(&output))
        .map(|ty| checker.memory(ty))
        .fold(Cost::ZERO, Cost::max);
    cost + output_cost + response_memory
}

/// How many handlers react to a change of an entity's rows.
pub(crate) fn change_handlers(m: &Manifest, entity: &str, change: Change) -> u64 {
    m.handlers
        .iter()
        .filter(|h| matches!(&h.trigger, Trigger::Entity { entity: e, change: c } if e == entity && *c == change))
        .count() as u64
}

/// How many handlers react to an event.
pub(crate) fn event_handlers(m: &Manifest, event: &str) -> u64 {
    m.handlers
        .iter()
        .filter(|h| matches!(&h.trigger, Trigger::Event { event: e } if e == event))
        .count() as u64
}

/// Checks a counter's `where` predicate: a pure boolean expression over one row.
/// What a row expression must evaluate to.
#[derive(Clone, Copy)]
pub(crate) enum RowExprKind {
    Bool,
    Int,
    /// A key-able scalar (an index part).
    Scalar,
}

pub(crate) fn check_row_predicate(
    m: &Manifest,
    entity: &str,
    predicate: &RowPredicate,
    path: &str,
    kind: RowExprKind,
    diags: &mut Vec<Diagnostic>,
) {
    let infos = BTreeMap::new();
    let mut checker = Checker::new(m, &infos, diags);
    checker.scopes.push(HashMap::new());
    checker.bind(predicate.var, Ty::Row(entity.to_owned()), path);
    let expr_path = format!("{path}.expr");
    if let Some(problem) = impurity(&predicate.expr) {
        checker.diag(
            Code::InvalidPredicate,
            &expr_path,
            format!("a row expression must depend only on the row, but it uses {problem}"),
        );
    }
    let (ty, _) = checker.expr(&predicate.expr, &expr_path);
    match kind {
        RowExprKind::Bool if !ty.is_boolish() => checker.type_mismatch(&expr_path, "bool", &ty),
        RowExprKind::Int if !ty.is_intish() => checker.type_mismatch(&expr_path, "int", &ty),
        RowExprKind::Scalar
            if matches!(
                ty.non_null(),
                Ty::List { .. } | Ty::Object(_) | Ty::Row(_) | Ty::OptRow(_)
            ) =>
        {
            checker.type_mismatch(&expr_path, "a string, int, or bool", &ty);
        }
        _ => {}
    }
}

/// Returns what makes an expression depend on more than its variables, if anything.
fn impurity(expr: &Expr) -> Option<&'static str> {
    match expr {
        Expr::Input { .. } => Some("a route input"),
        Expr::Caller | Expr::CallerRole { .. } | Expr::CallerProfile { .. } => Some("the caller"),
        Expr::Now => Some("the current time"),
        Expr::Lit { .. } | Expr::Var { .. } | Expr::Decimal { .. } => None,
        Expr::Field { of, .. }
        | Expr::IsNull { of }
        | Expr::Length { of }
        | Expr::Str { of, .. }
        | Expr::Truncate { of, .. }
        | Expr::Abs { arg: of }
        | Expr::First { of }
        | Expr::Not { arg: of }
        | Expr::Neg { arg: of }
        | Expr::In { value: of, .. }
        | Expr::Distinct { of }
        | Expr::Pad { of, .. }
        | Expr::Matches { of, .. }
        | Expr::FormatNumber { of, .. }
        | Expr::Round { of, .. }
        | Expr::ToInt { of, .. } => impurity(of),
        Expr::Binary { lhs, rhs, .. }
        | Expr::StrTest {
            of: lhs, arg: rhs, ..
        }
        | Expr::Includes {
            list: lhs,
            value: rhs,
        }
        | Expr::Sort {
            list: lhs, by: rhs, ..
        }
        | Expr::Split {
            of: lhs, sep: rhs, ..
        } => impurity(lhs).or_else(|| impurity(rhs)),
        Expr::Agg { list, value, .. } => {
            impurity(list).or_else(|| value.as_deref().and_then(impurity))
        }
        Expr::Slice { of, start, end } => impurity(of)
            .or_else(|| impurity(start))
            .or_else(|| end.as_deref().and_then(impurity)),
        Expr::Replace { of, find, with } => impurity(of)
            .or_else(|| impurity(find))
            .or_else(|| impurity(with)),
        Expr::StartOf { of, tz, .. }
        | Expr::TimePart { of, tz, .. }
        | Expr::FormatTime { of, tz, .. } => {
            impurity(of).or_else(|| tz.as_deref().and_then(impurity))
        }
        Expr::AddTime { of, amount, tz, .. } => impurity(of)
            .or_else(|| impurity(amount))
            .or_else(|| tz.as_deref().and_then(impurity)),
        Expr::Select {
            cond,
            then,
            otherwise,
        } => impurity(cond)
            .or_else(|| impurity(then))
            .or_else(|| impurity(otherwise)),
        Expr::Match { on, cases } => impurity(on).or_else(|| cases.values().find_map(impurity)),
        Expr::Object { fields } => fields.values().find_map(impurity),
        Expr::List { items: parts } | Expr::Concat { parts } | Expr::ListConcat { parts } => {
            parts.iter().find_map(impurity)
        }
    }
}

/// The step names of a workflow (its step handlers).
pub(crate) fn workflow_steps<'m>(
    m: &'m Manifest,
    workflow: &'m str,
) -> impl Iterator<Item = &'m str> {
    m.handlers.iter().filter_map(move |h| match &h.trigger {
        Trigger::Workflow {
            workflow: w, step, ..
        } if w == workflow => Some(step.as_str()),
        _ => None,
    })
}

/// The type of a workflow instance as `getWorkflow` reads it.
pub(crate) fn workflow_instance_ty(m: &Manifest, workflow: &str, decl: &WorkflowDecl) -> Ty {
    let state = decl
        .state
        .iter()
        .map(|(n, f)| (n.clone(), Ty::from_field(f).nullable()))
        .collect();
    let steps: Vec<String> = workflow_steps(m, workflow).map(str::to_owned).collect();
    Ty::Object(BTreeMap::from([
        ("instanceId".to_owned(), Ty::string(256)),
        (
            "status".to_owned(),
            Ty::Enum(
                ["running", "waiting", "completed", "failed", "cancelled"]
                    .map(str::to_owned)
                    .to_vec(),
            ),
        ),
        ("step".to_owned(), Ty::Enum(steps)),
        ("state".to_owned(), Ty::Object(state)),
        ("result".to_owned(), Ty::Unknown),
        ("error".to_owned(), Ty::string(4096).nullable()),
        (
            "steps".to_owned(),
            Ty::Int {
                max: Some(i64::from(decl.max_steps)),
            },
        ),
    ]))
}

fn field_kind(ty: &FieldType) -> &'static str {
    match ty {
        FieldType::String { .. } => "string",
        FieldType::Int { .. } => "int",
        FieldType::Decimal { .. } => "decimal",
        FieldType::Bool => "bool",
        FieldType::Enum { .. } => "enum",
        FieldType::Id => "id",
        FieldType::Timestamp => "timestamp",
        FieldType::TimeZone => "time zone",
        FieldType::Array { .. } => "array",
        FieldType::Object { .. } => "object",
    }
}

/// `result`, made nullable if `operand` may be `null` (operations pass `null` through).
fn keep_null(operand: &Ty, result: Ty) -> Ty {
    if matches!(operand, Ty::Null | Ty::Opt(_)) {
        result.nullable()
    } else {
        result
    }
}

struct Checker<'a> {
    m: &'a Manifest,
    infos: &'a BTreeMap<String, EntityInfo>,
    /// Errors the plan may fail with (a route's or a handler's).
    errors: Option<&'a BTreeMap<String, ErrorDecl>>,
    /// A route's inputs.
    input: Option<&'a Input>,
    /// A handler's payload types.
    payload: Option<BTreeMap<String, Ty>>,
    diags: &'a mut Vec<Diagnostic>,
    scopes: Vec<HashMap<VarId, Ty>>,
    /// Every variable defined anywhere in the plan. Ids are unique per plan, even across
    /// branches, so a plan never shadows.
    defined: HashSet<VarId>,
    /// The types of early `return` values, with their paths.
    outputs: Vec<(Ty, String)>,
    /// The workflow whose step this is, if any.
    workflow: Option<(&'a str, &'a WorkflowDecl)>,
}

impl<'a> Checker<'a> {
    fn new(
        m: &'a Manifest,
        infos: &'a BTreeMap<String, EntityInfo>,
        diags: &'a mut Vec<Diagnostic>,
    ) -> Self {
        Self {
            m,
            infos,
            errors: None,
            input: None,
            payload: None,
            diags,
            scopes: Vec::new(),
            defined: HashSet::new(),
            outputs: Vec::new(),
            workflow: None,
        }
    }

    fn diag(&mut self, code: Code, path: &str, message: impl Into<String>) {
        self.diags.push(Diagnostic::new(code, path, message));
    }

    fn type_mismatch(&mut self, path: &str, expected: &str, found: &Ty) {
        self.diag(
            Code::TypeMismatch,
            path,
            format!("expected {expected}, found {}", found.describe()),
        );
    }

    /// Checks a state-carrying loop. The state's type is found by running the pass once on the
    /// initial state's type (diagnostics discarded), joining, and running it again: the second
    /// pass must keep that type, or the state would grow without bound. Integers are widened
    /// first, so counters don't count as growth.
    #[allow(clippy::too_many_arguments)]
    fn loop_op(
        &mut self,
        max: u32,
        state: &BTreeMap<String, Expr>,
        var: VarId,
        cond: Option<&Expr>,
        body: &[Stmt],
        next: &BTreeMap<String, Expr>,
        path: &str,
        own: Cost,
    ) -> (Ty, Cost) {
        if !(1..=Op::MAX_LOOP_PASSES).contains(&max) {
            self.diag(
                Code::InvalidExpression,
                &format!("{path}.max"),
                format!("a loop runs 1 to {} passes, got {max}", Op::MAX_LOOP_PASSES),
            );
        }
        if !state.keys().eq(next.keys()) {
            self.diag(
                Code::InvalidExpression,
                &format!("{path}.next"),
                "a loop's next state must set every state field, and only those",
            );
        }
        let mut init_cost = Cost::ZERO;
        let mut initial = BTreeMap::new();
        for (name, expr) in state {
            let (ty, cost) = self.expr(expr, &format!("{path}.state.{name}"));
            init_cost += cost;
            initial.insert(name.clone(), widen_ints(ty));
        }
        let start = Ty::Object(initial);
        let (diags, defined) = (self.diags.len(), self.defined.clone());
        let (probe, _) = self.loop_pass(&start, var, cond, body, next, path);
        self.diags.truncate(diags);
        self.defined = defined;
        let ty = start.join(widen_ints(probe));
        let (after, pass_cost) = self.loop_pass(&ty, var, cond, body, next, path);
        if ty.clone().join(widen_ints(after.clone())) != ty {
            self.diag(
                Code::TypeMismatch,
                &format!("{path}.next"),
                format!(
                    "the loop's state grows from one pass to the next (from {} to {}), so it has \
                     no bound. Bound what grows (.truncate(n), .take(n)) so every pass keeps the \
                     same type",
                    ty.describe_fields(),
                    after.describe_fields()
                ),
            );
        }
        let per_pass = pass_cost + Cost::ops(1) + self.memory(&ty);
        (ty, own + init_cost + per_pass.times(u64::from(max)))
    }

    /// One pass of a loop on a state of type `state`: the type of its next state, and its cost.
    fn loop_pass(
        &mut self,
        state: &Ty,
        var: VarId,
        cond: Option<&Expr>,
        body: &[Stmt],
        next: &BTreeMap<String, Expr>,
        path: &str,
    ) -> (Ty, Cost) {
        self.scopes.push(HashMap::new());
        self.bind(var, state.clone(), path);
        let mut cost = match cond {
            Some(cond) => self.condition(cond, &format!("{path}.while")),
            None => Cost::ZERO,
        };
        cost += self.stmts(body, &format!("{path}.body"), true);
        let mut fields = BTreeMap::new();
        for (name, expr) in next {
            let (ty, c) = self.expr(expr, &format!("{path}.next.{name}"));
            cost += c;
            fields.insert(name.clone(), ty);
        }
        self.scopes.pop();
        (Ty::Object(fields), cost)
    }

    fn bind(&mut self, var: VarId, ty: Ty, path: &str) {
        if !self.defined.insert(var) {
            self.diag(
                Code::DuplicateVar,
                path,
                format!("variable {var} is defined more than once"),
            );
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(var, ty);
        }
    }

    /// Binds an expression's own variable (an aggregate's or sort's item). Expressions can be
    /// shared by reference in a plan, so these may repeat; they're visible only inside it.
    fn bind_scoped(&mut self, var: VarId, ty: Ty) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(var, ty);
        }
    }

    /// After `ensure(cond)` passes, what `cond` proves holds for the rest of the block: a variable
    /// that isn't null (`not(isNull(v))`, as `getOrFail` records) loses its `null`.
    fn narrow(&mut self, cond: &Expr) {
        match cond {
            Expr::Not { arg } => {
                if let Expr::IsNull { of } = &**arg
                    && let Expr::Var { id } = &**of
                    && let Some(ty) = self.lookup(*id).cloned()
                {
                    let narrowed = match ty {
                        Ty::OptRow(entity) => Ty::Row(entity),
                        other => other.non_null().clone(),
                    };
                    // Shadow it in the current block only: a narrowing inside a branch must
                    // not leak out of it.
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(*id, narrowed);
                    }
                }
            }
            Expr::Binary {
                op: BinaryOp::And,
                lhs,
                rhs,
            } => {
                self.narrow(lhs);
                self.narrow(rhs);
            }
            _ => {}
        }
    }

    /// Checks a value written to a declared field: it must not be `null` unless the field is
    /// optional, and it must be the right kind of value.
    fn check_write(&mut self, ty: &Ty, field: &Field, path: &str) {
        let may_be_null = matches!(ty, Ty::Null | Ty::Opt(_) | Ty::OptRow(_));
        if may_be_null && !field.optional {
            self.diag(
                Code::TypeMismatch,
                path,
                format!(
                    "the value may be null, but the field is required. Check it first (getOrFail, \
                     $.ensure(x.isSet())) or give a fallback with .ifNull(...); found {}",
                    ty.describe()
                ),
            );
            return;
        }
        let ok = matches!(
            (ty.non_null(), &field.ty),
            (Ty::Unknown | Ty::Null, _)
                | (Ty::Bool, FieldType::Bool)
                | (
                    Ty::Int { .. },
                    FieldType::Int { .. } | FieldType::Timestamp | FieldType::Decimal { .. },
                )
                | (Ty::Decimal { .. }, FieldType::Decimal { .. })
                | (
                    Ty::String { .. } | Ty::Enum(_),
                    FieldType::String { .. }
                        | FieldType::Id
                        | FieldType::TimeZone
                        | FieldType::Enum { .. }
                        | FieldType::Decimal { .. },
                )
                | (Ty::List { .. }, FieldType::Array { .. })
                | (
                    Ty::Object(_) | Ty::Row(_) | Ty::OptRow(_),
                    FieldType::Object { .. }
                )
        );
        if !ok {
            self.type_mismatch(
                path,
                &format!("a value for a {} field", field_kind(&field.ty)),
                ty,
            );
            return;
        }
        // Inside objects and lists, every declared field and item must fit too.
        match (ty.non_null(), &field.ty) {
            (Ty::Object(found), FieldType::Object { fields }) => {
                for (name, inner) in fields {
                    match found.get(name) {
                        Some(found) => self.check_write(found, inner, &format!("{path}.{name}")),
                        None if inner.optional => {}
                        None => self.diag(
                            Code::MissingRequiredField,
                            path,
                            format!("the object needs field {name:?}"),
                        ),
                    }
                }
            }
            (Ty::List { item, .. }, FieldType::Array { items, .. }) => {
                let item_field = Field {
                    ty: (**items).clone(),
                    optional: false,
                    default: None,
                    deprecated: None,
                };
                self.check_write(item, &item_field, &format!("{path}[]"));
            }
            _ => {}
        }
    }

    fn lookup(&self, var: VarId) -> Option<&Ty> {
        self.scopes.iter().rev().find_map(|scope| scope.get(&var))
    }

    fn entity(&mut self, name: &str, path: &str) -> Option<&'a Entity> {
        let entity = self.m.entities.get(name);
        if entity.is_none() {
            let diag = unknown_entity(path, name, self.m);
            self.diags.push(diag);
        }
        entity
    }

    /// Checks statements in a new scope.
    fn block(&mut self, stmts: &[Stmt], path: &str, pure: bool) -> Cost {
        self.scopes.push(HashMap::new());
        let cost = self.stmts(stmts, path, pure);
        self.scopes.pop();
        cost
    }

    /// Checks statements in the current scope.
    fn stmts(&mut self, stmts: &[Stmt], path: &str, pure: bool) -> Cost {
        let mut cost = Cost::default();
        for (i, stmt) in stmts.iter().enumerate() {
            cost += self.stmt(stmt, &format!("{path}[{i}]"), pure);
        }
        cost
    }

    #[allow(clippy::too_many_lines, reason = "one arm per statement kind")]
    fn stmt(&mut self, stmt: &Stmt, path: &str, pure: bool) -> Cost {
        let own = Cost::ops(1);
        match stmt {
            Stmt::Let { var, op } => {
                let (ty, cost) = self.op(op, &format!("{path}.op"), pure);
                let memory = self.memory(&ty);
                self.bind(*var, ty, path);
                own + cost + memory
            }
            Stmt::Ensure {
                cond,
                error,
                details,
            } => {
                let details = self.error_ref(error, details, path);
                let cost = own + self.condition(cond, &format!("{path}.cond")) + details;
                self.narrow(cond);
                cost
            }
            Stmt::Fail { error, details } => own + self.error_ref(error, details, path),
            Stmt::Return { value } => {
                if pure {
                    self.diag(
                        Code::WriteInPureBlock,
                        path,
                        "return can't be used inside map or filter",
                    );
                }
                let (ty, cost) = self.expr(value, &format!("{path}.value"));
                self.outputs.push((ty, format!("{path}.value")));
                own + cost
            }
            Stmt::SetHeader { name, value } => {
                self.response_stmt(pure, path);
                if let Err(message) = crate::check::check_header_name(name) {
                    self.diag(Code::InvalidRoute, &format!("{path}.name"), message);
                }
                let (_, cost) = self.string(value, &format!("{path}.value"));
                own + cost
            }
            Stmt::SetStatus { status } => {
                self.response_stmt(pure, path);
                if !(200..=399).contains(status) {
                    self.diag(
                        Code::InvalidErrorStatus,
                        &format!("{path}.status"),
                        "success statuses are 200-399; fail with a declared error for 4xx and 5xx",
                    );
                }
                own
            }
            Stmt::Log { value, .. } => own + self.expr(value, &format!("{path}.value")).1,
            Stmt::Next {
                step,
                delay_ms,
                state,
            } => {
                let mut cost = own + self.transition(pure, path, &[step], state);
                if let Some(delay) = delay_ms {
                    cost += self.int(delay, &format!("{path}.delayMs"));
                }
                cost
            }
            Stmt::WaitFor {
                signal,
                timeout_ms,
                then,
                on_timeout,
                state,
            } => {
                let mut cost = own + self.transition(pure, path, &[then, on_timeout], state);
                if let Some((name, decl)) = self.workflow
                    && !decl.signals.contains_key(signal)
                {
                    let known: Vec<&String> = decl.signals.keys().collect();
                    self.diag(
                        Code::InvalidHandler,
                        &format!("{path}.signal"),
                        format!("workflow {name:?} has no signal {signal:?} (declared: {known:?})"),
                    );
                }
                cost += self.int(timeout_ms, &format!("{path}.timeoutMs"));
                cost
            }
            Stmt::Complete { result, state } => {
                let mut cost = own + self.transition(pure, path, &[], state);
                if let Some(result) = result {
                    cost += self.expr(result, &format!("{path}.result")).1;
                }
                cost
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                let cond_cost = self.condition(cond, &format!("{path}.cond"));
                let then_cost = self.block(then, &format!("{path}.then"), pure);
                let else_cost = self.block(otherwise, &format!("{path}.else"), pure);
                own + cond_cost + then_cost.max(else_cost)
            }
            Stmt::ForEach { list, item, body } => {
                let (list_ty, list_cost) = self.expr(list, &format!("{path}.list"));
                let Some((item_ty, max)) = self.bounded_list(list_ty, &format!("{path}.list"))
                else {
                    return own + list_cost;
                };
                self.scopes.push(HashMap::new());
                let item_memory = self.memory(&item_ty);
                self.bind(*item, item_ty, path);
                let body_cost = self.block(body, &format!("{path}.body"), pure);
                self.scopes.pop();
                own + list_cost + body_cost.plus(Cost::ops(1) + item_memory).times(max)
            }
        }
    }

    fn condition(&mut self, cond: &Expr, path: &str) -> Cost {
        let (ty, cost) = self.expr(cond, path);
        if !ty.is_boolish() {
            self.type_mismatch(path, "bool", &ty);
        }
        cost
    }

    /// Checks a workflow transition: only in a workflow step (outside map/filter), to steps that
    /// exist, with state values that fit the declared state.
    fn transition(
        &mut self,
        pure: bool,
        path: &str,
        steps: &[&String],
        state: &BTreeMap<String, Expr>,
    ) -> Cost {
        let Some((name, decl)) = self.workflow.filter(|_| !pure) else {
            self.diag(
                Code::InvalidHandler,
                path,
                "next, waitFor, and complete can only end a workflow step (outside map and filter)",
            );
            return Cost::ZERO;
        };
        for step in steps {
            if !workflow_steps(self.m, name).any(|s| s == step.as_str()) {
                let known: Vec<&str> = workflow_steps(self.m, name).collect();
                self.diag(
                    Code::InvalidHandler,
                    path,
                    format!("workflow {name:?} has no step {step:?} (steps: {known:?})"),
                );
            }
        }
        self.state_values(name, &decl.state, state, &format!("{path}.state"))
    }

    /// Checks values for workflow state fields (all optional: state starts `null`).
    fn state_values(
        &mut self,
        workflow: &str,
        fields: &BTreeMap<String, Field>,
        given: &BTreeMap<String, Expr>,
        path: &str,
    ) -> Cost {
        let mut cost = Cost::ZERO;
        for (field_name, value) in given {
            let value_path = format!("{path}.{field_name}");
            let (ty, value_cost) = self.expr(value, &value_path);
            cost += value_cost;
            match fields.get(field_name) {
                Some(field) => {
                    let optional = Field {
                        optional: true,
                        ..field.clone()
                    };
                    self.check_write(&ty, &optional, &value_path);
                }
                None => {
                    let known: Vec<&String> = fields.keys().collect();
                    self.diag(
                        Code::UnknownField,
                        &value_path,
                        format!("workflow {workflow:?} has no state field {field_name:?} (state: {known:?})"),
                    );
                }
            }
        }
        cost
    }

    /// The workflow declaration an op names, or a diagnostic.
    fn workflow_decl(&mut self, workflow: &str, path: &str) -> Option<&'a WorkflowDecl> {
        let decl = self.m.workflows.get(workflow);
        if decl.is_none() {
            let known: Vec<&String> = self.m.workflows.keys().collect();
            self.diag(
                Code::InvalidDeclaration,
                &format!("{path}.workflow"),
                format!("unknown workflow {workflow:?} (declared: {known:?})"),
            );
        }
        decl
    }

    /// Statements that shape the HTTP response only make sense in routes, outside map/filter.
    fn response_stmt(&mut self, pure: bool, path: &str) {
        if pure || self.input.is_none() {
            self.diag(
                Code::InvalidRoute,
                path,
                "response headers and statuses can only be set in a route's plan, outside map and filter",
            );
        }
    }

    /// Checks a reference to a declared error and the details it's given.
    fn error_ref(&mut self, name: &str, details: &BTreeMap<String, Expr>, path: &str) -> Cost {
        let decl = self.errors.and_then(|errors| errors.get(name));
        if let Some(decl) = decl {
            return self.values(&decl.details, details, &format!("{path}.details"), false);
        }
        let path = &format!("{path}.error");
        {
            let known: Vec<&str> = self
                .errors
                .map(|errors| errors.keys().map(String::as_str).collect())
                .unwrap_or_default();
            self.diag(
                Code::UnknownError,
                path,
                format!("unknown error {name:?} (declared: {known:?})"),
            );
        }
        Cost::ZERO
    }

    /// Resolves a list type to its item type and maximum length.
    fn bounded_list(&mut self, ty: Ty, path: &str) -> Option<(Ty, u64)> {
        let ty = match ty {
            Ty::Opt(inner) => *inner,
            other => other,
        };
        match ty {
            Ty::List { item, max } => Some((*item, max)),
            other => {
                self.diag(
                    Code::UnboundedList,
                    path,
                    format!(
                        "expected a list with a known maximum length, found {}. Lists come from query \
                         results, array inputs with a max, list literals, or map/filter over those.",
                        other.describe()
                    ),
                );
                None
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per operation; each arm delegates"
    )]
    fn op(&mut self, op: &Op, path: &str, pure: bool) -> (Ty, Cost) {
        if pure && (op.is_write() || op.is_effect()) {
            self.diag(
                Code::WriteInPureBlock,
                path,
                "map and filter bodies may read but never write, so writes can't hide inside a transform. \
                 Compute the values with map, then write them with forEach.",
            );
        }
        if let Op::Update { entity, .. } | Op::Upsert { entity, .. } | Op::Delete { entity, .. } =
            op
            && self.m.entities.get(entity).is_some_and(|e| !e.mutable)
        {
            self.diag(
                Code::ImmutableEntity,
                path,
                format!(
                    "{entity} is append-only: rows can be inserted, but not updated or deleted"
                ),
            );
        }
        let own = Cost::ops(1);
        match op {
            Op::Get { entity, key } => {
                let Some(e) = self.entity(entity, &format!("{path}.entity")) else {
                    return (Ty::Unknown, own);
                };
                let key_cost = self.key_map(e, key, &format!("{path}.key"));
                let read = Cost {
                    reads: 1,
                    bytes_read: self.row_bytes(entity),
                    ..Cost::default()
                };
                (Ty::OptRow(entity.clone()), own + key_cost + read)
            }
            Op::Insert { entity, values } => {
                let (ty, cost) = self.insert(entity, values, path, 1, &[Change::Inserted]);
                (ty, own + cost)
            }
            Op::Upsert { entity, values } => {
                let (ty, cost) = self.insert(
                    entity,
                    values,
                    path,
                    2,
                    &[Change::Inserted, Change::Updated],
                );
                (ty, own + cost)
            }
            Op::InsertIfAbsent { entity, values } => {
                let (_, cost) = self.insert(entity, values, path, 1, &[Change::Inserted]);
                (Ty::Bool, own + cost)
            }
            Op::Update {
                entity,
                key,
                set,
                if_version,
            } => {
                let (ty, mut cost) = self.update(entity, key, set, path);
                if let Some(expected) = if_version {
                    if !self.m.entities.get(entity).is_some_and(|e| e.versioned) {
                        self.diag(
                            Code::InvalidRoute,
                            &format!("{path}.ifVersion"),
                            format!("{entity} isn't versioned; declare it with versioned: true"),
                        );
                    }
                    cost += self.int(expected, &format!("{path}.ifVersion"));
                }
                (ty, own + cost)
            }
            Op::Delete { entity, key } => {
                let Some(e) = self.entity(entity, &format!("{path}.entity")) else {
                    return (Ty::Unknown, own);
                };
                let cost = own
                    + self.key_map(e, key, &format!("{path}.key"))
                    + self.write_cost(entity, 1, &[Change::Deleted]);
                (Ty::Null, cost)
            }
            Op::Query {
                entity,
                index,
                prefix,
                limit,
                order: _,
                range,
                after,
                page,
                any_of,
            } => {
                let (list, mut cost) = self.query(entity, index.as_deref(), prefix, limit, path);
                if let Some(values) = any_of {
                    let any_path = format!("{path}.anyOf");
                    if values.is_empty() || values.len() > 10 {
                        self.diag(Code::InvalidPrefix, &any_path, "anyOf takes 1-10 values");
                    }
                    if after.is_some() || *page {
                        self.diag(
                            Code::InvalidPrefix,
                            &any_path,
                            "anyOf merges several ranges, so it can't be paged with a cursor",
                        );
                    }
                    let parts = self.m.entities.get(entity).and_then(|e| match index {
                        None => Some(e.key.len()),
                        Some(name) => e.indexes.get(name).map(|ix| ix.fields.len()),
                    });
                    if parts.is_some_and(|parts| prefix.len() >= parts) {
                        self.diag(Code::InvalidPrefix, &any_path, "anyOf gives values for the field after the prefix, but the prefix already covers every field");
                    }
                    for (i, value) in values.iter().enumerate() {
                        cost += self.expr(value, &format!("{any_path}[{i}]")).1;
                    }
                    // Each value's range is scanned up to the limit, then merged.
                    let k = values.len().max(1) as u64;
                    let max = match &list {
                        Ty::List { max, .. } => *max,
                        _ => 0,
                    };
                    let row = self.row_bytes(entity);
                    cost += Cost {
                        reads: (k - 1) * max,
                        bytes_read: (k - 1) * max * row,
                        ops: k * max,
                        ..Cost::ZERO
                    };
                }
                if let Some(range) = range {
                    cost += self.range(
                        entity,
                        index.as_deref(),
                        prefix.len() + usize::from(any_of.is_some()),
                        range,
                        &format!("{path}.range"),
                    );
                }
                if let Some(after) = after {
                    let (ty, after_cost) = self.expr(after, &format!("{path}.after"));
                    if !matches!(ty.non_null(), Ty::String { .. } | Ty::Null | Ty::Unknown) {
                        self.type_mismatch(&format!("{path}.after"), "cursor string or null", &ty);
                    }
                    cost += after_cost;
                }
                let ty = if *page {
                    Ty::Object(BTreeMap::from([
                        ("items".to_owned(), list),
                        ("cursor".to_owned(), Ty::string(CURSOR_MAX).nullable()),
                    ]))
                } else {
                    list
                };
                (ty, own + cost)
            }
            Op::CounterGet { counter, key } => {
                let cost = self.counter_get(counter, key, path);
                let ty = match self.m.counters.get(counter).and_then(|c| {
                    let agg = c.agg.as_ref()?;
                    Some((agg, self.m.entities.get(&c.of)?.fields.get(&agg.field)?))
                }) {
                    Some((agg, field)) if agg.kind != crate::manifest::AggKind::Distinct => {
                        Ty::from_field_type(&field.ty).nullable()
                    }
                    _ => Ty::Int { max: None },
                };
                (ty, own + cost)
            }
            Op::Map {
                list,
                item,
                body,
                result,
            } => {
                let (list_ty, list_cost) = self.expr(list, &format!("{path}.list"));
                let Some((item_ty, max)) = self.bounded_list(list_ty, &format!("{path}.list"))
                else {
                    return (Ty::Unknown, own + list_cost);
                };
                // The body and the yield share one scope: the yield may use the body's variables.
                self.scopes.push(HashMap::new());
                let item_memory = self.memory(&item_ty);
                self.bind(*item, item_ty, path);
                let body_cost = self.stmts(body, &format!("{path}.body"), true);
                let (result_ty, result_cost) = self.expr(result, &format!("{path}.yield"));
                self.scopes.pop();
                let per_item = body_cost + result_cost + Cost::ops(1) + item_memory;
                (
                    Ty::List {
                        item: Box::new(result_ty),
                        max,
                    },
                    own + list_cost + per_item.times(max),
                )
            }
            Op::Loop {
                max,
                state,
                var,
                cond,
                body,
                next,
            } => self.loop_op(*max, state, *var, cond.as_ref(), body, next, path, own),
            Op::Filter {
                list,
                item,
                body,
                keep,
            } => {
                let (list_ty, list_cost) = self.expr(list, &format!("{path}.list"));
                let Some((item_ty, max)) = self.bounded_list(list_ty, &format!("{path}.list"))
                else {
                    return (Ty::Unknown, own + list_cost);
                };
                self.scopes.push(HashMap::new());
                let item_memory = self.memory(&item_ty);
                self.bind(*item, item_ty.clone(), path);
                let body_cost = self.stmts(body, &format!("{path}.body"), true);
                let keep_cost = self.condition(keep, &format!("{path}.keep"));
                self.scopes.pop();
                let per_item = body_cost + keep_cost + Cost::ops(1) + item_memory;
                (
                    Ty::List {
                        item: Box::new(item_ty),
                        max,
                    },
                    own + list_cost + per_item.times(max),
                )
            }
            Op::Emit { event, payload } => {
                let cost = match self.m.events.get(event) {
                    Some(decl) => {
                        self.values(&decl.payload, payload, &format!("{path}.payload"), false)
                    }
                    None => {
                        self.unknown(
                            Code::UnknownEvent,
                            &format!("{path}.event"),
                            "event",
                            event,
                            self.m.events.keys(),
                        );
                        Cost::default()
                    }
                };
                let effects = Cost {
                    effects: event_handlers(self.m, event),
                    ..Cost::ZERO
                };
                (Ty::Null, own + cost + effects)
            }
            Op::Enqueue {
                queue,
                payload,
                delay_ms,
                key,
            } => {
                let mut cost = match self.m.queues.get(queue) {
                    Some(decl) => {
                        self.values(&decl.payload, payload, &format!("{path}.payload"), false)
                    }
                    None => {
                        self.unknown(
                            Code::UnknownQueue,
                            &format!("{path}.queue"),
                            "queue",
                            queue,
                            self.m.queues.keys(),
                        );
                        Cost::default()
                    }
                };
                if let Some(key) = key {
                    cost += self.string(key, &format!("{path}.key")).1;
                }
                if let Some(delay) = delay_ms {
                    let (ty, delay_cost) = self.expr(delay, &format!("{path}.delayMs"));
                    if !ty.is_intish() {
                        self.type_mismatch(&format!("{path}.delayMs"), "int", &ty);
                    }
                    cost += delay_cost;
                }
                (
                    Ty::Null,
                    own + cost
                        + Cost {
                            effects: 1,
                            ..Cost::ZERO
                        },
                )
            }
            Op::Publish {
                channel,
                key,
                message,
            } => {
                let cost = match self.m.channels.get(channel) {
                    Some(decl) => {
                        self.values(&decl.key, key, &format!("{path}.key"), true)
                            + self.values(&decl.message, message, &format!("{path}.message"), false)
                    }
                    None => {
                        self.unknown(
                            Code::UnknownChannel,
                            &format!("{path}.channel"),
                            "channel",
                            channel,
                            self.m.channels.keys(),
                        );
                        Cost::default()
                    }
                };
                (
                    Ty::Null,
                    own + cost
                        + Cost {
                            effects: 1,
                            ..Cost::ZERO
                        },
                )
            }
            Op::UploadUrl {
                bucket,
                key,
                content_type,
                expires_s,
            } => {
                let decl = self.bucket(bucket, path);
                let mut cost = own + self.string(key, &format!("{path}.key")).1;
                cost += self.string(content_type, &format!("{path}.contentType")).1;
                if let (
                    Some(decl),
                    Expr::Lit {
                        value: crate::manifest::Literal::String(literal),
                    },
                ) = (decl, content_type.as_ref())
                    && !decl.allows(literal)
                {
                    self.diag(
                        Code::InvalidExpression,
                        &format!("{path}.contentType"),
                        format!(
                            "bucket {bucket:?} doesn't accept {literal:?} files (it allows {:?})",
                            decl.content_types
                        ),
                    );
                }
                self.expires(*expires_s, path);
                let fields = BTreeMap::from([
                    ("url".into(), Ty::string(u64::from(BucketDecl::MAX_URL_LEN))),
                    ("key".into(), Ty::string(u64::from(BucketDecl::MAX_KEY_LEN))),
                    ("expiresAt".into(), Ty::Int { max: None }),
                ]);
                (Ty::Object(fields), cost)
            }
            Op::DownloadUrl {
                bucket,
                key,
                expires_s,
            } => {
                self.bucket(bucket, path);
                let cost = own + self.string(key, &format!("{path}.key")).1;
                self.expires(*expires_s, path);
                let fields = BTreeMap::from([
                    ("url".into(), Ty::string(u64::from(BucketDecl::MAX_URL_LEN))),
                    ("expiresAt".into(), Ty::Int { max: None }),
                ]);
                (Ty::Object(fields), cost)
            }
            Op::GetBlob { bucket, key } => {
                self.bucket(bucket, path);
                let cost = own + self.string(key, &format!("{path}.key")).1;
                let read = Cost {
                    reads: 1,
                    bytes_read: crate::runtime::MAX_BLOB_INFO_BYTES,
                    ..Cost::ZERO
                };
                (Ty::Object(blob_info_fields()).nullable(), cost + read)
            }
            Op::DeleteBlob { bucket, key } => {
                self.bucket(bucket, path);
                let cost = own + self.string(key, &format!("{path}.key")).1;
                let write = Cost {
                    writes: 1,
                    effects: 1,
                    ..Cost::ZERO
                };
                (Ty::Null, cost + write)
            }
            Op::Call {
                integration,
                operation,
                input,
            }
            | Op::Send {
                integration,
                operation,
                input,
            } => {
                let call = matches!(op, Op::Call { .. });
                let (mode, expected) = if call {
                    (OpMode::Call, "call")
                } else {
                    (OpMode::Effect, "effect")
                };
                let Some(decl) = self.m.integrations.get(integration) else {
                    self.unknown(
                        Code::UnknownIntegration,
                        &format!("{path}.integration"),
                        "integration",
                        integration,
                        self.m.integrations.keys(),
                    );
                    return (Ty::Unknown, own);
                };
                let Some(op_decl) = decl.operations.get(operation) else {
                    self.unknown(
                        Code::UnknownOperation,
                        &format!("{path}.operation"),
                        "operation",
                        operation,
                        decl.operations.keys(),
                    );
                    return (Ty::Unknown, own);
                };
                if op_decl.mode != mode {
                    self.diag(
                        Code::UnknownOperation,
                        &format!("{path}.operation"),
                        format!(
                            "{integration}.{operation} isn't a {expected} operation; use {}",
                            if call { "send" } else { "call" }
                        ),
                    );
                }
                let cost = self.values(&op_decl.input, input, &format!("{path}.input"), false);
                if call {
                    let output = op_decl
                        .output
                        .iter()
                        .map(|(n, f)| (n.clone(), Ty::from_field(f)))
                        .collect();
                    (
                        Ty::Object(output),
                        own + cost
                            + Cost {
                                calls: 1,
                                ..Cost::ZERO
                            },
                    )
                } else {
                    (
                        Ty::Null,
                        own + cost
                            + Cost {
                                effects: 1,
                                ..Cost::ZERO
                            },
                    )
                }
            }
            Op::Eval { expr } => {
                let (ty, cost) = self.expr(expr, &format!("{path}.expr"));
                (ty, own + cost)
            }
            Op::StartWorkflow {
                workflow,
                key,
                input,
            } => {
                let mut cost = own + self.string(key, &format!("{path}.key")).1;
                if let Some(decl) = self.workflow_decl(workflow, path) {
                    cost += self.values(&decl.input, input, &format!("{path}.input"), false);
                    for (field_name, value) in input {
                        if let Some(field) = decl.input.get(field_name) {
                            let (ty, _) = self.expr(value, &format!("{path}.input.{field_name}"));
                            self.check_write(&ty, field, &format!("{path}.input.{field_name}"));
                        }
                    }
                }
                let start = Cost {
                    reads: 1,
                    writes: 1,
                    effects: 1,
                    bytes_read: crate::limits::Limits::ROUTE.max_row_bytes,
                    ..Cost::ZERO
                };
                (Ty::Bool, cost + start)
            }
            Op::SignalWorkflow {
                workflow,
                key,
                signal,
                payload,
            } => {
                let mut cost = own + self.string(key, &format!("{path}.key")).1;
                if let Some(decl) = self.workflow_decl(workflow, path) {
                    match decl.signals.get(signal) {
                        Some(fields) => {
                            cost += self.values(fields, payload, &format!("{path}.payload"), false);
                        }
                        None => {
                            let known: Vec<&String> = decl.signals.keys().collect();
                            self.diag(
                                Code::InvalidDeclaration,
                                &format!("{path}.signal"),
                                format!("workflow {workflow:?} has no signal {signal:?} (declared: {known:?})"),
                            );
                        }
                    }
                }
                let effect = Cost {
                    effects: 1,
                    ..Cost::ZERO
                };
                (Ty::Null, cost + effect)
            }
            Op::GetWorkflow { workflow, key } => {
                let cost = own + self.string(key, &format!("{path}.key")).1;
                let ty = match self.workflow_decl(workflow, path) {
                    Some(decl) => workflow_instance_ty(self.m, workflow, decl).nullable(),
                    None => Ty::Unknown,
                };
                let read = Cost {
                    reads: 1,
                    bytes_read: crate::limits::Limits::ROUTE.max_row_bytes,
                    ..Cost::ZERO
                };
                (ty, cost + read)
            }
            Op::CancelWorkflow { workflow, key } => {
                let cost = own + self.string(key, &format!("{path}.key")).1;
                self.workflow_decl(workflow, path);
                let effect = Cost {
                    effects: 1,
                    ..Cost::ZERO
                };
                (Ty::Null, cost + effect)
            }
            Op::Cancel { queue, key } => {
                if !self.m.queues.contains_key(queue) {
                    self.unknown(
                        Code::UnknownQueue,
                        &format!("{path}.queue"),
                        "queue",
                        queue,
                        self.m.queues.keys(),
                    );
                }
                let (_, cost) = self.string(key, &format!("{path}.key"));
                (
                    Ty::Null,
                    own + cost
                        + Cost {
                            effects: 1,
                            ..Cost::ZERO
                        },
                )
            }
            Op::GetUser { user_id } => {
                let (_, cost) = self.string(user_id, &format!("{path}.userId"));
                let profile = &self.m.auth.profile;
                let mut fields: BTreeMap<String, Ty> = profile
                    .iter()
                    .map(|(n, f)| (n.clone(), Ty::from_field(f)))
                    .collect();
                fields.insert("userId".into(), Ty::string(CALLER_ID_MAX));
                let bytes = crate::check::size::profile_bytes(profile);
                (
                    Ty::Object(fields).nullable(),
                    own + cost
                        + Cost {
                            reads: 1,
                            bytes_read: bytes,
                            ..Cost::ZERO
                        },
                )
            }
        }
    }

    /// Checks an insert-like operation. `per_index` is 2 for upserts, which may update.
    fn insert(
        &mut self,
        entity: &str,
        values: &BTreeMap<String, Expr>,
        path: &str,
        per_index: u64,
        changes: &[Change],
    ) -> (Ty, Cost) {
        let Some(e) = self.entity(entity, &format!("{path}.entity")) else {
            return (Ty::Unknown, Cost::default());
        };
        let mut cost = self.write_cost(entity, per_index, changes);
        for (name, value) in values {
            let value_path = format!("{path}.values.{name}");
            let (ty, value_cost) = self.expr(value, &value_path);
            match e.fields.get(name) {
                None => self.diags.push(unknown_field(&value_path, name, e)),
                Some(field) => self.check_write(&ty, field, &value_path),
            }
            cost += value_cost;
        }
        for (name, field) in &e.fields {
            if values.contains_key(name) || field.default.is_some() {
                continue;
            }
            if e.key.contains(name) {
                self.diag(
                    Code::MissingKeyField,
                    &format!("{path}.values"),
                    format!("key field {name:?} needs a value (or a default)"),
                );
            } else if !field.optional {
                self.diag(
                    Code::MissingRequiredField,
                    &format!("{path}.values"),
                    format!("required field {name:?} needs a value"),
                );
            }
        }
        (Ty::Row(entity.to_owned()), cost)
    }

    fn update(
        &mut self,
        entity: &str,
        key: &BTreeMap<String, Expr>,
        set: &BTreeMap<String, Expr>,
        path: &str,
    ) -> (Ty, Cost) {
        let Some(e) = self.entity(entity, &format!("{path}.entity")) else {
            return (Ty::Unknown, Cost::default());
        };
        let mut cost = self.key_map(e, key, &format!("{path}.key"))
            + self.write_cost(entity, 2, &[Change::Updated]);
        for (name, value) in set {
            let value_path = format!("{path}.set.{name}");
            let (ty, value_cost) = self.expr(value, &value_path);
            match e.fields.get(name) {
                None => self.diags.push(unknown_field(&value_path, name, e)),
                Some(_) if e.key.contains(name) => self.diag(
                    Code::ImmutableKeyField,
                    &value_path,
                    format!("key field {name:?} can't be updated"),
                ),
                Some(field) => self.check_write(&ty, field, &value_path),
            }
            cost += value_cost;
        }
        (Ty::Row(entity.to_owned()), cost)
    }

    fn query(
        &mut self,
        entity: &str,
        index: Option<&str>,
        prefix: &[Expr],
        limit: &Expr,
        path: &str,
    ) -> (Ty, Cost) {
        let Some(e) = self.entity(entity, &format!("{path}.entity")) else {
            return (Ty::Unknown, Cost::default());
        };
        let fields = match index {
            None => Some(e.key.len()),
            Some(name) => match e.indexes.get(name) {
                Some(ix) => Some(ix.fields.len()),
                None => {
                    let known: Vec<&str> = e.indexes.keys().map(String::as_str).collect();
                    self.diag(
                        Code::UnknownIndex,
                        &format!("{path}.index"),
                        format!("unknown index {name:?} on {entity} (declared: {known:?})"),
                    );
                    None
                }
            },
        };
        if let Some(fields) = fields
            && prefix.len() > fields
        {
            self.diag(
                Code::InvalidPrefix,
                &format!("{path}.prefix"),
                format!(
                    "prefix has {} values but the scanned key has only {fields} fields",
                    prefix.len()
                ),
            );
        }
        let mut cost = Cost::default();
        for (i, value) in prefix.iter().enumerate() {
            cost += self.expr(value, &format!("{path}.prefix[{i}]")).1;
        }
        let limit_path = format!("{path}.limit");
        let (limit_ty, limit_cost) = self.expr(limit, &limit_path);
        cost += limit_cost;
        let max = match limit_ty {
            Ty::Int { max: Some(max) } => max.max(0).cast_unsigned(),
            other => {
                self.diag(
                            Code::UnboundedLimit,
                            &limit_path,
                            format!(
                                "query limit must have a static upper bound, found {}. Use a literal, an int \
                                 input with a max, or $.min(limit, N).",
                                other.describe()
                            ),
                        );
                return (
                    Ty::List {
                        item: Box::new(Ty::Row(entity.to_owned())),
                        max: 0,
                    },
                    cost,
                );
            }
        };
        let read = Cost {
            reads: max,
            bytes_read: max.saturating_mul(self.row_bytes(entity)),
            ..Cost::default()
        };
        (
            Ty::List {
                item: Box::new(Ty::Row(entity.to_owned())),
                max,
            },
            cost + read,
        )
    }

    fn counter_get(&mut self, counter: &str, key: &[Expr], path: &str) -> Cost {
        let decl = self.m.counters.get(counter);
        let shards = decl.and_then(|c| c.shards).map_or(1, u64::from);
        let bytes = match decl.and_then(|c| c.agg.as_ref()) {
            // A min/max entry holds the value.
            Some(_) => 64,
            None => 20,
        };
        let mut cost = Cost {
            reads: shards,
            bytes_read: shards * bytes,
            ..Cost::default()
        };
        for (i, value) in key.iter().enumerate() {
            cost += self.expr(value, &format!("{path}.key[{i}]")).1;
        }
        match self.m.counters.get(counter) {
            None => {
                let known: Vec<&str> = self.m.counters.keys().map(String::as_str).collect();
                self.diag(
                    Code::UnknownCounter,
                    &format!("{path}.counter"),
                    format!("unknown counter {counter:?} (declared: {known:?})"),
                );
            }
            Some(c) if c.group_by.len() != key.len() => {
                let message = format!(
                    "counter {counter:?} is grouped by {:?}, so it needs {} key values, got {}",
                    c.group_by,
                    c.group_by.len(),
                    key.len()
                );
                self.diag(Code::InvalidCounterKey, &format!("{path}.key"), message);
            }
            Some(_) => {}
        }
        cost
    }

    /// Checks values given for declared fields: no unknown names, and every required field (or,
    /// with `exact`, every field) present.
    fn values(
        &mut self,
        fields: &BTreeMap<String, Field>,
        given: &BTreeMap<String, Expr>,
        path: &str,
        exact: bool,
    ) -> Cost {
        let mut cost = Cost::default();
        for (name, value) in given {
            let value_path = format!("{path}.{name}");
            if !fields.contains_key(name) {
                let known: Vec<&String> = fields.keys().collect();
                self.diag(
                    Code::InvalidPayload,
                    &value_path,
                    format!("unknown field {name:?} (fields: {known:?})"),
                );
            }
            cost += self.expr(value, &value_path).1;
        }
        for (name, field) in fields {
            let required = exact || (!field.optional && field.default.is_none());
            if required && !given.contains_key(name) {
                self.diag(
                    Code::InvalidPayload,
                    path,
                    format!("missing field {name:?}"),
                );
            }
        }
        cost
    }

    fn unknown<'k>(
        &mut self,
        code: Code,
        path: &str,
        what: &str,
        name: &str,
        known: impl Iterator<Item = &'k String>,
    ) {
        let known: Vec<&String> = known.collect();
        self.diag(
            code,
            path,
            format!("unknown {what} {name:?} (declared: {known:?})"),
        );
    }

    /// Checks a query's range bounds: a field must follow the prefix, with at most one lower and
    /// one upper bound.
    fn range(
        &mut self,
        entity: &str,
        index: Option<&str>,
        prefix_len: usize,
        range: &crate::manifest::QueryRange,
        path: &str,
    ) -> Cost {
        let parts = self.m.entities.get(entity).and_then(|e| match index {
            None => Some(e.key.len()),
            Some(name) => e.indexes.get(name).map(|ix| ix.fields.len()),
        });
        if parts.is_some_and(|parts| prefix_len >= parts) {
            self.diag(Code::InvalidRange, path, "a range bounds the field after the prefix, but the prefix already covers every field");
        }
        if range.gt.is_some() && range.gte.is_some() || range.lt.is_some() && range.lte.is_some() {
            self.diag(
                Code::InvalidRange,
                path,
                "use at most one lower bound (gt or gte) and one upper bound (lt or lte)",
            );
        }
        let mut cost = Cost::default();
        for (name, bound) in [
            ("gt", &range.gt),
            ("gte", &range.gte),
            ("lt", &range.lt),
            ("lte", &range.lte),
        ] {
            if let Some(bound) = bound {
                cost += self.expr(bound, &format!("{path}.{name}")).1;
            }
        }
        cost
    }

    /// The memory a bound value of type `ty` can take (values of unknown size count as a
    /// maximum row; the runtime meter is the backstop for those).
    fn memory(&self, ty: &Ty) -> Cost {
        let rows = |entity: &str| self.row_bytes(entity);
        Cost {
            memory: super::size::ty_bytes(ty, &rows, 64 * 1024),
            ..Cost::ZERO
        }
    }

    fn row_bytes(&self, entity: &str) -> u64 {
        self.infos.get(entity).map_or(0, |info| info.row_bytes)
    }

    /// Worst-case cost of one row write. Every write first reads the row it replaces (to keep
    /// indexes and counters consistent), and `per_index` is 2 for updates, which may move an index
    /// entry (delete + insert) or move a row between counter groups.
    /// The write also releases one effect per handler of each change it may make.
    fn write_cost(&self, entity: &str, per_index: u64, changes: &[Change]) -> Cost {
        let mut writes = self.infos.get(entity).map_or(1, |info| {
            1 + per_index * info.index_count + per_index * info.counter_count
        });
        // An insert into a TTL entity may first purge an expired row with the same key.
        if changes.contains(&Change::Inserted)
            && self.m.entities.get(entity).is_some_and(|e| e.ttl.is_some())
        {
            writes += self
                .infos
                .get(entity)
                .map_or(1, |info| 1 + info.index_count + info.counter_count);
        }
        // Each unique index is probed before its entry is written.
        let probes = self.infos.get(entity).map_or(0, |info| info.unique_count);
        let effects = changes
            .iter()
            .map(|c| change_handlers(self.m, entity, *c))
            .max()
            .unwrap_or(0);
        Cost {
            reads: 1 + probes,
            writes,
            bytes_read: self.row_bytes(entity) * (1 + probes),
            effects,
            ..Cost::ZERO
        }
    }

    /// Checks that `key` names exactly the entity's primary key fields.
    fn key_map(&mut self, entity: &Entity, key: &BTreeMap<String, Expr>, path: &str) -> Cost {
        let expected: BTreeSet<&str> = entity.key.iter().map(String::as_str).collect();
        let given: BTreeSet<&str> = key.keys().map(String::as_str).collect();
        for missing in expected.difference(&given) {
            self.diag(
                Code::MissingKeyField,
                path,
                format!("missing key field {missing:?} (key: {:?})", entity.key),
            );
        }
        for extra in given.difference(&expected) {
            self.diag(
                Code::UnknownField,
                path,
                format!("{extra:?} isn't a key field (key: {:?})", entity.key),
            );
        }
        let mut cost = Cost::default();
        for (name, value) in key {
            cost += self.expr(value, &format!("{path}.{name}")).1;
        }
        cost
    }

    #[allow(clippy::too_many_lines, reason = "one arm per expression kind")]
    fn expr(&mut self, expr: &Expr, path: &str) -> (Ty, Cost) {
        let own = Cost::ops(1);
        match expr {
            Expr::Lit { value } => (Ty::from_literal(value), own),
            Expr::Decimal { value } => match crate::decimal::Decimal::parse(value) {
                Some(d) => (
                    Ty::Decimal {
                        scale: Some(d.scale()),
                    },
                    own,
                ),
                None => {
                    self.diag(
                        Code::InvalidExpression,
                        path,
                        format!("{value:?} isn't a decimal number"),
                    );
                    (Ty::Unknown, own)
                }
            },
            Expr::Var { id } => match self.lookup(*id) {
                Some(ty) => (ty.clone(), own),
                None => {
                    self.diag(
                        Code::UndefinedVar,
                        path,
                        format!("variable {id} isn't defined here"),
                    );
                    (Ty::Unknown, own)
                }
            },
            Expr::Input { source, name } => {
                let ty = match source {
                    InputSource::Payload => {
                        self.payload.as_ref().and_then(|p| p.get(name)).cloned()
                    }
                    InputSource::Params | InputSource::Query | InputSource::Body => {
                        self.input.and_then(|input| {
                            let fields = match source {
                                InputSource::Params => &input.params,
                                InputSource::Query => &input.query,
                                _ => &input.body,
                            };
                            fields.get(name).map(Ty::from_field)
                        })
                    }
                };
                match ty {
                    Some(ty) => (ty, own),
                    None => {
                        self.diag(
                            Code::UnknownInput,
                            path,
                            format!("unknown {source:?} input {name:?}").to_lowercase(),
                        );
                        (Ty::Unknown, own)
                    }
                }
            }
            Expr::Caller => (Ty::string(CALLER_ID_MAX), own),
            Expr::CallerRole { org } => {
                let (_, cost) = self.expr(org, &format!("{path}.org"));
                let roles = &self.m.auth.roles;
                let ty = if roles.is_empty() {
                    Ty::string(64)
                } else {
                    Ty::Enum(roles.clone())
                };
                (ty.nullable(), own + cost)
            }
            Expr::CallerProfile { field } => {
                let ty = match self.m.auth.profile.get(field) {
                    Some(f) => Ty::from_field(f).nullable(),
                    None => {
                        let known: Vec<&String> = self.m.auth.profile.keys().collect();
                        self.diag(
                            Code::UnknownField,
                            path,
                            format!(
                                "the caller profile has no field {field:?} (declared in auth.profile: {known:?})"
                            ),
                        );
                        Ty::Unknown
                    }
                };
                (ty, own)
            }
            Expr::Now => (Ty::Int { max: None }, own),
            Expr::Field { of, name } => {
                let (of_ty, cost) = self.expr(of, &format!("{path}.of"));
                let nullable = matches!(of_ty, Ty::OptRow(_) | Ty::Opt(_));
                let ty = match of_ty.non_null() {
                    Ty::Row(entity) | Ty::OptRow(entity) => {
                        match self.m.entities.get(entity).map(|e| (e, e.fields.get(name))) {
                            Some((_, Some(field))) => Ty::from_field(field),
                            Some((e, None)) => {
                                self.diags.push(unknown_field(path, name, e));
                                Ty::Unknown
                            }
                            None => Ty::Unknown,
                        }
                    }
                    Ty::Object(fields) => match fields.get(name) {
                        Some(ty) => ty.clone(),
                        None => {
                            let known: Vec<&String> = fields.keys().collect();
                            self.diag(
                                Code::UnknownField,
                                path,
                                format!("object has no field {name:?} (fields: {known:?})"),
                            );
                            Ty::Unknown
                        }
                    },
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        self.diag(
                            Code::InvalidFieldAccess,
                            path,
                            format!("can't read field {name:?} of {}", other.describe()),
                        );
                        Ty::Unknown
                    }
                };
                (if nullable { ty.nullable() } else { ty }, own + cost)
            }
            Expr::IsNull { of } => (Ty::Bool, own + self.expr(of, &format!("{path}.of")).1),
            Expr::Not { arg } => (Ty::Bool, own + self.condition(arg, &format!("{path}.arg"))),
            Expr::Neg { arg } | Expr::Abs { arg } => {
                let (ty, cost) = self.numeric(arg, &format!("{path}.arg"));
                (ty, own + cost)
            }
            Expr::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs, path),
            Expr::In { value, list: _ } => {
                (Ty::Bool, own + self.expr(value, &format!("{path}.value")).1)
            }
            Expr::Select {
                cond,
                then,
                otherwise,
            } => {
                let cond_cost = self.condition(cond, &format!("{path}.cond"));
                let (mut tt, tc) = self.expr(then, &format!("{path}.then"));
                let (mut et, ec) = self.expr(otherwise, &format!("{path}.else"));
                // `x is null ? fallback : x` (and its negation) never yields null from `x`.
                match &**cond {
                    Expr::IsNull { of } if **of == **otherwise => et = et.non_null().clone(),
                    Expr::Not { arg } => {
                        if let Expr::IsNull { of } = &**arg
                            && **of == **then
                        {
                            tt = tt.non_null().clone();
                        }
                    }
                    _ => {}
                }
                (tt.join(et), own + cond_cost + tc.max(ec))
            }
            Expr::Match { on, cases } => self.match_expr(on, cases, path),
            Expr::Object { fields } => {
                let mut cost = own;
                let mut types = BTreeMap::new();
                for (name, value) in fields {
                    let (ty, c) = self.expr(value, &format!("{path}.fields.{name}"));
                    cost += c;
                    types.insert(name.clone(), ty);
                }
                (Ty::Object(types), cost)
            }
            Expr::List { items } => {
                let mut cost = own;
                let mut item_ty: Option<Ty> = None;
                for (i, item) in items.iter().enumerate() {
                    let (ty, c) = self.expr(item, &format!("{path}.items[{i}]"));
                    cost += c;
                    item_ty = Some(match item_ty {
                        Some(prev) => prev.join(ty),
                        None => ty,
                    });
                }
                let ty = Ty::List {
                    item: Box::new(item_ty.unwrap_or(Ty::Unknown)),
                    max: items.len() as u64,
                };
                (ty, cost)
            }
            Expr::First { of } => {
                let (ty, cost) = self.expr(of, &format!("{path}.of"));
                let item = match ty.non_null() {
                    Ty::List { item, .. } => (**item).clone().nullable(),
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        self.type_mismatch(&format!("{path}.of"), "list", other);
                        Ty::Unknown
                    }
                };
                (item, own + cost)
            }
            Expr::Length { of } => {
                let (ty, cost) = self.expr(of, &format!("{path}.of"));
                let max = match ty.non_null() {
                    Ty::List { max, .. } => i64::try_from(*max).ok(),
                    t @ (Ty::String { .. } | Ty::Enum(_)) => {
                        t.max_chars().and_then(|m| i64::try_from(m).ok())
                    }
                    Ty::Unknown => None,
                    other => {
                        self.type_mismatch(&format!("{path}.of"), "list or string", other);
                        None
                    }
                };
                (Ty::Int { max }, own + cost)
            }
            Expr::Str { op, of } => {
                let (ty, cost) = self.string(of, &format!("{path}.of"));
                // Case mapping can turn one character into up to three (`ß` → `SS`).
                let grow = if matches!(op, StrOp::Trim) { 1 } else { 3 };
                let max = ty.max_chars().map(|m| m.saturating_mul(grow));
                (Ty::String { max }, own + cost)
            }
            Expr::Truncate { of, max } => {
                let (ty, cost) = self.string(of, &format!("{path}.of"));
                let max = ty
                    .max_chars()
                    .map_or(u64::from(*max), |m| m.min(u64::from(*max)));
                (Ty::string(max), own + cost)
            }
            Expr::StrTest { of, arg, .. } => {
                let cost = self.string(of, &format!("{path}.of")).1
                    + self.string(arg, &format!("{path}.arg")).1;
                (Ty::Bool, own + cost)
            }
            Expr::Concat { parts } => {
                let mut cost = own;
                let mut max = Some(0_u64);
                for (i, part) in parts.iter().enumerate() {
                    let (ty, c) = self.expr(part, &format!("{path}.parts[{i}]"));
                    cost += c;
                    max = max.zip(ty.max_chars()).map(|(a, b)| a.saturating_add(b));
                }
                (Ty::String { max }, cost)
            }
            Expr::Agg {
                op,
                list,
                item,
                value,
            } => self.aggregate(*op, list, *item, value.as_deref(), path),
            Expr::Sort {
                list,
                item,
                by,
                desc: _,
            } => {
                let (list_ty, list_cost) = self.expr(list, &format!("{path}.list"));
                let Some((item_ty, max)) =
                    self.bounded_list(list_ty.clone(), &format!("{path}.list"))
                else {
                    return (Ty::Unknown, own + list_cost);
                };
                self.scopes.push(HashMap::new());
                self.bind_scoped(*item, item_ty);
                let (by_ty, by_cost) = self.expr(by, &format!("{path}.by"));
                self.scopes.pop();
                self.orderable(&by_ty, &format!("{path}.by"));
                let cost = own
                    + list_cost
                    + by_cost.plus(Cost::ops(1)).times(max)
                    + Cost::ops(crate::runtime::sort_comparisons(max));
                (list_ty.non_null().clone(), cost)
            }
            Expr::Slice { of, start, end } => {
                let (ty, mut cost) = self.expr(of, &format!("{path}.of"));
                cost += self.int(start, &format!("{path}.start"));
                let mut end_bound = None;
                if let Some(end) = end {
                    cost += self.int(end, &format!("{path}.end"));
                    if let Expr::Lit {
                        value: crate::manifest::Literal::Int(n),
                    } = &**end
                        && *n >= 0
                    {
                        end_bound = Some(n.cast_unsigned());
                    }
                }
                let result = match ty.non_null() {
                    Ty::List { item, max } => Ty::List {
                        item: item.clone(),
                        max: end_bound.map_or(*max, |e| e.min(*max)),
                    },
                    t @ (Ty::String { .. } | Ty::Enum(_)) => Ty::String { max: t.max_chars() },
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        self.type_mismatch(&format!("{path}.of"), "list or string", other);
                        Ty::Unknown
                    }
                };
                (result, own + cost)
            }
            Expr::Distinct { of } => {
                let (ty, cost) = self.expr(of, &format!("{path}.of"));
                let Some((item, max)) = self.bounded_list(ty, &format!("{path}.of")) else {
                    return (Ty::Unknown, own + cost);
                };
                (
                    Ty::List {
                        item: Box::new(item),
                        max,
                    },
                    own + cost + Cost::ops(max),
                )
            }
            Expr::ListConcat { parts } => {
                let mut cost = own;
                let mut item_ty: Option<Ty> = None;
                let mut total = 0_u64;
                for (i, part) in parts.iter().enumerate() {
                    let part_path = format!("{path}.parts[{i}]");
                    let (ty, c) = self.expr(part, &part_path);
                    cost += c;
                    if let Some((item, max)) = self.bounded_list(ty, &part_path) {
                        total = total.saturating_add(max);
                        item_ty = Some(match item_ty {
                            Some(prev) => prev.join(item),
                            None => item,
                        });
                    }
                }
                (
                    Ty::List {
                        item: Box::new(item_ty.unwrap_or(Ty::Unknown)),
                        max: total,
                    },
                    cost,
                )
            }
            Expr::Includes { list, value } => {
                let (ty, list_cost) = self.expr(list, &format!("{path}.list"));
                let max = self
                    .bounded_list(ty, &format!("{path}.list"))
                    .map_or(0, |(_, max)| max);
                let (_, value_cost) = self.expr(value, &format!("{path}.value"));
                (Ty::Bool, own + list_cost + value_cost + Cost::ops(max))
            }
            Expr::Split { of, sep, max } => {
                let (ty, cost) = self.string(of, &format!("{path}.of"));
                let sep_cost = self.string(sep, &format!("{path}.sep")).1;
                if !(1..=1000).contains(max) {
                    self.diag(
                        Code::InvalidExpression,
                        &format!("{path}.max"),
                        "split takes 1-1000 parts",
                    );
                }
                let list = Ty::List {
                    item: Box::new(Ty::String {
                        max: ty.max_chars(),
                    }),
                    max: u64::from(*max),
                };
                (list, own + cost + sep_cost)
            }
            Expr::Replace { of, find, with } => {
                let (ty, cost) = self.string(of, &format!("{path}.of"));
                let find_cost = self.string(find, &format!("{path}.find")).1;
                let (with_ty, with_cost) = self.string(with, &format!("{path}.with"));
                // At most one match per character, each replaced by `with`.
                let max = ty
                    .max_chars()
                    .zip(with_ty.max_chars())
                    .map(|(n, w)| n.saturating_mul(w.max(1)));
                (Ty::String { max }, own + cost + find_cost + with_cost)
            }
            Expr::Pad {
                of, length, fill, ..
            } => {
                let (ty, cost) = self.expr(of, &format!("{path}.of"));
                if *length > 1000 || fill.is_empty() {
                    self.diag(
                        Code::InvalidExpression,
                        path,
                        "padding needs a length of at most 1000 and a fill that isn't empty",
                    );
                }
                let max = ty.max_chars().map(|m| m.max(u64::from(*length)));
                (Ty::String { max }, own + cost)
            }
            Expr::Matches { of, pattern } => {
                let cost = self.string(of, &format!("{path}.of")).1;
                if pattern.chars().count() > 256 {
                    self.diag(
                        Code::InvalidExpression,
                        &format!("{path}.pattern"),
                        "patterns are limited to 256 characters",
                    );
                } else if let Err(e) = regex_lite::Regex::new(pattern) {
                    self.diag(
                        Code::InvalidExpression,
                        &format!("{path}.pattern"),
                        format!("invalid pattern: {e}"),
                    );
                }
                (Ty::Bool, own + cost)
            }
            Expr::FormatNumber { of, .. } => {
                let (ty, cost) = self.numeric(of, &format!("{path}.of"));
                (keep_null(&ty, Ty::string(60)), own + cost)
            }
            Expr::Round { of, scale, .. } => {
                let (ty, cost) = self.numeric(of, &format!("{path}.of"));
                self.scale(*scale, &format!("{path}.scale"));
                let result = Ty::Decimal {
                    scale: Some(*scale),
                };
                (keep_null(&ty, result), own + cost)
            }
            Expr::ToInt { of, .. } => {
                let (ty, cost) = self.numeric(of, &format!("{path}.of"));
                (keep_null(&ty, Ty::Int { max: None }), own + cost)
            }
            Expr::StartOf { of, tz, .. } => {
                let cost = self.int(of, &format!("{path}.of")) + self.tz(tz.as_deref(), path);
                (Ty::Int { max: None }, own + cost)
            }
            Expr::AddTime { of, amount, tz, .. } => {
                let cost = self.int(of, &format!("{path}.of"))
                    + self.int(amount, &format!("{path}.amount"))
                    + self.tz(tz.as_deref(), path);
                (Ty::Int { max: None }, own + cost)
            }
            Expr::TimePart { of, part, tz } => {
                let cost = self.int(of, &format!("{path}.of")) + self.tz(tz.as_deref(), path);
                let max = match part {
                    crate::manifest::TimePart::Year => 9999,
                    crate::manifest::TimePart::Month => 12,
                    crate::manifest::TimePart::Day => 31,
                    crate::manifest::TimePart::Hour => 23,
                    crate::manifest::TimePart::Minute | crate::manifest::TimePart::Second => 59,
                    crate::manifest::TimePart::Weekday => 7,
                    crate::manifest::TimePart::DayOfYear => 366,
                };
                (Ty::Int { max: Some(max) }, own + cost)
            }
            Expr::FormatTime { of, pattern, tz } => {
                let cost = self.int(of, &format!("{path}.of")) + self.tz(tz.as_deref(), path);
                let max = match pattern {
                    None => 29,
                    Some(pattern) => {
                        if pattern.chars().count() > 64 {
                            self.diag(
                                Code::InvalidExpression,
                                &format!("{path}.pattern"),
                                "time patterns are limited to 64 characters",
                            );
                        } else if let Err(e) = crate::runtime::valid_time_pattern(pattern) {
                            self.diag(
                                Code::InvalidExpression,
                                &format!("{path}.pattern"),
                                format!("invalid time pattern: {e}"),
                            );
                        }
                        // The longest directive (a time zone name) is under 32 characters.
                        pattern.chars().count() as u64 * 32
                    }
                };
                (Ty::string(max), own + cost)
            }
        }
    }

    fn binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr, path: &str) -> (Ty, Cost) {
        let (lt, lc) = self.expr(lhs, &format!("{path}.lhs"));
        let (rt, rc) = self.expr(rhs, &format!("{path}.rhs"));
        let cost = Cost::ops(1) + lc + rc;
        let ty = match op {
            op if op.is_comparison() => Ty::Bool,
            BinaryOp::And | BinaryOp::Or => {
                for (side, ty) in [("lhs", &lt), ("rhs", &rt)] {
                    if !ty.is_boolish() {
                        self.type_mismatch(&format!("{path}.{side}"), "bool", ty);
                    }
                }
                Ty::Bool
            }
            _ => {
                let int_only = matches!(
                    op,
                    BinaryOp::Mod | BinaryOp::Pow | BinaryOp::DivFloor | BinaryOp::DivCeil
                );
                for (side, ty) in [("lhs", &lt), ("rhs", &rt)] {
                    if int_only && !ty.is_intish() {
                        self.type_mismatch(&format!("{path}.{side}"), "int", ty);
                    } else if !ty.is_numeric() {
                        self.type_mismatch(&format!("{path}.{side}"), "int or decimal", ty);
                    }
                }
                let scale = |ty: &Ty| match ty.non_null() {
                    Ty::Decimal { scale } => Some(*scale),
                    Ty::Int { .. } => Some(Some(0)),
                    _ => None,
                };
                let is_decimal = |ty: &Ty| matches!(ty.non_null(), Ty::Decimal { .. });
                let nullable =
                    matches!(lt, Ty::Null | Ty::Opt(_)) || matches!(rt, Ty::Null | Ty::Opt(_));
                let result = if is_decimal(&lt) || is_decimal(&rt) {
                    let (a, b) = (scale(&lt).flatten(), scale(&rt).flatten());
                    let scale = a.zip(b).map(|(a, b)| match op {
                        BinaryOp::Mul => (a + b).min(crate::decimal::MAX_SCALE),
                        _ => a.max(b),
                    });
                    Ty::Decimal { scale }
                } else {
                    let bound = |ty: &Ty| match ty.non_null() {
                        Ty::Int { max } => *max,
                        _ => None,
                    };
                    let max = match op {
                        // min(a, b) never exceeds either bound.
                        BinaryOp::Min => match (bound(&lt), bound(&rt)) {
                            (Some(a), Some(b)) => Some(a.min(b)),
                            (a, b) => a.or(b),
                        },
                        BinaryOp::Max => bound(&lt).zip(bound(&rt)).map(|(a, b)| a.max(b)),
                        _ => None,
                    };
                    Ty::Int { max }
                };
                // `null` propagates through arithmetic.
                if nullable { result.nullable() } else { result }
            }
        };
        (ty, cost)
    }

    fn aggregate(
        &mut self,
        op: AggOp,
        list: &Expr,
        item: VarId,
        value: Option<&Expr>,
        path: &str,
    ) -> (Ty, Cost) {
        let own = Cost::ops(1);
        let (list_ty, list_cost) = self.expr(list, &format!("{path}.list"));
        let Some((item_ty, max)) = self.bounded_list(list_ty, &format!("{path}.list")) else {
            return (Ty::Unknown, own + list_cost);
        };
        self.scopes.push(HashMap::new());
        self.bind_scoped(item, item_ty.clone());
        let (value_ty, value_cost) = match value {
            Some(value) => self.expr(value, &format!("{path}.value")),
            None => (item_ty, Cost::ZERO),
        };
        self.scopes.pop();
        let value_path = format!("{path}.value");
        let ty = match op {
            AggOp::Some | AggOp::Every | AggOp::Count => {
                if value.is_some() && !value_ty.is_boolish() {
                    self.type_mismatch(&value_path, "bool", &value_ty);
                } else if value.is_none() && !matches!(op, AggOp::Count) && !value_ty.is_boolish() {
                    self.type_mismatch(&value_path, "a list of bools", &value_ty);
                }
                if matches!(op, AggOp::Count) {
                    Ty::Int {
                        max: i64::try_from(max).ok(),
                    }
                } else {
                    Ty::Bool
                }
            }
            AggOp::Sum => {
                if !value_ty.is_numeric() {
                    self.type_mismatch(&value_path, "int or decimal", &value_ty);
                }
                match value_ty.non_null() {
                    Ty::Decimal { scale } => Ty::Decimal { scale: *scale },
                    _ => Ty::Int { max: None },
                }
            }
            AggOp::Min | AggOp::Max => {
                self.orderable(&value_ty, &value_path);
                value_ty.non_null().clone().nullable()
            }
        };
        (
            ty,
            own + list_cost + value_cost.plus(Cost::ops(1)).times(max),
        )
    }

    /// Checks a numeric operand and returns its type (ints and decimals keep their kind, and a
    /// value that may be `null` stays nullable).
    fn numeric(&mut self, expr: &Expr, path: &str) -> (Ty, Cost) {
        let (ty, cost) = self.expr(expr, path);
        if !ty.is_numeric() {
            self.type_mismatch(path, "int or decimal", &ty);
        }
        let numeric = match ty.non_null() {
            Ty::Decimal { scale } => Ty::Decimal { scale: *scale },
            _ => Ty::Int { max: None },
        };
        (keep_null(&ty, numeric), cost)
    }

    fn int(&mut self, expr: &Expr, path: &str) -> Cost {
        let (ty, cost) = self.expr(expr, path);
        if !ty.is_intish() && ty != Ty::Null {
            self.type_mismatch(path, "int", &ty);
        }
        cost
    }

    fn tz(&mut self, tz: Option<&Expr>, path: &str) -> Cost {
        let Some(tz) = tz else {
            return Cost::ZERO;
        };
        let tz_path = format!("{path}.tz");
        if let Expr::Lit {
            value: crate::manifest::Literal::String(name),
        } = tz
            && !crate::runtime::time_zone_exists(name)
        {
            self.diag(
                Code::InvalidExpression,
                &tz_path,
                format!("{name:?} isn't an IANA time zone"),
            );
        }
        self.string(tz, &tz_path).1
    }

    fn scale(&mut self, scale: u8, path: &str) {
        if scale > crate::decimal::MAX_SCALE {
            self.diag(
                Code::InvalidExpression,
                path,
                format!("decimal scales are 0-{}", crate::decimal::MAX_SCALE),
            );
        }
    }

    /// Checks that values of a type can be ordered (sort keys, min, max).
    fn orderable(&mut self, ty: &Ty, path: &str) {
        if matches!(
            ty.non_null(),
            Ty::List { .. } | Ty::Object(_) | Ty::Row(_) | Ty::OptRow(_)
        ) {
            self.type_mismatch(path, "a string, int, decimal, or bool", ty);
        }
    }

    /// The declared bucket, or a diagnostic.
    fn bucket(&mut self, bucket: &str, path: &str) -> Option<&'a BucketDecl> {
        let decl = self.m.buckets.get(bucket);
        if decl.is_none() {
            self.unknown(
                Code::UnknownBucket,
                &format!("{path}.bucket"),
                "bucket",
                bucket,
                self.m.buckets.keys(),
            );
        }
        decl
    }

    fn expires(&mut self, expires_s: Option<u32>, path: &str) {
        if let Some(seconds) = expires_s
            && !(1..=BucketDecl::MAX_EXPIRES_S).contains(&seconds)
        {
            self.diag(
                Code::InvalidExpression,
                &format!("{path}.expiresS"),
                format!(
                    "a signed URL lasts 1 second to 7 days ({} seconds), got {seconds}",
                    BucketDecl::MAX_EXPIRES_S
                ),
            );
        }
    }

    fn string(&mut self, expr: &Expr, path: &str) -> (Ty, Cost) {
        let (ty, cost) = self.expr(expr, path);
        if !ty.is_stringish() {
            self.type_mismatch(path, "string", &ty);
        }
        (ty.non_null().clone(), cost)
    }

    fn match_expr(&mut self, on: &Expr, cases: &BTreeMap<String, Expr>, path: &str) -> (Ty, Cost) {
        let (on_ty, on_cost) = self.expr(on, &format!("{path}.on"));
        let has_default = cases.contains_key(DEFAULT_CASE);
        match on_ty.non_null() {
            Ty::Enum(values) => {
                for case in cases.keys().filter(|c| *c != DEFAULT_CASE) {
                    if !values.contains(case) {
                        self.diag(
                            Code::UnknownMatchCase,
                            &format!("{path}.cases.{case}"),
                            format!("{case:?} isn't one of {values:?}"),
                        );
                    }
                }
                let missing: Vec<&String> =
                    values.iter().filter(|v| !cases.contains_key(*v)).collect();
                if !missing.is_empty() && !has_default {
                    self.diag(Code::NonExhaustiveMatch, path, format!("match doesn't handle {missing:?}; add those cases or a {DEFAULT_CASE:?} case"));
                }
            }
            _ if !has_default => {
                self.diag(
                    Code::NonExhaustiveMatch,
                    path,
                    format!(
                        "matching on {} needs a {DEFAULT_CASE:?} case",
                        on_ty.describe()
                    ),
                );
            }
            _ => {}
        }
        let mut result: Option<Ty> = None;
        let mut worst = Cost::default();
        for (case, value) in cases {
            let (ty, cost) = self.expr(value, &format!("{path}.cases.{case}"));
            worst = worst.max(cost);
            result = Some(match result {
                Some(prev) => prev.join(ty),
                None => ty,
            });
        }
        (
            result.unwrap_or(Ty::Unknown),
            Cost::ops(1) + on_cost + worst,
        )
    }
}

/// The type with every integer's upper bound dropped. A loop's state is typed this way, so a
/// counter that grows by one each pass keeps the same type.
fn widen_ints(ty: Ty) -> Ty {
    match ty {
        Ty::Int { .. } => Ty::Int { max: None },
        Ty::Opt(inner) => Ty::Opt(Box::new(widen_ints(*inner))),
        Ty::List { item, max } => Ty::List {
            item: Box::new(widen_ints(*item)),
            max,
        },
        Ty::Object(fields) => Ty::Object(
            fields
                .into_iter()
                .map(|(name, ty)| (name, widen_ints(ty)))
                .collect(),
        ),
        other => other,
    }
}

/// A stored file's metadata, as plans and `upload` handlers see it.
pub(crate) fn blob_info_fields() -> BTreeMap<String, Ty> {
    BTreeMap::from([
        ("key".into(), Ty::string(u64::from(BucketDecl::MAX_KEY_LEN))),
        (
            "size".into(),
            Ty::Int {
                max: i64::try_from(BucketDecl::MAX_BYTES).ok(),
            },
        ),
        (
            "contentType".into(),
            Ty::string(u64::from(BucketDecl::MAX_CONTENT_TYPE_LEN)),
        ),
        ("uploadedAt".into(), Ty::Int { max: None }),
    ])
}
