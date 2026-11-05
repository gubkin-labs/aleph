//! Plan checking: scoping, types, and worst-case cost.
//!
//! Every operation has a known cost and every list has a known maximum length, so the worst case
//! is a fold over the plan tree: sequences add, branches take the maximum, and loops multiply by
//! their bound. There's no recursion or unbounded looping in the format, so this always
//! terminates.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::{EntityInfo, Ty, unknown_entity, unknown_field};
use crate::diagnostics::{Diagnostic, DiagnosticCode as Code};
use crate::limits::Cost;
use crate::manifest::{
    BinaryOp, Entity, Expr, InputSource, Manifest, Op, Route, RowPredicate, Stmt, VarId,
};

/// The match case that catches every other value.
pub(crate) const DEFAULT_CASE: &str = "_";

/// Checks a route's plan and returns its worst-case cost.
pub(crate) fn check_route(
    m: &Manifest,
    infos: &BTreeMap<String, EntityInfo>,
    route: &Route,
    path: &str,
    diags: &mut Vec<Diagnostic>,
) -> Cost {
    let mut checker = Checker::new(m, infos, Some(route), diags);
    let plan_path = format!("{path}.plan");
    checker.scopes.push(HashMap::new());
    let mut cost = Cost::default();
    for (i, stmt) in route.plan.body.iter().enumerate() {
        cost += checker.stmt(stmt, &format!("{plan_path}.body[{i}]"), false);
    }
    let (_, output_cost) = checker.expr(&route.plan.output, &format!("{plan_path}.output"));
    cost + output_cost
}

/// Checks a counter's `where` predicate: a pure boolean expression over one row.
pub(crate) fn check_row_predicate(
    m: &Manifest,
    entity: &str,
    predicate: &RowPredicate,
    path: &str,
    diags: &mut Vec<Diagnostic>,
) {
    let infos = BTreeMap::new();
    let mut checker = Checker::new(m, &infos, None, diags);
    checker.scopes.push(HashMap::new());
    checker.bind(predicate.var, Ty::Row(entity.to_owned()), path);
    let expr_path = format!("{path}.expr");
    if let Some(problem) = impurity(&predicate.expr) {
        checker.diag(
            Code::InvalidPredicate,
            &expr_path,
            format!("a counter predicate must depend only on the row, but it uses {problem}"),
        );
    }
    let (ty, _) = checker.expr(&predicate.expr, &expr_path);
    if !ty.is_boolish() {
        checker.type_mismatch(&expr_path, "bool", &ty);
    }
}

/// Returns what makes an expression depend on more than its variables, if anything.
fn impurity(expr: &Expr) -> Option<&'static str> {
    match expr {
        Expr::Input { .. } => Some("a route input"),
        Expr::Caller => Some("the caller"),
        Expr::Now => Some("the current time"),
        Expr::Lit { .. } | Expr::Var { .. } => None,
        Expr::Field { of, .. }
        | Expr::IsNull { of }
        | Expr::Length { of }
        | Expr::Not { arg: of }
        | Expr::Neg { arg: of }
        | Expr::In { value: of, .. } => impurity(of),
        Expr::Binary { lhs, rhs, .. } => impurity(lhs).or_else(|| impurity(rhs)),
        Expr::Select {
            cond,
            then,
            otherwise,
        } => impurity(cond)
            .or_else(|| impurity(then))
            .or_else(|| impurity(otherwise)),
        Expr::Match { on, cases } => impurity(on).or_else(|| cases.values().find_map(impurity)),
        Expr::Object { fields } => fields.values().find_map(impurity),
        Expr::List { items: parts } | Expr::Concat { parts } => parts.iter().find_map(impurity),
    }
}

struct Checker<'a> {
    m: &'a Manifest,
    infos: &'a BTreeMap<String, EntityInfo>,
    route: Option<&'a Route>,
    diags: &'a mut Vec<Diagnostic>,
    scopes: Vec<HashMap<VarId, Ty>>,
    /// Every variable defined anywhere in the plan. Ids are unique per plan, even across
    /// branches, so a plan never shadows.
    defined: HashSet<VarId>,
}

impl<'a> Checker<'a> {
    fn new(
        m: &'a Manifest,
        infos: &'a BTreeMap<String, EntityInfo>,
        route: Option<&'a Route>,
        diags: &'a mut Vec<Diagnostic>,
    ) -> Self {
        Self {
            m,
            infos,
            route,
            diags,
            scopes: Vec::new(),
            defined: HashSet::new(),
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

    fn stmt(&mut self, stmt: &Stmt, path: &str, pure: bool) -> Cost {
        let own = Cost::ops(1);
        match stmt {
            Stmt::Let { var, op } => {
                let (ty, cost) = self.op(op, &format!("{path}.op"), pure);
                self.bind(*var, ty, path);
                own + cost
            }
            Stmt::Ensure { cond, error } => {
                self.error_ref(error, &format!("{path}.error"));
                own + self.condition(cond, &format!("{path}.cond"))
            }
            Stmt::Fail { error } => {
                self.error_ref(error, &format!("{path}.error"));
                own
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
                self.bind(*item, item_ty, path);
                let body_cost = self.block(body, &format!("{path}.body"), pure);
                self.scopes.pop();
                own + list_cost + body_cost.plus(Cost::ops(1)).times(max)
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

    fn error_ref(&mut self, name: &str, path: &str) {
        let declared = self.route.is_some_and(|r| r.errors.contains_key(name));
        if !declared {
            let known: Vec<&str> = self
                .route
                .map(|r| r.errors.keys().map(String::as_str).collect())
                .unwrap_or_default();
            self.diag(
                Code::UnknownError,
                path,
                format!("unknown error {name:?} (declared: {known:?})"),
            );
        }
    }

    /// Resolves a list type to its item type and maximum length.
    fn bounded_list(&mut self, ty: Ty, path: &str) -> Option<(Ty, u64)> {
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
        if pure && op.is_write() {
            self.diag(
                Code::WriteInPureBlock,
                path,
                "map and filter bodies may read but never write, so writes can't hide inside a transform. \
                 Compute the values with map, then write them with forEach.",
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
                let (ty, cost) = self.insert(entity, values, path);
                (ty, own + cost)
            }
            Op::Update { entity, key, set } => {
                let (ty, cost) = self.update(entity, key, set, path);
                (ty, own + cost)
            }
            Op::Delete { entity, key } => {
                let Some(e) = self.entity(entity, &format!("{path}.entity")) else {
                    return (Ty::Unknown, own);
                };
                let cost =
                    own + self.key_map(e, key, &format!("{path}.key")) + self.write_cost(entity, 1);
                (Ty::Null, cost)
            }
            Op::Query {
                entity,
                index,
                prefix,
                limit,
                order: _,
            } => {
                let (ty, cost) = self.query(entity, index.as_deref(), prefix, limit, path);
                (ty, own + cost)
            }
            Op::CounterGet { counter, key } => (
                Ty::Int { max: None },
                own + self.counter_get(counter, key, path),
            ),
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
                self.bind(*item, item_ty, path);
                let body_cost = self.stmts(body, &format!("{path}.body"), true);
                let (result_ty, result_cost) = self.expr(result, &format!("{path}.yield"));
                self.scopes.pop();
                let per_item = body_cost + result_cost + Cost::ops(1);
                (
                    Ty::List {
                        item: Box::new(result_ty),
                        max,
                    },
                    own + list_cost + per_item.times(max),
                )
            }
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
                self.bind(*item, item_ty.clone(), path);
                let body_cost = self.stmts(body, &format!("{path}.body"), true);
                let keep_cost = self.condition(keep, &format!("{path}.keep"));
                self.scopes.pop();
                let per_item = body_cost + keep_cost + Cost::ops(1);
                (
                    Ty::List {
                        item: Box::new(item_ty),
                        max,
                    },
                    own + list_cost + per_item.times(max),
                )
            }
            Op::Eval { expr } => {
                let (ty, cost) = self.expr(expr, &format!("{path}.expr"));
                (ty, own + cost)
            }
        }
    }

    fn insert(&mut self, entity: &str, values: &BTreeMap<String, Expr>, path: &str) -> (Ty, Cost) {
        let Some(e) = self.entity(entity, &format!("{path}.entity")) else {
            return (Ty::Unknown, Cost::default());
        };
        let mut cost = self.write_cost(entity, 1);
        for (name, value) in values {
            let value_path = format!("{path}.values.{name}");
            if !e.fields.contains_key(name) {
                self.diags.push(unknown_field(&value_path, name, e));
            }
            cost += self.expr(value, &value_path).1;
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
        let mut cost = self.key_map(e, key, &format!("{path}.key")) + self.write_cost(entity, 2);
        for (name, value) in set {
            let value_path = format!("{path}.set.{name}");
            if !e.fields.contains_key(name) {
                self.diags.push(unknown_field(&value_path, name, e));
            } else if e.key.contains(name) {
                self.diag(
                    Code::ImmutableKeyField,
                    &value_path,
                    format!("key field {name:?} can't be updated"),
                );
            }
            cost += self.expr(value, &value_path).1;
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
        let mut cost = Cost {
            reads: 1,
            bytes_read: 20,
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

    fn row_bytes(&self, entity: &str) -> u64 {
        self.infos.get(entity).map_or(0, |info| info.row_bytes)
    }

    /// Worst-case cost of one row write. Every write first reads the row it replaces (to keep
    /// indexes and counters consistent), and `per_index` is 2 for updates, which may move an index
    /// entry (delete + insert) or move a row between counter groups.
    fn write_cost(&self, entity: &str, per_index: u64) -> Cost {
        let writes = self.infos.get(entity).map_or(1, |info| {
            1 + per_index * info.index_count + per_index * info.counter_count
        });
        Cost {
            reads: 1,
            writes,
            bytes_read: self.row_bytes(entity),
            ops: 0,
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

    #[allow(clippy::too_many_lines)]
    fn expr(&mut self, expr: &Expr, path: &str) -> (Ty, Cost) {
        let own = Cost::ops(1);
        match expr {
            Expr::Lit { value } => (Ty::from_literal(value), own),
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
                let fields = self.route.map(|r| match source {
                    InputSource::Params => &r.input.params,
                    InputSource::Query => &r.input.query,
                    InputSource::Body => &r.input.body,
                });
                match fields.and_then(|f| f.get(name)) {
                    Some(field) => (Ty::from_field_type(&field.ty), own),
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
            Expr::Caller => (Ty::String, own),
            Expr::Now => (Ty::Int { max: None }, own),
            Expr::Field { of, name } => {
                let (of_ty, cost) = self.expr(of, &format!("{path}.of"));
                let ty = match &of_ty {
                    Ty::Row(entity) | Ty::OptRow(entity) => {
                        match self.m.entities.get(entity).map(|e| (e, e.fields.get(name))) {
                            Some((_, Some(field))) => Ty::from_field_type(&field.ty),
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
                (ty, own + cost)
            }
            Expr::IsNull { of } => (Ty::Bool, own + self.expr(of, &format!("{path}.of")).1),
            Expr::Not { arg } => (Ty::Bool, own + self.condition(arg, &format!("{path}.arg"))),
            Expr::Neg { arg } => {
                let (ty, cost) = self.expr(arg, &format!("{path}.arg"));
                if !ty.is_intish() {
                    self.type_mismatch(&format!("{path}.arg"), "int", &ty);
                }
                (Ty::Int { max: None }, own + cost)
            }
            Expr::Binary { op, lhs, rhs } => {
                let (lt, lc) = self.expr(lhs, &format!("{path}.lhs"));
                let (rt, rc) = self.expr(rhs, &format!("{path}.rhs"));
                let cost = own + lc + rc;
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
                        for (side, ty) in [("lhs", &lt), ("rhs", &rt)] {
                            if !ty.is_intish() {
                                self.type_mismatch(&format!("{path}.{side}"), "int", ty);
                            }
                        }
                        let bound = |ty: &Ty| match ty {
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
                    }
                };
                (ty, cost)
            }
            Expr::In { value, list: _ } => {
                (Ty::Bool, own + self.expr(value, &format!("{path}.value")).1)
            }
            Expr::Select {
                cond,
                then,
                otherwise,
            } => {
                let cond_cost = self.condition(cond, &format!("{path}.cond"));
                let (tt, tc) = self.expr(then, &format!("{path}.then"));
                let (et, ec) = self.expr(otherwise, &format!("{path}.else"));
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
            Expr::Length { of } => {
                let (ty, cost) = self.expr(of, &format!("{path}.of"));
                let max = match ty {
                    Ty::List { max, .. } => i64::try_from(max).ok(),
                    Ty::String | Ty::Enum(_) | Ty::Unknown => None,
                    other => {
                        self.type_mismatch(&format!("{path}.of"), "list or string", &other);
                        None
                    }
                };
                (Ty::Int { max }, own + cost)
            }
            Expr::Concat { parts } => {
                let mut cost = own;
                for (i, part) in parts.iter().enumerate() {
                    cost += self.expr(part, &format!("{path}.parts[{i}]")).1;
                }
                (Ty::String, cost)
            }
        }
    }

    fn match_expr(&mut self, on: &Expr, cases: &BTreeMap<String, Expr>, path: &str) -> (Ty, Cost) {
        let (on_ty, on_cost) = self.expr(on, &format!("{path}.on"));
        let has_default = cases.contains_key(DEFAULT_CASE);
        match &on_ty {
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
