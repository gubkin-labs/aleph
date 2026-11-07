//! Targeted checks of the usage analysis and the interpreter's charges, on small hand-built
//! manifests. Each test pins one rule: what the checker accepts or rejects, the exact worst case
//! it computes, or (for plans without branches over max-size data) that the runtime charges
//! exactly that worst case. Written to kill the mutants `cargo mutants -p aleph-core` found
//! surviving in `check/plan.rs` and `runtime/exec.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    missing_docs
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use aleph_core::manifest::Method;
use aleph_core::{
    Caller, Cost, Engine, FixedEnv, Limits, MemoryStore, Request, Response, TenantId,
    parse_and_validate,
};
use serde_json::{Value as Json, json};

fn lit(value: Json) -> Json {
    json!({ "kind": "lit", "value": value })
}

fn var(id: u32) -> Json {
    json!({ "kind": "var", "id": id })
}

fn body(name: &str) -> Json {
    json!({ "kind": "input", "source": "body", "name": name })
}

fn field(of: Json, name: &str) -> Json {
    json!({ "kind": "field", "of": of, "name": name })
}

fn let_(id: u32, op: Json) -> Json {
    json!({ "kind": "let", "var": id, "op": op })
}

/// A manifest with one entity, `Item` (key `id`; an int `n` of 0-100, a 1-20 character `name`,
/// an optional `tag`, an optional list `xs` of up to 3 ints, and an optional object `meta` with a
/// required int `a` and an optional string `b`), and one public route `POST /r` taking
/// `n`, `name`, and `xs` (up to 5 ints) in its body, whose plan is `body` then `output`.
fn manifest(plan_body: Vec<Json>, output: Json) -> Json {
    json!({
        "formatVersion": "0.2",
        "app": { "name": "t" },
        "auth": { "apiKeyScopes": [], "roles": [] },
        "counters": {},
        "entities": {
            "Item": {
                "key": ["id"],
                "fields": {
                    "id": { "type": { "kind": "string", "min": 1, "max": 20 } },
                    "n": { "type": { "kind": "int", "min": 0, "max": 100 } },
                    "name": { "type": { "kind": "string", "min": 1, "max": 20 } },
                    "tag": { "type": { "kind": "string", "min": 0, "max": 10 }, "optional": true },
                    "xs": { "type": { "kind": "array", "items": { "kind": "int", "min": 0, "max": 9 }, "max": 3 }, "optional": true },
                    "meta": {
                        "type": { "kind": "object", "fields": {
                            "a": { "type": { "kind": "int" } },
                            "b": { "type": { "kind": "string", "max": 5 }, "optional": true },
                        } },
                        "optional": true,
                    },
                },
                "indexes": {},
            }
        },
        "routes": [{
            "id": "r",
            "method": "POST",
            "path": "/r",
            "auth": { "kind": "public" },
            "errors": { "missing": { "status": 404, "message": "not found" } },
            "input": {
                "body": {
                    "n": { "type": { "kind": "int", "min": 0, "max": 100 } },
                    "name": { "type": { "kind": "string", "min": 1, "max": 20 } },
                    "xs": { "type": { "kind": "array", "items": { "kind": "int", "min": 0, "max": 9 }, "max": 5 }, "optional": true },
                },
                "params": {},
                "query": {},
            },
            "plan": { "body": plan_body, "output": output },
        }],
    })
}

/// The diagnostic codes validating `m` produces (empty if it validates).
fn codes(m: &Json) -> BTreeSet<String> {
    match parse_and_validate(&m.to_string(), &Limits::ROUTE) {
        Ok(_) => BTreeSet::new(),
        Err(diags) => diags
            .iter()
            .map(|d| {
                serde_json::to_value(d.code)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect(),
    }
}

fn bounds(m: &Json) -> Cost {
    let valid =
        parse_and_validate(&m.to_string(), &Limits::ROUTE).unwrap_or_else(|d| panic!("{d:#?}"));
    valid.report().routes["r"].bounds
}

fn run(m: &Json, request_body: Json) -> Response {
    let valid =
        parse_and_validate(&m.to_string(), &Limits::ROUTE).unwrap_or_else(|d| panic!("{d:#?}"));
    let engine = Engine::new(
        valid,
        Arc::new(MemoryStore::new()),
        Arc::new(FixedEnv::new(1, 1)),
    );
    engine.handle(
        &TenantId::new("t").unwrap(),
        &Request {
            method: Method::Post,
            path: "/r".into(),
            query: BTreeMap::new(),
            body: Some(request_body),
            caller: Some(Caller::user("u1")),
            ip: None,
            idempotency_key: None,
            headers: BTreeMap::new(),
            raw_body: None,
        },
    )
}

fn obj(fields: Json) -> Json {
    json!({ "kind": "object", "fields": fields })
}

fn list(items: Json) -> Json {
    json!({ "kind": "list", "items": items })
}

fn insert(values: Json) -> Json {
    json!({ "op": "insert", "entity": "Item", "values": values })
}

fn get(id: Json) -> Json {
    json!({ "op": "get", "entity": "Item", "key": { "id": id } })
}

fn ensure(cond: Json) -> Json {
    json!({ "kind": "ensure", "cond": cond, "error": "missing" })
}

fn not_null(of: Json) -> Json {
    json!({ "kind": "not", "arg": { "kind": "isNull", "of": of } })
}

fn and(lhs: Json, rhs: Json) -> Json {
    json!({ "kind": "binary", "op": "and", "lhs": lhs, "rhs": rhs })
}

fn ok() -> BTreeSet<String> {
    BTreeSet::new()
}

fn only(code: &str) -> BTreeSet<String> {
    BTreeSet::from([code.to_owned()])
}

/// The base values for an `Item` insert, with `extra` merged in.
fn item(extra: Json) -> Json {
    let mut values = json!({ "id": lit(json!("i1")), "n": body("n"), "name": body("name") });
    for (k, v) in extra.as_object().unwrap() {
        values[k] = v.clone();
    }
    values
}

#[test]
fn the_harness_manifest_validates() {
    let m = manifest(vec![let_(0, insert(item(json!({}))))], lit(json!(1)));
    assert_eq!(codes(&m), ok());
    let r = run(&m, json!({ "n": 1, "name": "a" }));
    assert_eq!(r.status, 200, "{:?}", r.body);
}

#[test]
fn nested_object_writes_need_their_required_fields() {
    let write = |meta: Json| {
        codes(&manifest(
            vec![let_(0, insert(item(json!({ "meta": meta }))))],
            lit(json!(1)),
        ))
    };
    assert_eq!(write(obj(json!({ "a": lit(json!(1)) }))), ok());
    assert_eq!(
        write(obj(json!({ "a": lit(json!(1)), "b": lit(json!("x")) }))),
        ok()
    );
    assert_eq!(
        write(obj(json!({ "b": lit(json!("x")) }))),
        only("missing_required_field")
    );
    assert_eq!(
        write(obj(json!({ "a": lit(json!("not an int")) }))),
        only("type_mismatch")
    );
}

#[test]
fn list_writes_check_every_item() {
    let write = |xs: Json| {
        codes(&manifest(
            vec![let_(0, insert(item(json!({ "xs": xs }))))],
            lit(json!(1)),
        ))
    };
    assert_eq!(write(list(json!([lit(json!(1)), lit(json!(2))]))), ok());
    assert_eq!(
        write(list(json!([lit(json!("one"))]))),
        only("type_mismatch")
    );
}

#[test]
fn ensure_with_and_narrows_both_sides() {
    // After `ensure(found != null && true)`, the row is known to exist, so its fields aren't
    // nullable and can be written to required fields.
    let plan = |cond: Json| {
        manifest(
            vec![
                let_(0, get(lit(json!("i0")))),
                ensure(cond),
                let_(1, insert(item(json!({ "name": field(var(0), "name") })))),
            ],
            lit(json!(1)),
        )
    };
    assert_eq!(codes(&plan(not_null(var(0)))), ok());
    assert_eq!(codes(&plan(and(not_null(var(0)), lit(json!(true))))), ok());
    assert_eq!(codes(&plan(and(lit(json!(true)), not_null(var(0))))), ok());
    assert_eq!(codes(&plan(lit(json!(true)))), only("type_mismatch"));
}

fn handler(id: &str, event: &str, body: Vec<Json>) -> Json {
    json!({
        "id": id,
        "trigger": { "kind": "event", "event": event },
        "errors": {},
        "plan": { "body": body, "output": lit(Json::Null) },
    })
}

fn emit(event: &str) -> Json {
    json!({ "op": "emit", "event": event, "payload": {} })
}

#[test]
fn an_emit_charges_one_effect_per_handler_of_its_event() {
    let mut m = manifest(vec![let_(0, emit("ping"))], lit(json!(1)));
    m["events"] = json!({ "ping": { "payload": {} }, "pong": { "payload": {} } });
    m["handlers"] = json!([
        handler("a", "ping", vec![]),
        handler("b", "ping", vec![]),
        handler("c", "pong", vec![]),
    ]);
    assert_eq!(bounds(&m).effects, 2);
    let r = run(&m, json!({ "n": 1, "name": "a" }));
    assert_eq!((r.status, r.usage.effects), (200, 2), "{:?}", r.body);
}

#[test]
fn row_expressions_must_have_the_right_type() {
    let n = || field(var(0), "n");
    let row = |expr: Json| json!({ "var": 0, "expr": expr });
    let with_counter = |counter: Json| {
        let mut m = manifest(vec![], lit(json!(1)));
        m["counters"] = json!({ "c": counter });
        codes(&m)
    };
    let gt0 = json!({ "kind": "binary", "op": "gt", "lhs": n(), "rhs": lit(json!(0)) });
    assert_eq!(
        with_counter(json!({ "of": "Item", "groupBy": [], "where": row(gt0) })),
        ok()
    );
    assert_eq!(
        with_counter(json!({ "of": "Item", "groupBy": [], "where": row(n()) })),
        only("type_mismatch"),
        "a where filter must be a bool"
    );
    assert_eq!(
        with_counter(json!({ "of": "Item", "groupBy": [], "sum": row(n()) })),
        ok()
    );
    assert_eq!(
        with_counter(json!({ "of": "Item", "groupBy": [], "sum": row(lit(json!(true))) })),
        only("type_mismatch"),
        "a sum must be an int"
    );

    let with_index = |expr: Json| {
        let mut m = manifest(vec![], lit(json!(1)));
        m["entities"]["Item"]["indexes"] =
            json!({ "byX": { "fields": [{ "name": "x", "var": 0, "expr": expr }] } });
        codes(&m)
    };
    assert_eq!(with_index(n()), ok());
    assert_eq!(
        with_index(var(0)),
        only("type_mismatch"),
        "an index part must be a scalar"
    );
    assert_eq!(with_index(list(json!([n()]))), only("type_mismatch"));
}

#[test]
fn response_statements_only_shape_a_route_response() {
    let status = json!({ "kind": "setStatus", "status": 201 });
    let header = json!({ "kind": "setHeader", "name": "x-a", "value": lit(json!("b")) });
    for stmt in [status, header] {
        assert_eq!(
            codes(&manifest(vec![stmt.clone()], lit(json!(1)))),
            ok(),
            "{stmt}"
        );

        let in_map = json!({
            "op": "map", "list": list(json!([lit(json!(1))])), "item": 1,
            "body": [stmt.clone()], "yield": lit(json!(1)),
        });
        assert_eq!(
            codes(&manifest(vec![let_(0, in_map)], lit(json!(1)))),
            only("invalid_route"),
            "{stmt} in map"
        );

        let mut m = manifest(vec![], lit(json!(1)));
        m["events"] = json!({ "ping": { "payload": {} } });
        m["handlers"] = json!([handler("h", "ping", vec![stmt.clone()])]);
        assert_eq!(codes(&m), only("invalid_route"), "{stmt} in a handler");
    }
}

#[test]
fn workflow_steps_belong_to_their_own_workflow() {
    let flows = |next: &str, start2: &str| {
        let mut m = manifest(vec![], lit(json!(1)));
        m["workflows"] = json!({
            "w1": { "input": {}, "start": "a" },
            "w2": { "input": {}, "start": start2 },
        });
        let step = |id: &str, workflow: &str, step: &str, body: Vec<Json>| {
            json!({
                "id": id,
                "trigger": { "kind": "workflow", "workflow": workflow, "step": step },
                "errors": {},
                "plan": { "body": body, "output": lit(Json::Null) },
            })
        };
        m["handlers"] = json!([
            step(
                "w1_a",
                "w1",
                "a",
                vec![json!({ "kind": "next", "step": next })]
            ),
            step("w1_c", "w1", "c", vec![]),
            step("w2_b", "w2", "b", vec![]),
        ]);
        codes(&m)
    };
    assert_eq!(flows("c", "b"), ok());
    assert_eq!(
        flows("b", "b"),
        only("invalid_handler"),
        "b is w2's step, not w1's"
    );
    assert_eq!(
        flows("c", "a"),
        only("invalid_declaration"),
        "w2 has no step a"
    );
}

#[test]
fn a_mistyped_write_names_the_field_kind_it_needs() {
    let m = manifest(
        vec![let_(0, insert(item(json!({ "n": lit(json!("five")) }))))],
        lit(json!(1)),
    );
    let diags = parse_and_validate(&m.to_string(), &Limits::ROUTE).unwrap_err();
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert!(
        diags[0].message.contains("a value for a int field"),
        "{}",
        diags[0].message
    );
}

/// Straight-line plans (no branches, loops over full lists, rows that exist) whose worst case is
/// what actually happens: for these, the runtime must charge exactly the static bounds on every
/// counted resource. A checker that overcounts or a meter that undercharges both break it.
#[test]
#[allow(clippy::too_many_lines, reason = "one entry per plan shape")]
fn straight_line_plans_use_exactly_their_bounds() {
    let n = || body("n");
    let name = || body("name");
    let bin = |op: &str, lhs: Json, rhs: Json| json!({ "kind": "binary", "op": op, "lhs": lhs, "rhs": rhs });
    let seeded = || let_(9, insert(item(json!({ "id": lit(json!("i0")) }))));
    let three = || list(json!([lit(json!(1)), lit(json!(2)), n()]));
    let exprs = [
        ("lit", lit(json!(1))),
        ("input", n()),
        ("add", bin("add", n(), lit(json!(2)))),
        (
            "nested",
            bin(
                "mul",
                bin("add", n(), lit(json!(2))),
                bin("sub", n(), lit(json!(1))),
            ),
        ),
        ("eq", bin("eq", name(), lit(json!("a")))),
        (
            "and",
            bin("and", lit(json!(true)), bin("gt", n(), lit(json!(0)))),
        ),
        ("not", json!({ "kind": "not", "arg": lit(json!(false)) })),
        ("neg", json!({ "kind": "neg", "arg": n() })),
        ("isNull", json!({ "kind": "isNull", "of": n() })),
        (
            "select",
            json!({ "kind": "select", "cond": lit(json!(true)), "then": n(), "else": n() }),
        ),
        ("object", obj(json!({ "a": n(), "b": name() }))),
        ("list", list(json!([n(), n(), n()]))),
        ("length", json!({ "kind": "length", "of": name() })),
        (
            "lower",
            json!({ "kind": "str", "op": "lower", "of": name() }),
        ),
        (
            "concat",
            json!({ "kind": "concat", "parts": [name(), lit(json!("-")), n()] }),
        ),
        ("abs", json!({ "kind": "abs", "arg": n() })),
        (
            "truncate",
            json!({ "kind": "truncate", "of": name(), "max": 3 }),
        ),
        (
            "startsWith",
            json!({ "kind": "strTest", "op": "startsWith", "of": name(), "arg": lit(json!("a")) }),
        ),
        (
            "first",
            json!({ "kind": "first", "of": list(json!([n()])) }),
        ),
        ("in", json!({ "kind": "in", "value": n(), "list": [1, 5] })),
        ("now", json!({ "kind": "now" })),
    ];
    let mut plans: Vec<(String, Vec<Json>, Json)> = exprs
        .into_iter()
        .map(|(label, expr)| (label.to_owned(), vec![], expr))
        .collect();
    plans.extend([
        ("insert".to_owned(), vec![let_(0, insert(item(json!({}))))], var(0)),
        ("get".to_owned(), vec![seeded(), let_(0, get(lit(json!("i0"))))], var(0)),
        (
            "update".to_owned(),
            vec![seeded(), let_(0, json!({ "op": "update", "entity": "Item", "key": { "id": lit(json!("i0")) }, "set": { "n": n() } }))],
            var(0),
        ),
        (
            "delete".to_owned(),
            vec![seeded(), let_(0, json!({ "op": "delete", "entity": "Item", "key": { "id": lit(json!("i0")) } }))],
            lit(json!(1)),
        ),
        ("ensure".to_owned(), vec![ensure(lit(json!(true)))], lit(json!(1))),
        (
            "upsert existing".to_owned(),
            vec![seeded(), let_(0, json!({ "op": "upsert", "entity": "Item", "values": item(json!({ "id": lit(json!("i0")) })) }))],
            var(0),
        ),
        (
            "insertIfAbsent".to_owned(),
            vec![let_(0, json!({ "op": "insertIfAbsent", "entity": "Item", "values": item(json!({})) }))],
            var(0),
        ),
        (
            "map".to_owned(),
            vec![let_(0, json!({
                "op": "map", "list": three(), "item": 1,
                "body": [let_(2, json!({ "op": "eval", "expr": bin("add", var(1), n()) }))],
                "yield": var(2),
            }))],
            var(0),
        ),
        (
            "filter".to_owned(),
            vec![let_(0, json!({ "op": "filter", "list": three(), "item": 1, "keep": bin("gte", var(1), lit(json!(0))) }))],
            var(0),
        ),
        (
            "forEach".to_owned(),
            vec![json!({ "kind": "forEach", "list": three(), "item": 1, "body": [
                let_(2, insert(item(json!({ "id": json!({ "kind": "concat", "parts": [lit(json!("x")), var(1)] }) })))),
            ] })],
            lit(json!(1)),
        ),
        (
            "loop".to_owned(),
            vec![let_(0, json!({
                "op": "loop", "max": 4, "state": { "i": lit(json!(0)) }, "var": 1,
                "body": [], "next": { "i": bin("add", field(var(1), "i"), lit(json!(1))) },
            }))],
            var(0),
        ),
        (
            "query a full page".to_owned(),
            vec![
                let_(7, insert(item(json!({ "id": lit(json!("q1")) })))),
                let_(8, insert(item(json!({ "id": lit(json!("q2")) })))),
                let_(0, json!({ "op": "query", "entity": "Item", "limit": lit(json!(2)) })),
            ],
            var(0),
        ),
        (
            "query a full page, descending".to_owned(),
            vec![
                let_(7, insert(item(json!({ "id": lit(json!("q1")) })))),
                let_(8, insert(item(json!({ "id": lit(json!("q2")) })))),
                let_(0, json!({ "op": "query", "entity": "Item", "limit": lit(json!(2)), "order": "desc" })),
            ],
            var(0),
        ),
        ("sum".to_owned(), vec![], json!({ "kind": "agg", "op": "sum", "list": three(), "item": 1, "value": var(1) })),
        ("count".to_owned(), vec![], json!({ "kind": "agg", "op": "count", "list": three(), "item": 1 })),
        ("sort".to_owned(), vec![], json!({ "kind": "sort", "list": three(), "item": 1, "by": var(1), "desc": true })),
        ("distinct".to_owned(), vec![], json!({ "kind": "distinct", "of": three() })),
        ("includes".to_owned(), vec![], json!({ "kind": "includes", "list": three(), "value": n() })),
        ("listConcat".to_owned(), vec![], json!({ "kind": "listConcat", "parts": [three(), three()] })),
        ("slice".to_owned(), vec![], json!({ "kind": "slice", "of": three(), "start": lit(json!(0)) })),
        (
            "log".to_owned(),
            vec![json!({ "kind": "log", "label": "x", "value": n() })],
            lit(json!(1)),
        ),
    ]);

    let mut slack = Vec::new();
    for (label, plan_body, output) in plans {
        let m = manifest(
            plan_body,
            json!({ "kind": "object", "fields": { "v": output } }),
        );
        assert_eq!(codes(&m), ok(), "{label}");
        let bounds = bounds(&m);
        let r = run(&m, json!({ "n": 5, "name": "abc" }));
        assert_eq!(r.status, 200, "{label}: {:?}", r.body);
        let used = r.usage;
        assert!(
            used.fits_within(&bounds),
            "{label}: {used:?} over {bounds:?}"
        );
        let counted = |c: Cost| (c.ops, c.reads, c.writes, c.effects, c.calls);
        if counted(used) != counted(bounds) {
            slack.push(format!(
                "{label}: used {:?}, bounds {:?}",
                counted(used),
                counted(bounds)
            ));
        }
    }
    assert!(
        slack.is_empty(),
        "(ops, reads, writes, effects, calls):\n{}",
        slack.join("\n")
    );
}

/// The harness manifest plus an index `byNameN` on (`name`, `n`) and a counter `all` of items.
fn with_index_and_counter(plan_body: Vec<Json>) -> Json {
    let mut m = manifest(plan_body, lit(json!(1)));
    m["entities"]["Item"]["indexes"] = json!({ "byNameN": { "fields": ["name", "n"] } });
    m["counters"] = json!({ "all": { "of": "Item", "groupBy": [] } });
    m
}

fn query(extra: Json) -> Json {
    let mut q =
        json!({ "op": "query", "entity": "Item", "index": "byNameN", "limit": lit(json!(5)) });
    for (k, v) in extra.as_object().unwrap() {
        q[k] = v.clone();
    }
    q
}

#[test]
fn any_of_takes_one_to_ten_values_and_no_cursor() {
    let check = |extra: Json| codes(&with_index_and_counter(vec![let_(0, query(extra))]));
    let values = |k: usize| Json::Array((0..k).map(|i| lit(json!(format!("v{i}")))).collect());
    assert_eq!(check(json!({ "anyOf": values(1) })), ok());
    assert_eq!(check(json!({ "anyOf": values(10) })), ok());
    assert_eq!(check(json!({ "anyOf": values(0) })), only("invalid_prefix"));
    assert_eq!(
        check(json!({ "anyOf": values(11) })),
        only("invalid_prefix")
    );
    assert_eq!(
        check(json!({ "anyOf": values(2), "page": true })),
        only("invalid_prefix")
    );
    // anyOf covers the field after the prefix, so a range bounds the one after that: with no
    // prefix, `n`; after a prefix for `name` and anyOf for `n`, nothing is left to bound.
    let range = json!({ "gt": lit(json!(1)) });
    assert_eq!(check(json!({ "anyOf": values(2), "range": range })), ok());
    assert_eq!(
        check(json!({ "prefix": [lit(json!("a"))], "anyOf": values(2), "range": range })),
        only("invalid_range")
    );
}

#[test]
fn a_range_has_at_most_one_bound_on_each_side() {
    let check = |range: Json| {
        codes(&with_index_and_counter(vec![let_(
            0,
            query(json!({ "range": range })),
        )]))
    };
    let one = || lit(json!("a"));
    assert_eq!(check(json!({ "gt": one(), "lt": one() })), ok());
    assert_eq!(check(json!({ "gte": one(), "lte": one() })), ok());
    assert_eq!(
        check(json!({ "gt": one(), "gte": one() })),
        only("invalid_range")
    );
    assert_eq!(
        check(json!({ "lt": one(), "lte": one() })),
        only("invalid_range")
    );
}

#[test]
fn counter_reads_need_one_key_value_per_group_field() {
    let get = |key: Json| {
        codes(&with_index_and_counter(vec![let_(
            0,
            json!({ "op": "counterGet", "counter": "all", "key": key }),
        )]))
    };
    assert_eq!(get(json!([])), ok());
    assert_eq!(get(json!([lit(json!(1))])), only("invalid_counter_key"));
}

#[test]
fn payloads_may_omit_optional_fields_but_not_required_ones() {
    let emit_with = |payload: Json| {
        let mut m = manifest(
            vec![let_(
                0,
                json!({ "op": "emit", "event": "ping", "payload": payload }),
            )],
            lit(json!(1)),
        );
        m["events"] = json!({ "ping": { "payload": {
            "req": { "type": { "kind": "int" } },
            "opt": { "type": { "kind": "int" }, "optional": true },
        } } });
        codes(&m)
    };
    assert_eq!(emit_with(json!({ "req": lit(json!(1)) })), ok());
    assert_eq!(
        emit_with(json!({ "req": lit(json!(1)), "opt": lit(json!(2)) })),
        ok()
    );
    assert_eq!(
        emit_with(json!({ "opt": lit(json!(2)) })),
        only("invalid_payload")
    );
    assert_eq!(
        emit_with(json!({ "req": lit(json!(1)), "nope": lit(json!(2)) })),
        only("invalid_payload")
    );
}

#[test]
fn select_on_is_null_narrows_only_the_checked_value() {
    // `name` of a row that may not exist is nullable; writing it to the required `name` field
    // needs a fallback for exactly that value.
    let maybe = || field(var(0), "name");
    let other = || field(var(0), "id");
    let fallback = || lit(json!("z"));
    let is_null = |of: Json| json!({ "kind": "isNull", "of": of });
    let select = |cond: Json, then: Json, otherwise: Json| json!({ "kind": "select", "cond": cond, "then": then, "else": otherwise });
    let write = |name: Json| {
        codes(&manifest(
            vec![
                let_(0, get(lit(json!("i0")))),
                let_(
                    1,
                    insert(item(json!({ "id": lit(json!("i1")), "name": name }))),
                ),
            ],
            lit(json!(1)),
        ))
    };
    assert_eq!(write(maybe()), only("type_mismatch"));
    assert_eq!(write(select(is_null(maybe()), fallback(), maybe())), ok());
    assert_eq!(write(select(not_null(maybe()), maybe(), fallback())), ok());
    // Checking one value says nothing about another.
    assert_eq!(
        write(select(is_null(other()), fallback(), maybe())),
        only("type_mismatch")
    );
    assert_eq!(
        write(select(not_null(other()), maybe(), fallback())),
        only("type_mismatch")
    );
}

#[test]
fn inserting_over_an_expired_row_stays_within_bounds_exactly() {
    // A TTL entity with an index and a counter: re-inserting a key whose row expired first
    // purges the old row (its row, index entry, and counter), which the bounds must cover.
    let insert_at = let_(
        0,
        insert(item(
            json!({ "id": lit(json!("i0")), "at": { "kind": "now" } }),
        )),
    );
    let mut m = with_index_and_counter(vec![insert_at]);
    m["entities"]["Item"]["fields"]["at"] = json!({ "type": { "kind": "timestamp" } });
    m["entities"]["Item"]["ttl"] = json!({ "field": "at", "ms": 10 });
    let bounds = bounds(&m);

    let valid = parse_and_validate(&m.to_string(), &Limits::ROUTE).unwrap();
    let env = Arc::new(FixedEnv::new(1_000, 1));
    let engine = Engine::new(valid, Arc::new(MemoryStore::new()), env.clone());
    let tenant = TenantId::new("t").unwrap();
    let request = Request {
        method: Method::Post,
        path: "/r".into(),
        query: BTreeMap::new(),
        body: Some(json!({ "n": 1, "name": "a" })),
        caller: None,
        ip: None,
        idempotency_key: None,
        headers: BTreeMap::new(),
        raw_body: None,
    };
    let first = engine.handle(&tenant, &request);
    assert_eq!(first.status, 200, "{:?}", first.body);
    env.advance(100);
    let second = engine.handle(&tenant, &request);
    assert_eq!(second.status, 200, "{:?}", second.body);
    assert!(
        second.usage.fits_within(&bounds),
        "{:?} over {bounds:?}",
        second.usage
    );
    assert_eq!(second.usage.writes, bounds.writes, "{:?}", second.usage);
    assert!(
        second.usage.writes > first.usage.writes,
        "the purge wrote too"
    );
}

#[test]
fn expression_limits_hold_at_their_exact_bounds() {
    let check = |expr: Json| codes(&manifest(vec![], obj(json!({ "v": expr }))));
    let name = || body("name");
    let split =
        |max: u32| json!({ "kind": "split", "of": name(), "sep": lit(json!(",")), "max": max });
    let pad = |length: u32, fill: &str| json!({ "kind": "pad", "of": name(), "length": length, "fill": fill });
    let matches = |pattern: String| json!({ "kind": "matches", "of": name(), "pattern": pattern });
    let format_time = |pattern: String| json!({ "kind": "formatTime", "of": { "kind": "now" }, "pattern": pattern });
    let bad = || only("invalid_expression");

    assert_eq!(check(split(1)), ok());
    assert_eq!(check(split(1000)), ok());
    assert_eq!(check(split(0)), bad());
    assert_eq!(check(split(1001)), bad());

    assert_eq!(check(pad(1000, "0")), ok());
    assert_eq!(check(pad(1001, "0")), bad());
    assert_eq!(check(pad(5, "")), bad());

    assert_eq!(check(matches("a".repeat(256))), ok());
    assert_eq!(check(matches("a".repeat(257))), bad());

    assert_eq!(check(format_time("x".repeat(64))), ok());
    assert_eq!(check(format_time("x".repeat(65))), bad());
}

#[test]
fn a_literal_slice_end_bounds_the_result() {
    let slice = |end: Option<i64>| {
        let mut s = json!({ "kind": "slice", "of": list(json!([lit(json!(1)), lit(json!(2)), lit(json!(3))])), "start": lit(json!(0)) });
        if let Some(end) = end {
            s["end"] = lit(json!(end));
        }
        bounds(&manifest(vec![], obj(json!({ "v": s })))).memory
    };
    let (whole, none, one) = (slice(None), slice(Some(0)), slice(Some(1)));
    assert!(none < one && one < whole, "{none} {one} {whole}");
    // A negative end counts from the end, so it doesn't bound the length.
    assert_eq!(slice(Some(-1)), whole);
}

/// The inferred output schema of the route `POST /r` whose output is `{ v: expr }`, at `v`.
fn output_of(expr: Json) -> Json {
    let m = manifest(vec![], obj(json!({ "v": expr })));
    let valid =
        parse_and_validate(&m.to_string(), &Limits::ROUTE).unwrap_or_else(|d| panic!("{d:#?}"));
    let output = serde_json::to_value(&valid.report().routes["r"].output).unwrap();
    output["fields"]["v"].clone()
}

fn dec(value: &str) -> Json {
    json!({ "kind": "decimal", "value": value })
}

fn bin(op: &str, lhs: Json, rhs: Json) -> Json {
    json!({ "kind": "binary", "op": op, "lhs": lhs, "rhs": rhs })
}

#[test]
fn decimal_arithmetic_tracks_scale() {
    let d2 = || dec("1.25");
    let d3 = || dec("1.125");
    let scale = |expr: Json| output_of(expr)["scale"].clone();
    assert_eq!(scale(bin("add", d2(), lit(json!(1)))), json!(2));
    assert_eq!(scale(bin("add", lit(json!(1)), d3())), json!(3));
    assert_eq!(scale(bin("sub", d2(), d3())), json!(3));
    assert_eq!(scale(bin("mul", d2(), d3())), json!(5));
    assert_eq!(scale(bin("mul", d2(), lit(json!(3)))), json!(2));
    assert_eq!(scale(json!({ "kind": "abs", "arg": d3() })), json!(3));
    let sum = json!({ "kind": "agg", "op": "sum", "list": list(json!([d2(), d2()])), "item": 1 });
    assert_eq!(scale(sum), json!(2));
    assert_eq!(
        output_of(bin("add", lit(json!(1)), lit(json!(2))))["kind"],
        "int"
    );
}

#[test]
fn min_and_max_keep_integer_bounds_for_query_limits() {
    // A query's limit must have a known maximum; min and max of bounded ints keep one.
    let reads = |limit: Json| {
        bounds(&manifest(
            vec![let_(
                0,
                json!({ "op": "query", "entity": "Item", "limit": limit }),
            )],
            lit(json!(1)),
        ))
        .reads
    };
    let three = || lit(json!(3));
    let n = || body("n");
    assert_eq!(reads(three()), reads(bin("min", n(), three())));
    assert_eq!(reads(three()), reads(bin("min", three(), n())));
    assert_eq!(reads(three()), reads(bin("max", lit(json!(2)), three())));
    assert_eq!(reads(three()), reads(bin("max", three(), lit(json!(1)))));
    assert!(reads(n()) > reads(three()), "n is bounded by 100");
    assert_eq!(reads(n()), reads(bin("max", n(), three())));
}

#[test]
fn operands_must_have_the_types_their_operators_need() {
    let check = |expr: Json| codes(&manifest(vec![], obj(json!({ "v": expr }))));
    let mismatch = || only("type_mismatch");
    let n = || body("n");
    let name = || body("name");
    let ints = || list(json!([lit(json!(1)), lit(json!(2))]));
    let agg = |op: &str, list: Json, value: Option<Json>| {
        let mut a = json!({ "kind": "agg", "op": op, "list": list, "item": 1 });
        if let Some(value) = value {
            a["value"] = value;
        }
        a
    };

    assert_eq!(check(bin("mod", n(), lit(json!(3)))), ok());
    assert_eq!(check(bin("mod", dec("1.5"), lit(json!(3)))), mismatch());
    assert_eq!(check(bin("add", name(), lit(json!(3)))), mismatch());

    // some/every need bools, from the value or the list itself; count may count anything.
    let bools = || list(json!([lit(json!(true))]));
    assert_eq!(check(agg("some", bools(), None)), ok());
    assert_eq!(check(agg("every", ints(), None)), mismatch());
    assert_eq!(check(agg("some", ints(), None)), mismatch());
    assert_eq!(check(agg("count", ints(), None)), ok());
    assert_eq!(
        check(agg("count", ints(), Some(bin("gt", var(1), lit(json!(1)))))),
        ok()
    );
    assert_eq!(check(agg("count", ints(), Some(var(1)))), mismatch());

    // min and max need values that order.
    let objects = || list(json!([obj(json!({ "a": lit(json!(1)) }))]));
    assert_eq!(check(agg("max", ints(), None)), ok());
    assert_eq!(check(agg("max", objects(), None)), mismatch());
    assert_eq!(check(agg("min", objects(), None)), mismatch());

    // Int arguments take ints (or null), not strings.
    let slice = |start: Json| json!({ "kind": "slice", "of": ints(), "start": start });
    assert_eq!(check(slice(lit(json!(1)))), ok());
    assert_eq!(check(slice(lit(Json::Null))), ok());
    assert_eq!(check(slice(lit(json!("1")))), mismatch());
}

#[test]
fn rounding_scales_and_time_zones_are_validated() {
    let check = |expr: Json| codes(&manifest(vec![], obj(json!({ "v": expr }))));
    let round = |scale: u8| json!({ "kind": "round", "of": dec("1.25"), "scale": scale });
    assert_eq!(check(round(18)), ok());
    assert_eq!(check(round(19)), only("invalid_expression"));
    let format =
        |tz: &str| json!({ "kind": "formatTime", "of": { "kind": "now" }, "tz": lit(json!(tz)) });
    assert_eq!(check(format("Europe/Berlin")), ok());
    assert_eq!(check(format("Mars/Olympus")), only("invalid_expression"));
}

#[test]
fn matching_on_anything_but_an_enum_needs_a_default_case() {
    let check = |cases: Json| {
        codes(&manifest(
            vec![],
            obj(json!({ "v": { "kind": "match", "on": body("name"), "cases": cases } })),
        ))
    };
    assert_eq!(
        check(json!({ "a": lit(json!(1)), "_": lit(json!(2)) })),
        ok()
    );
    assert_eq!(
        check(json!({ "a": lit(json!(1)) })),
        only("non_exhaustive_match")
    );
}

fn engine_for(m: &Json) -> (Engine, TenantId, Arc<FixedEnv>) {
    let valid =
        parse_and_validate(&m.to_string(), &Limits::ROUTE).unwrap_or_else(|d| panic!("{d:#?}"));
    let env = Arc::new(FixedEnv::new(1_000, 1));
    let engine = Engine::new(valid, Arc::new(MemoryStore::new()), env.clone());
    (engine, TenantId::new("t").unwrap(), env)
}

fn post(engine: &Engine, tenant: &TenantId) -> Response {
    engine.handle(
        tenant,
        &Request {
            method: Method::Post,
            path: "/r".into(),
            query: BTreeMap::new(),
            body: Some(json!({ "n": 1, "name": "a" })),
            caller: None,
            ip: None,
            idempotency_key: None,
            headers: BTreeMap::new(),
            raw_body: None,
        },
    )
}

/// The harness manifest with a queue `q` (payload `i`) whose handler `job` runs `job_body`.
fn with_queue(route_body: Vec<Json>, job_body: Vec<Json>) -> Json {
    let mut m = manifest(route_body, lit(json!(1)));
    m["queues"] = json!({ "q": { "payload": { "i": { "type": { "kind": "int" } } } } });
    m["handlers"] = json!([{
        "id": "job",
        "trigger": { "kind": "queue", "queue": "q" },
        "errors": {},
        "plan": { "body": job_body, "output": lit(Json::Null) },
    }]);
    m
}

fn enqueue(extra: Json) -> Json {
    let mut op = json!({ "op": "enqueue", "queue": "q", "payload": { "i": lit(json!(0)) } });
    for (k, v) in extra.as_object().unwrap() {
        op[k] = v.clone();
    }
    op
}

#[test]
fn enqueue_delays_are_capped_at_30_days() {
    let status = |delay: i64| {
        let m = with_queue(
            vec![let_(0, enqueue(json!({ "delayMs": lit(json!(delay)) })))],
            vec![],
        );
        let (engine, tenant, _) = engine_for(&m);
        post(&engine, &tenant).status
    };
    let cap = 30 * 24 * 60 * 60 * 1000;
    assert_eq!(status(0), 200);
    assert_eq!(status(cap), 200);
    assert_eq!(status(cap + 1), 500);
    assert_eq!(status(-1), 500);
}

#[test]
fn job_and_workflow_keys_are_1_to_256_bytes() {
    let job = |key: String| {
        let m = with_queue(
            vec![let_(0, enqueue(json!({ "key": lit(json!(key)) })))],
            vec![],
        );
        let (engine, tenant, _) = engine_for(&m);
        post(&engine, &tenant).status
    };
    assert_eq!(job("k".repeat(256)), 200);
    assert_eq!(job("k".repeat(257)), 500);
    assert_eq!(job(String::new()), 500);

    let workflow = |key: String| {
        let mut m = manifest(
            vec![let_(
                0,
                json!({ "op": "startWorkflow", "workflow": "w", "key": lit(json!(key)), "input": {} }),
            )],
            lit(json!(1)),
        );
        m["workflows"] = json!({ "w": { "input": {}, "start": "a" } });
        m["handlers"] = json!([{
            "id": "w_a",
            "trigger": { "kind": "workflow", "workflow": "w", "step": "a" },
            "errors": {},
            "plan": { "body": [{ "kind": "complete" }], "output": lit(Json::Null) },
        }]);
        let (engine, tenant, _) = engine_for(&m);
        post(&engine, &tenant).status
    };
    assert_eq!(workflow("k".repeat(256)), 200);
    assert_eq!(workflow("k".repeat(257)), 500);
    assert_eq!(workflow(String::new()), 500);
}

#[test]
fn a_job_that_re_enqueues_its_queue_continues_past_the_chain_depth() {
    // Each run re-enqueues the next batch until i reaches 20, more hops than `MAX_CHAIN_DEPTH`
    // allows, but a continuation stays on its hop.
    let i = || json!({ "kind": "input", "source": "payload", "name": "i" });
    let again = json!({
        "kind": "if",
        "cond": bin("lt", i(), lit(json!(20))),
        "then": [let_(1, json!({ "op": "enqueue", "queue": "q", "payload": { "i": bin("add", i(), lit(json!(1))) } }))],
        "else": [],
    });
    let m = with_queue(vec![let_(0, enqueue(json!({})))], vec![again]);
    let (engine, tenant, _) = engine_for(&m);
    assert_eq!(post(&engine, &tenant).status, 200);
    let report = engine.process(&tenant, 1_000);
    assert_eq!(report.failures, Vec::<String>::new());
    assert_eq!(report.ran, 21, "{report:#?}");
}

#[test]
fn an_upsert_over_an_expired_row_starts_a_fresh_row() {
    // Route 1 inserts `i0` tagged "old"; route `r` upserts `i0` without a tag. Over a live row
    // the tag stays; over an expired one the row is replaced, so the tag is gone.
    let upsert = let_(
        0,
        json!({ "op": "upsert", "entity": "Item", "values": item(json!({ "id": lit(json!("i0")), "at": { "kind": "now" } })) }),
    );
    let mut m = manifest(vec![upsert], field(var(0), "tag"));
    m["entities"]["Item"]["fields"]["at"] = json!({ "type": { "kind": "timestamp" } });
    m["entities"]["Item"]["ttl"] = json!({ "field": "at", "ms": 10 });
    let seed =
        item(json!({ "id": lit(json!("i0")), "tag": lit(json!("old")), "at": { "kind": "now" } }));
    let mut seed_route = m["routes"][0].clone();
    seed_route["id"] = json!("seed");
    seed_route["path"] = json!("/seed");
    seed_route["plan"] = json!({ "body": [let_(0, insert(seed))], "output": lit(json!(1)) });
    m["routes"].as_array_mut().unwrap().push(seed_route);

    let tag_after = |wait_ms: i64| {
        let (engine, tenant, env) = engine_for(&m);
        let seeded = engine.handle(
            &tenant,
            &Request {
                path: "/seed".into(),
                ..request_for(json!({ "n": 1, "name": "a" }))
            },
        );
        assert_eq!(seeded.status, 200, "{:?}", seeded.body);
        env.advance(wait_ms);
        let r = post(&engine, &tenant);
        assert_eq!(r.status, 200, "{:?}", r.body);
        r.body.clone()
    };
    assert_eq!(tag_after(1), json!("old"));
    assert_eq!(tag_after(100), Json::Null);
}

fn request_for(body: Json) -> Request {
    Request {
        method: Method::Post,
        path: "/r".into(),
        query: BTreeMap::new(),
        body: Some(body),
        caller: None,
        ip: None,
        idempotency_key: None,
        headers: BTreeMap::new(),
        raw_body: None,
    }
}

#[test]
fn an_expired_row_cant_be_updated() {
    let update = let_(
        0,
        json!({ "op": "update", "entity": "Item", "key": { "id": lit(json!("i0")) }, "set": { "n": body("n") } }),
    );
    let mut m = manifest(vec![update], lit(json!(1)));
    m["entities"]["Item"]["fields"]["at"] = json!({ "type": { "kind": "timestamp" } });
    m["entities"]["Item"]["ttl"] = json!({ "field": "at", "ms": 10 });
    let mut seed_route = m["routes"][0].clone();
    seed_route["id"] = json!("seed");
    seed_route["path"] = json!("/seed");
    let seed = item(json!({ "id": lit(json!("i0")), "at": { "kind": "now" } }));
    seed_route["plan"] = json!({ "body": [let_(0, insert(seed))], "output": lit(json!(1)) });
    m["routes"].as_array_mut().unwrap().push(seed_route);
    let status_after = |wait_ms: i64| {
        let (engine, tenant, env) = engine_for(&m);
        let seeded = engine.handle(
            &tenant,
            &Request {
                path: "/seed".into(),
                ..request_for(json!({ "n": 1, "name": "a" }))
            },
        );
        assert_eq!(seeded.status, 200, "{:?}", seeded.body);
        env.advance(wait_ms);
        post(&engine, &tenant).status
    };
    assert_eq!(status_after(1), 200);
    assert_eq!(status_after(100), 404);
}
