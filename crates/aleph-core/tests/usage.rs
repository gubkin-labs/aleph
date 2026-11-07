//! Usage counts real work: expressions that copy, build, or match big values are charged for it,
//! so the checker rejects routes that would take long or allocate far more than their bounds.
//! `benches/adversarial.rs` measures the same attacks.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aleph_core::manifest::Method;
use aleph_core::{
    DiagnosticCode, Engine, FixedEnv, Limits, MemoryStore, Request, TenantId, parse_and_validate,
};
use serde_json::{Value as Json, json};

fn route(inputs: &Json, stmts: &[Json], output: &Json) -> String {
    json!({
        "formatVersion": "0.2",
        "app": { "name": "usage" },
        "routes": [{ "id": "x", "method": "POST", "path": "/x", "auth": { "kind": "public" },
            "input": { "body": inputs }, "plan": { "body": stmts, "output": output } }],
    })
    .to_string()
}

fn text(max: u32) -> Json {
    json!({ "s": { "type": { "kind": "string", "min": 0, "max": max } } })
}

fn s() -> Json {
    json!({ "kind": "input", "source": "body", "name": "s" })
}

fn codes(manifest: &str) -> Vec<DiagnosticCode> {
    parse_and_validate(manifest, &Limits::ROUTE)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.code)
        .collect()
}

fn run(manifest: &str, body: &Json) -> aleph_core::Response {
    let valid = parse_and_validate(manifest, &Limits::ROUTE).unwrap();
    let engine = Engine::new(
        valid,
        Arc::new(MemoryStore::new()),
        Arc::new(FixedEnv::new(1_700_000_000_000, 1)),
    );
    engine.handle(
        &TenantId::new("t").unwrap(),
        &Request {
            method: Method::Post,
            path: "/x".into(),
            query: BTreeMap::new(),
            body: Some(body.clone()),
            caller: None,
            ip: None,
            idempotency_key: None,
            headers: BTreeMap::new(),
            raw_body: None,
        },
    )
}

#[test]
fn copies_of_a_big_input_count_toward_memory() {
    // 200 copies of a 64 KB string, concatenated: ~50 MB, never bound to a variable.
    let concat = json!({ "kind": "length", "of": { "kind": "concat", "parts": vec![s(); 200] } });
    assert_eq!(
        codes(&route(&text(65_536), &[], &concat)),
        [DiagnosticCode::OverBudget]
    );
    // Small ones fit, and the run stays within its bounds.
    let concat = json!({ "kind": "length", "of": { "kind": "concat", "parts": vec![s(); 20] } });
    let manifest = route(&text(1000), &[], &concat);
    let r = run(&manifest, &json!({ "s": "a".repeat(1000) }));
    assert_eq!(r.body, json!(20_000));
    let bounds = parse_and_validate(&manifest, &Limits::ROUTE)
        .unwrap()
        .report()
        .routes["x"]
        .bounds;
    assert!(r.usage.fits_within(&bounds), "{:?} > {bounds:?}", r.usage);
    assert!(r.usage.memory > 20 * 1000, "{:?}", r.usage);
}

#[test]
fn matching_is_charged_per_character() {
    let matches = |pattern: &str| json!({ "kind": "matches", "of": s(), "pattern": pattern });
    assert_eq!(
        codes(&route(&text(100), &[], &matches("a{0,1100}b"))),
        [DiagnosticCode::InvalidExpression]
    );
    let big = format!("(?:a|b|c|d|e|f|g|h|i|j)*{}z", "[a-z]".repeat(40));
    assert_eq!(
        codes(&route(&text(65_536), &[], &matches(&big))),
        [DiagnosticCode::OverBudget]
    );
    let unbounded = json!({ "kind": "matches", "pattern": "^a+$",
        "of": { "kind": "concat", "parts": [{ "kind": "list", "items": [s()] }] } });
    assert_eq!(
        codes(&route(&text(100), &[], &unbounded)),
        [DiagnosticCode::InvalidExpression]
    );
    // A text near the limit is charged most of the route's ops, and runs in a few ms.
    let manifest = route(&text(1100), &[], &matches("a{0,100}b"));
    let started = Instant::now();
    let r = run(&manifest, &json!({ "s": "a".repeat(1100) }));
    assert_eq!(r.body, json!(false));
    assert!(r.usage.ops > 1500, "{:?}", r.usage);
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[test]
fn in_scans_long_literal_lists_for_ops() {
    let list: Vec<Json> = (0..6400).map(|i| json!(i)).collect();
    let manifest = route(
        &json!({}),
        &[],
        &json!({ "kind": "in", "value": { "kind": "lit", "value": -1 }, "list": list }),
    );
    let r = run(&manifest, &json!({}));
    assert_eq!(r.body, json!(false));
    assert_eq!(r.usage.ops, 102);
}

#[test]
fn nested_loops_check_in_linear_time() {
    // The checker checks a loop's body twice to find its state type; nested, that used to double
    // per level (20 levels took ~2 s; JSON allows ~40).
    let mut stmts: Vec<Json> = Vec::new();
    for d in (0..36_u32).rev() {
        let op = json!({ "op": "loop", "max": 1, "var": d * 2,
            "state": { "n": { "kind": "lit", "value": 0 } }, "body": stmts,
            "next": { "n": { "kind": "lit", "value": 1 } } });
        stmts = vec![json!({ "kind": "let", "var": d * 2 + 1, "op": op })];
    }
    let manifest = route(&json!({}), &stmts, &json!({ "kind": "lit", "value": 1 }));
    let started = Instant::now();
    assert_eq!(codes(&manifest), []);
    assert!(started.elapsed() < Duration::from_secs(1));
}
