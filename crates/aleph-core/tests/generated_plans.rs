//! Generated plans and mutated manifests against the trusted core (tickets 002 and 003).
//!
//! A seeded generator builds random, mostly well-typed plans over a fixed schema that exercises
//! indexes, unique indexes, counters (count, sum, min/max, distinct, sharded), decimals, objects,
//! and lists. For every plan that validates, and random inputs against a seeded store:
//! 1. nothing panics (the validator rejects, or the engine answers);
//! 2. actual usage never exceeds the static bounds, on every resource (differential test);
//! 3. the execution only touches its own tenant's keys;
//! 4. a failed execution leaves the store exactly as it was.
//!
//! Mutated conformance manifests must never panic the validator or the engine.
//!
//! `PROPTEST_CASES=100000 cargo test --release -p aleph-core --test generated_plans` runs the
//! long version; the default keeps `pnpm verify` fast.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    clippy::range_plus_one,
    missing_docs
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use aleph_core::keys::Keyspace;
use aleph_core::{
    Caller, Engine, FixedEnv, Limits, MemoryStore, Request, TenantId, parse_and_validate,
};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

/// A small deterministic RNG (splitmix64), so a failing seed reproduces exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[usize::try_from(self.below(items.len() as u64)).unwrap()]
    }
}

/// What the generator knows about a bound variable.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Int,
    Row,
    Rows,
}

struct Gen {
    rng: Rng,
    next_var: u32,
    vars: Vec<(u32, Kind)>,
}

fn lit(v: Json) -> Json {
    json!({ "kind": "lit", "value": v })
}

fn body(name: &str) -> Json {
    json!({ "kind": "input", "source": "body", "name": name })
}

fn var(id: u32) -> Json {
    json!({ "kind": "var", "id": id })
}

fn field(of: Json, name: &str) -> Json {
    json!({ "kind": "field", "of": of, "name": name })
}

impl Gen {
    fn fresh(&mut self) -> u32 {
        self.next_var += 1;
        self.next_var
    }

    fn var_of(&mut self, kind: Kind) -> Option<u32> {
        let matching: Vec<u32> = self
            .vars
            .iter()
            .filter(|(_, k)| *k == kind)
            .map(|(v, _)| *v)
            .collect();
        (!matching.is_empty()).then(|| *self.rng.pick(&matching))
    }

    fn int(&mut self, depth: u32) -> Json {
        let choice = self.rng.below(if depth > 2 { 3 } else { 9 });
        match choice {
            0 => lit(json!(self.rng.below(60))),
            1 => body("n"),
            2 => match self.var_of(Kind::Int) {
                Some(v) => var(v),
                None => lit(json!(1)),
            },
            3 => {
                let op = *self
                    .rng
                    .pick(&["add", "sub", "mul", "min", "max", "mod", "divFloor"]);
                let rhs = if matches!(op, "mod" | "divFloor") {
                    lit(json!(1 + self.rng.below(9)))
                } else {
                    self.int(depth + 1)
                };
                json!({ "kind": "binary", "op": op, "lhs": self.int(depth + 1), "rhs": rhs })
            }
            4 => match self.var_of(Kind::Row) {
                Some(v) => field(var(v), "n"),
                None => lit(json!(2)),
            },
            5 => match self.var_of(Kind::Rows) {
                Some(v) => json!({ "kind": "length", "of": var(v) }),
                None => lit(json!(3)),
            },
            6 => match self.var_of(Kind::Rows) {
                Some(v) => {
                    let item = self.fresh();
                    let op = *self.rng.pick(&["sum", "count", "max"]);
                    let value = if op == "count" {
                        json!({ "kind": "binary", "op": "gt", "lhs": field(var(item), "n"), "rhs": lit(json!(5)) })
                    } else {
                        field(var(item), "n")
                    };
                    let agg = json!({ "kind": "agg", "op": op, "list": var(v), "item": item, "value": value });
                    if op == "max" {
                        json!({ "kind": "select", "cond": json!({ "kind": "isNull", "of": agg.clone() }), "then": lit(json!(0)), "else": agg })
                    } else {
                        agg
                    }
                }
                None => lit(json!(4)),
            },
            7 => json!({ "kind": "length", "of": self.string(depth + 1) }),
            _ => json!({ "kind": "abs", "arg": self.int(depth + 1) }),
        }
    }

    fn boolean(&mut self, depth: u32) -> Json {
        match self.rng.below(if depth > 2 { 2 } else { 6 }) {
            0 => lit(json!(self.rng.chance(50))),
            1 => {
                let op = *self.rng.pick(&["eq", "neq", "lt", "lte", "gt", "gte"]);
                json!({ "kind": "binary", "op": op, "lhs": self.int(depth + 1), "rhs": self.int(depth + 1) })
            }
            2 => {
                let op = *self.rng.pick(&["and", "or"]);
                json!({ "kind": "binary", "op": op, "lhs": self.boolean(depth + 1), "rhs": self.boolean(depth + 1) })
            }
            3 => match self.var_of(Kind::Row) {
                Some(v) => json!({ "kind": "not", "arg": { "kind": "isNull", "of": var(v) } }),
                None => lit(json!(true)),
            },
            4 => {
                json!({ "kind": "strTest", "op": "startsWith", "of": self.string(depth + 1), "arg": lit(json!("a")) })
            }
            _ => match self.var_of(Kind::Rows) {
                Some(v) => {
                    let item = self.fresh();
                    json!({ "kind": "agg", "op": "some", "list": var(v), "item": item,
                            "value": { "kind": "binary", "op": "eq", "lhs": field(var(item), "g"), "rhs": body("g") } })
                }
                None => lit(json!(false)),
            },
        }
    }

    fn string(&mut self, depth: u32) -> Json {
        match self.rng.below(if depth > 2 { 2 } else { 6 }) {
            0 => lit(json!(*self.rng.pick(&["a", "b", "ab", ""]))),
            1 => body("g"),
            2 => {
                json!({ "kind": "str", "op": *self.rng.pick(&["lower", "upper", "trim"]), "of": self.string(depth + 1) })
            }
            3 => json!({ "kind": "truncate", "of": self.string(depth + 1), "max": 8 }),
            4 => {
                json!({ "kind": "slice", "of": self.string(depth + 1), "start": lit(json!(0)), "end": lit(json!(4)) })
            }
            _ => json!({ "kind": "pad", "of": self.string(depth + 1), "length": 3, "fill": "x" }),
        }
    }

    /// A key-able group string (at most 8 characters, so rows stay valid).
    fn group(&mut self) -> Json {
        json!({ "kind": "truncate", "of": self.string(1), "max": 8 })
    }

    fn values(&mut self) -> Json {
        let mut values = serde_json::Map::new();
        values.insert("g".into(), self.group());
        values.insert(
            "n".into(),
            json!({ "kind": "binary", "op": "min", "lhs": { "kind": "abs", "arg": self.int(1) }, "rhs": lit(json!(1000)) }),
        );
        if self.rng.chance(50) {
            values.insert("id".into(), lit(json!(self.rng.below(20))));
        } else {
            values.insert("id".into(), body("id"));
        }
        if self.rng.chance(40) {
            values.insert(
                "price".into(),
                json!({ "kind": "round", "of": self.int(2), "scale": 2 }),
            );
        }
        if self.rng.chance(30) {
            values.insert(
                "info".into(),
                json!({ "kind": "object", "fields": { "a": self.int(2), "b": lit(Json::Null) } }),
            );
        }
        Json::Object(values)
    }

    fn key(&mut self) -> Json {
        json!({ "g": self.group(), "id": if self.rng.chance(50) { body("id") } else { lit(json!(self.rng.below(20))) } })
    }

    /// A statement; writes only outside pure (map/filter) bodies.
    fn stmt(&mut self, depth: u32, pure: bool) -> Json {
        let choice = self.rng.below(if depth > 1 { 6 } else { 10 });
        match choice {
            0 | 1 => {
                let v = self.fresh();
                let op = json!({ "op": "get", "entity": "Item", "key": self.key() });
                self.vars.push((v, Kind::Row));
                json!({ "kind": "let", "var": v, "op": op })
            }
            2 => {
                let v = self.fresh();
                let limit = lit(json!(self.rng.below(12)));
                let op = if self.rng.chance(50) {
                    json!({ "op": "query", "entity": "Item", "prefix": [self.group()], "limit": limit })
                } else {
                    let order = if self.rng.chance(50) { "asc" } else { "desc" };
                    json!({ "op": "query", "entity": "Item", "index": "byN", "prefix": [self.group()],
                            "limit": limit, "order": order, "range": { "gte": lit(json!(self.rng.below(10))) } })
                };
                self.vars.push((v, Kind::Rows));
                json!({ "kind": "let", "var": v, "op": op })
            }
            3 => {
                let v = self.fresh();
                let counter = *self.rng.pick(&["count", "total", "hot", "distinctN"]);
                let op = json!({ "op": "counterGet", "counter": counter, "key": [self.group()] });
                self.vars.push((v, Kind::Int));
                json!({ "kind": "let", "var": v, "op": op })
            }
            4 => {
                let v = self.fresh();
                let expr = self.int(0);
                self.vars.push((v, Kind::Int));
                json!({ "kind": "let", "var": v, "op": { "op": "eval", "expr": expr } })
            }
            5 => json!({ "kind": "ensure", "cond": self.boolean(1), "error": "nope" }),
            6 if !pure => {
                let v = self.fresh();
                let op = match self.rng.below(5) {
                    0 => json!({ "op": "insert", "entity": "Item", "values": self.values() }),
                    1 => json!({ "op": "upsert", "entity": "Item", "values": self.values() }),
                    2 => {
                        json!({ "op": "insertIfAbsent", "entity": "Item", "values": self.values() })
                    }
                    3 => json!({ "op": "update", "entity": "Item", "key": self.key(),
                                 "set": { "n": json!({ "kind": "binary", "op": "min", "lhs": { "kind": "abs", "arg": self.int(1) }, "rhs": lit(json!(1000)) }) } }),
                    _ => json!({ "op": "delete", "entity": "Item", "key": self.key() }),
                };
                json!({ "kind": "let", "var": v, "op": op })
            }
            7 => {
                let then: Vec<Json> = (0..self.rng.below(3))
                    .map(|_| self.stmt(depth + 1, pure))
                    .collect();
                let otherwise: Vec<Json> = (0..self.rng.below(2))
                    .map(|_| self.stmt(depth + 1, pure))
                    .collect();
                // Variables bound inside a branch aren't visible after it.
                self.vars
                    .truncate(self.vars.len().saturating_sub(then.len() + otherwise.len()));
                json!({ "kind": "if", "cond": self.boolean(1), "then": then, "else": otherwise })
            }
            8 if !pure => {
                let Some(list) = self.var_of(Kind::Rows) else {
                    return json!({ "kind": "ensure", "cond": lit(json!(true)), "error": "nope" });
                };
                let item = self.fresh();
                let saved = self.vars.len();
                self.vars.push((item, Kind::Row));
                let body_stmts: Vec<Json> = (0..1 + self.rng.below(2))
                    .map(|_| self.stmt(depth + 1, pure))
                    .collect();
                self.vars.truncate(saved);
                json!({ "kind": "forEach", "list": var(list), "item": item, "body": body_stmts })
            }
            _ => {
                let Some(list) = self.var_of(Kind::Rows) else {
                    return json!({ "kind": "ensure", "cond": lit(json!(true)), "error": "nope" });
                };
                let v = self.fresh();
                let item = self.fresh();
                let saved = self.vars.len();
                self.vars.push((item, Kind::Row));
                let op = if self.rng.chance(50) {
                    let inner: Vec<Json> = (0..self.rng.below(2))
                        .map(|_| self.stmt(depth + 1, true))
                        .collect();
                    let result = self.int(1);
                    json!({ "op": "map", "list": var(list), "item": item, "body": inner, "yield": { "kind": "object", "fields": { "n": result } } })
                } else {
                    let keep = self.boolean(1);
                    json!({ "op": "filter", "list": var(list), "item": item, "body": [], "keep": keep })
                };
                self.vars.truncate(saved);
                self.vars.push((v, Kind::Rows));
                json!({ "kind": "let", "var": v, "op": op })
            }
        }
    }

    fn plan(&mut self) -> Json {
        let stmts: Vec<Json> = (0..1 + self.rng.below(6))
            .map(|_| self.stmt(0, false))
            .collect();
        let output = match self.rng.below(3) {
            0 => self.int(0),
            1 => {
                json!({ "kind": "object", "fields": { "ok": self.boolean(0), "s": self.string(0) } })
            }
            _ => match self.var_of(Kind::Rows) {
                Some(v) => var(v),
                None => lit(Json::Null),
            },
        };
        json!({ "body": stmts, "output": output })
    }
}

fn int_field(max: i64) -> Json {
    json!({ "type": { "kind": "int", "min": 0, "max": max } })
}

fn manifest(plan: &Json) -> String {
    let n = || json!({ "kind": "var", "id": 0 });
    json!({
        "formatVersion": "0.2",
        "app": { "name": "generated" },
        "entities": {
            "Item": {
                "key": ["g", "id"],
                "fields": {
                    "g": { "type": { "kind": "string", "max": 8 } },
                    "id": int_field(1000),
                    "n": int_field(1000),
                    "price": { "type": { "kind": "decimal", "scale": 2 }, "optional": true },
                    "info": { "type": { "kind": "object", "fields": {
                        "a": { "type": { "kind": "int" } },
                        "b": { "type": { "kind": "string", "max": 4 }, "optional": true }
                    } }, "optional": true },
                },
                "indexes": {
                    "byN": { "fields": ["g", "n"] },
                    "oneBig": { "fields": ["g"], "unique": true, "where": { "var": 0, "expr": { "kind": "binary", "op": "gt", "lhs": field(n(), "n"), "rhs": lit(json!(990)) } } },
                },
            }
        },
        "counters": {
            "count": { "of": "Item", "groupBy": ["g"] },
            "total": { "of": "Item", "groupBy": ["g"], "sum": { "var": 0, "expr": field(n(), "n") } },
            "hot": { "of": "Item", "groupBy": ["g"], "shards": 4 },
            "distinctN": { "of": "Item", "groupBy": ["g"], "agg": { "kind": "distinct", "field": "n" } },
            "smallest": { "of": "Item", "groupBy": ["g"], "agg": { "kind": "min", "field": "n" } },
        },
        "routes": [{
            "id": "generated",
            "method": "POST",
            "path": "/run",
            "auth": { "kind": "public" },
            "input": { "body": {
                "n": int_field(50),
                "id": int_field(30),
                "g": { "type": { "kind": "string", "max": 4 } },
            } },
            "errors": { "nope": { "status": 409, "message": "nope" } },
            "plan": plan,
        }],
    })
    .to_string()
}

/// Every stored entry, as bytes.
fn snapshot(store: &MemoryStore) -> Vec<(Vec<u8>, Vec<u8>)> {
    store
        .entries()
        .into_iter()
        .map(|(k, v)| (k.as_bytes().to_vec(), v))
        .collect()
}

static VALID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn run_seed(seed: u64, n: i64, id: i64, group: &str) -> bool {
    let mut generator = Gen {
        rng: Rng(seed),
        next_var: 0,
        vars: Vec::new(),
    };
    let plan = generator.plan();
    let manifest = manifest(&plan);
    let Ok(valid) = parse_and_validate(&manifest, &Limits::ROUTE) else {
        return false;
    };
    let bounds = valid.report().routes["generated"].bounds;
    let store = Arc::new(MemoryStore::new());
    let engine = Engine::new(valid, store.clone(), Arc::new(FixedEnv::new(1_000, seed)));
    let (mine, other) = (
        TenantId::new("mine").unwrap(),
        TenantId::new("other").unwrap(),
    );
    let rows: Vec<Json> = (0..12)
        .map(|i| {
            let g = ["a", "b", "ab"][i % 3];
            json!({ "g": g, "id": i, "n": (i * 7) % 40 })
        })
        .collect();
    engine.seed(&mine, "Item", &rows).unwrap();
    engine.seed(&other, "Item", &rows).unwrap();
    let request = Request {
        method: aleph_core::manifest::Method::Post,
        path: "/run".into(),
        query: BTreeMap::new(),
        body: Some(json!({ "n": n, "id": id, "g": group })),
        caller: Some(Caller::user("u")),
        ip: None,
        idempotency_key: None,
        headers: BTreeMap::new(),
        raw_body: None,
    };
    let other_prefix = Keyspace::new(&other).all();
    let before = snapshot(&store);
    let response = engine.handle(&mine, &request);
    let after = snapshot(&store);
    assert!(
        response.usage.fits_within(&bounds),
        "seed {seed}: used {:?}, bounds {bounds:?}\nplan: {plan}\nresponse: {}",
        response.usage,
        response.body
    );
    assert_ne!(
        response.status, 500,
        "seed {seed}: a valid plan hit an internal error: {}\nplan: {plan}",
        response.body
    );
    let in_other = |entries: &[(Vec<u8>, Vec<u8>)]| -> Vec<(Vec<u8>, Vec<u8>)> {
        entries
            .iter()
            .filter(|(k, _)| other_prefix.contains(&aleph_core::keys::Key::from_bytes(k.clone())))
            .cloned()
            .collect()
    };
    assert_eq!(
        in_other(&before),
        in_other(&after),
        "seed {seed}: touched another tenant"
    );
    if response.status >= 400 {
        assert_eq!(
            before, after,
            "seed {seed}: a failed request changed the store"
        );
    }
    VALID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    true
}

proptest! {
    #![proptest_config(ProptestConfig {
        // `PROPTEST_CASES` overrides the default (for long runs).
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(400),
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_plans_stay_within_bounds_and_their_tenant(
        seed: u64,
        n in 0_i64..=50,
        id in 0_i64..=30,
        group in "(a|b|ab|)",
    ) {
        run_seed(seed, n, id, &group);
    }
}

#[test]
#[ignore = "reports how many generated plans validated; run with --ignored after a long run"]
fn report_valid_count() {
    for seed in 0..2000_u64 {
        run_seed(seed, 1, 1, "a");
    }
    println!(
        "{} generated plans validated and ran",
        VALID.load(std::sync::atomic::Ordering::Relaxed)
    );
}

#[test]
fn most_generated_plans_are_valid() {
    // Guards the generator itself: if it mostly produced invalid plans, the properties above
    // would test little.
    let valid = (0..300_u64)
        .filter(|seed| run_seed(*seed, 5, 3, "a"))
        .count();
    assert!(valid >= 150, "only {valid}/300 generated plans validated");
}

/// Mutates a JSON value at a random position: drop a key, replace a value, or swap a number.
fn mutate(value: &mut Json, rng: &mut Rng) {
    match value {
        Json::Object(map) if !map.is_empty() => {
            let keys: Vec<String> = map.keys().cloned().collect();
            let key = rng.pick(&keys).clone();
            match rng.below(4) {
                0 => {
                    map.remove(&key);
                }
                1 => {
                    let replacement = rng
                        .pick(&[
                            json!(null),
                            json!(-1),
                            json!(""),
                            json!([]),
                            json!({}),
                            json!(1e300),
                            json!(i64::MAX),
                        ])
                        .clone();
                    map.insert(key, replacement);
                }
                _ => mutate(map.get_mut(&key).unwrap(), rng),
            }
        }
        Json::Array(items) if !items.is_empty() => {
            let len = items.len() as u64;
            let i = usize::try_from(rng.below(len)).unwrap();
            if rng.chance(25) {
                items.remove(i);
            } else {
                mutate(&mut items[i], rng);
            }
        }
        Json::Number(_) => {
            *value = rng
                .pick(&[json!(0), json!(-7), json!(u32::MAX), json!(i64::MIN)])
                .clone();
        }
        Json::String(s) => {
            *value = json!(
                rng.pick(&[String::new(), format!("{s}x"), "_".into(), "x".repeat(300)])
                    .clone()
            );
        }
        _ => {}
    }
}

#[test]
fn mutated_manifests_never_panic() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases");
    let cases = std::fs::read_dir(dir).unwrap();
    let mut seen = 0;
    for case in cases {
        let path = case.unwrap().path().join("manifest.json");
        let base: Json = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut rng = Rng(7);
        for _ in 0..150 {
            let mut manifest = base.clone();
            for _ in 0..1 + rng.below(3) {
                mutate(&mut manifest, &mut rng);
            }
            if let Ok(valid) = parse_and_validate(&manifest.to_string(), &Limits::ROUTE) {
                // A mutation that still validates must also run without panicking.
                let engine = Engine::new(
                    valid,
                    Arc::new(MemoryStore::new()),
                    Arc::new(FixedEnv::new(1, 1)),
                );
                let tenant = TenantId::new("t").unwrap();
                for route in &engine.manifest().manifest().routes.clone() {
                    let path = route.path.replace(':', "");
                    let _ = engine.handle(
                        &tenant,
                        &Request {
                            method: route.method,
                            path,
                            query: BTreeMap::new(),
                            body: Some(json!({})),
                            caller: Some(Caller::user("u")),
                            ip: None,
                            idempotency_key: None,
                            headers: BTreeMap::new(),
                            raw_body: None,
                        },
                    );
                }
                engine.process(&tenant, 100);
            }
            seen += 1;
        }
    }
    assert!(seen > 0);
}
