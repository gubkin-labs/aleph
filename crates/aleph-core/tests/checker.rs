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
