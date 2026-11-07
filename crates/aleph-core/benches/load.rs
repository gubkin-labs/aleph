//! Loading apps: how long a manifest takes to validate and load, and how much heap a loaded app
//! holds. Checks the homepage's claims (manifests load in milliseconds; thousands of apps fit in
//! one process). Also times validation of very large and deeply nested manifests.
//!
//! `cargo bench -p aleph-core --bench load` prints a table and then runs criterion. `-- --report`
//! prints only the table.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::needless_pass_by_value,
    missing_docs
)]

use std::hint::black_box;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use aleph_core::{Engine, FixedEnv, Limits, MemoryStore, parse_and_validate};
use criterion::Criterion;
use peak_alloc::PeakAlloc;
use serde_json::{Value as Json, json};

#[global_allocator]
static HEAP: PeakAlloc = PeakAlloc;

const APPS: [&str; 5] = ["commerce", "fulfillment", "ledger", "pipeline", "projects"];

fn manifest(case: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../conformance/cases/{case}/manifest.json"));
    std::fs::read_to_string(path).unwrap()
}

fn load(json: &str) -> Engine {
    let valid = parse_and_validate(json, &Limits::ROUTE).unwrap();
    Engine::new(
        valid,
        Arc::new(MemoryStore::new()),
        Arc::new(FixedEnv::new(1_700_000_000_000, 1)),
    )
}

/// A manifest with `n` copies of the ledger's routes (renamed), to see how validation scales.
fn wide(n: usize) -> String {
    let mut m: Json = serde_json::from_str(&manifest("ledger")).unwrap();
    let routes = m["routes"].as_array().unwrap().clone();
    let mut out = Vec::new();
    for i in 0..n {
        for r in &routes {
            let mut r = r.clone();
            r["id"] = json!(format!("{}_{i}", r["id"].as_str().unwrap()));
            r["path"] = json!(format!("/v{i}{}", r["path"].as_str().unwrap()));
            out.push(r);
        }
    }
    m["routes"] = Json::Array(out);
    m.to_string()
}

/// A route whose output is `depth` nested `not`s: as deep as JSON parsing allows.
fn deep(depth: usize) -> String {
    let mut e = json!({ "kind": "lit", "value": true });
    for _ in 0..depth {
        e = json!({ "kind": "not", "arg": e });
    }
    route(&[], e)
}

/// `depth` loops nested in each other's bodies, one pass each. The checker checks a loop body
/// twice (to find the state's type), so this measures whether that compounds.
fn nested_loops(depth: u32) -> String {
    let mut stmts: Vec<Json> = Vec::new();
    for d in (0..depth).rev() {
        let op = json!({ "op": "loop", "max": 1, "var": d * 2, "state": { "n": { "kind": "lit", "value": 0 } },
            "body": stmts, "next": { "n": { "kind": "lit", "value": 1 } } });
        stmts = vec![json!({ "kind": "let", "var": d * 2 + 1, "op": op })];
    }
    route(&stmts, json!({ "kind": "lit", "value": 1 }))
}

fn route(stmts: &[Json], output: Json) -> String {
    json!({
        "formatVersion": "0.2",
        "app": { "name": "load" },
        "routes": [{ "id": "x", "method": "GET", "path": "/x", "auth": { "kind": "public" },
            "plan": { "body": stmts, "output": output } }],
    })
    .to_string()
}

fn time_check(name: &str, json: &str) {
    let started = Instant::now();
    let result = parse_and_validate(json, &Limits::ROUTE);
    let elapsed = started.elapsed();
    let status = match result {
        Ok(_) => "valid".to_owned(),
        Err(d) => format!(
            "invalid: {}",
            d[0].message.chars().take(60).collect::<String>()
        ),
    };
    println!(
        "{name:<34} {:>8.1} KB  check {:>9.2} ms  {status}",
        json.len() as f64 / 1024.0,
        elapsed.as_secs_f64() * 1000.0,
    );
}

fn report() {
    println!(
        "{:<12} {:>9} {:>11} {:>11} {:>13} {:>13}",
        "app", "manifest", "validate", "Engine::new", "heap/app", "peak to load"
    );
    for app in APPS {
        let json = manifest(app);
        // Warm up, then time one validation and one load.
        drop(load(&json));
        let started = Instant::now();
        let valid = parse_and_validate(&json, &Limits::ROUTE).unwrap();
        let validate = started.elapsed();
        let started = Instant::now();
        let engine = Engine::new(
            valid,
            Arc::new(MemoryStore::new()),
            Arc::new(FixedEnv::new(1_700_000_000_000, 1)),
        );
        let new = started.elapsed();
        drop(engine);
        // Retained heap: load 1,000 copies and divide.
        let before = HEAP.current_usage();
        HEAP.reset_peak_usage();
        let engines: Vec<Engine> = (0..1000).map(|_| load(&json)).collect();
        let per_app = (HEAP.current_usage() - before) / engines.len();
        drop(engines);
        HEAP.reset_peak_usage();
        let before = HEAP.current_usage();
        drop(load(&json));
        let peak = HEAP.peak_usage() - before;
        println!(
            "{app:<12} {:>6.1} KB {:>8.2} ms {:>8.3} ms {:>10.1} KB {:>10.1} KB",
            json.len() as f64 / 1024.0,
            validate.as_secs_f64() * 1000.0,
            new.as_secs_f64() * 1000.0,
            per_app as f64 / 1024.0,
            peak as f64 / 1024.0,
        );
    }
    println!();
    for n in [10, 100] {
        time_check(&format!("ledger routes x{n}"), &wide(n));
    }
    time_check("not() nested 120 deep", &deep(120));
    time_check("not() nested 200 deep", &deep(200));
    for depth in [8, 16, 20, 36] {
        time_check(&format!("loops nested {depth} deep"), &nested_loops(depth));
    }
}

fn main() {
    report();
    if std::env::args().any(|a| a == "--report") {
        return;
    }
    let mut c = Criterion::default().configure_from_args();
    for app in APPS {
        let json = manifest(app);
        c.bench_function(&format!("validate/{app}"), |b| {
            b.iter(|| black_box(parse_and_validate(&json, &Limits::ROUTE).is_ok()));
        });
        c.bench_function(&format!("load/{app}"), |b| {
            b.iter(|| black_box(load(&json)));
        });
    }
    let wide = wide(100);
    c.bench_function("validate/ledger routes x100", |b| {
        b.iter(|| black_box(parse_and_validate(&wide, &Limits::ROUTE).is_ok()));
    });
    c.final_summary();
}
