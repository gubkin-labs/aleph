//! Model-based property test of counter maintenance: after any sequence of upserts, updates
//! (which may move a row to another group or change its value), and deletes, every kind of
//! counter (count, filtered count, sum, sharded count, min, max, distinct) reads exactly what
//! recomputing it from the live rows gives.
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
use aleph_core::{Engine, FixedEnv, Limits, MemoryStore, Request, TenantId, parse_and_validate};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

fn lit(value: Json) -> Json {
    json!({ "kind": "lit", "value": value })
}

fn body(name: &str) -> Json {
    json!({ "kind": "input", "source": "body", "name": name })
}

fn n_of_row() -> Json {
    json!({ "kind": "field", "of": { "kind": "var", "id": 0 }, "name": "n" })
}

fn route(id: &str, input: &[&str], plan_body: Json, output: Json) -> Json {
    let all = json!({
        "id": { "type": { "kind": "string", "min": 1, "max": 4 } },
        "grp": { "type": { "kind": "string", "min": 1, "max": 4 } },
        "n": { "type": { "kind": "int", "min": 0, "max": 9 } },
    });
    let fields: serde_json::Map<String, Json> = input
        .iter()
        .map(|f| ((*f).to_owned(), all[*f].clone()))
        .collect();
    json!({
        "id": id,
        "method": "POST",
        "path": format!("/{id}"),
        "auth": { "kind": "public" },
        "errors": {},
        "input": { "body": fields, "params": {}, "query": {} },
        "plan": { "body": plan_body, "output": output },
    })
}

const COUNTERS: [&str; 7] = ["count", "big", "total", "sharded", "lo", "hi", "uniq"];

fn manifest() -> String {
    let id = || json!({ "id": body("id") });
    let counter_get = |name: &str| {
        json!({ "kind": "let", "var": 1 + COUNTERS.iter().position(|c| *c == name).unwrap(), "op": {
            "op": "counterGet", "counter": name, "key": [body("grp")],
        } })
    };
    let read_fields: serde_json::Map<String, Json> = COUNTERS
        .iter()
        .enumerate()
        .map(|(i, c)| ((*c).to_owned(), json!({ "kind": "var", "id": i + 1 })))
        .collect();
    let row = |expr: Json| json!({ "var": 0, "expr": expr });
    json!({
        "formatVersion": "0.2",
        "app": { "name": "counters" },
        "auth": { "apiKeyScopes": [], "roles": [] },
        "entities": {
            "Row": {
                "key": ["id"],
                "fields": {
                    "id": { "type": { "kind": "string", "min": 1, "max": 4 } },
                    "grp": { "type": { "kind": "string", "min": 1, "max": 4 } },
                    "n": { "type": { "kind": "int", "min": 0, "max": 9 } },
                },
                "indexes": {},
            }
        },
        "counters": {
            "count": { "of": "Row", "groupBy": ["grp"] },
            "big": { "of": "Row", "groupBy": ["grp"], "where": row(json!({
                "kind": "binary", "op": "gt", "lhs": n_of_row(), "rhs": lit(json!(4)),
            })) },
            "total": { "of": "Row", "groupBy": ["grp"], "sum": row(n_of_row()) },
            "sharded": { "of": "Row", "groupBy": ["grp"], "shards": 4 },
            "lo": { "of": "Row", "groupBy": ["grp"], "agg": { "kind": "min", "field": "n" } },
            "hi": { "of": "Row", "groupBy": ["grp"], "agg": { "kind": "max", "field": "n" } },
            "uniq": { "of": "Row", "groupBy": ["grp"], "agg": { "kind": "distinct", "field": "n" } },
        },
        "routes": [
            route("upsert", &["id", "grp", "n"], json!([{ "kind": "let", "var": 0, "op": {
                "op": "upsert", "entity": "Row",
                "values": { "id": body("id"), "grp": body("grp"), "n": body("n") },
            } }]), lit(json!(1))),
            route("update", &["id", "grp", "n"], json!([{ "kind": "let", "var": 0, "op": {
                "op": "update", "entity": "Row", "key": id(),
                "set": { "grp": body("grp"), "n": body("n") },
            } }]), lit(json!(1))),
            route("delete", &["id"], json!([{ "kind": "let", "var": 0, "op": {
                "op": "delete", "entity": "Row", "key": id(),
            } }]), lit(json!(1))),
            route(
                "read",
                &["grp"],
                Json::Array(COUNTERS.iter().map(|c| counter_get(c)).collect()),
                json!({ "kind": "object", "fields": read_fields }),
            ),
        ],
    })
    .to_string()
}

#[derive(Debug, Clone)]
enum Action {
    Upsert(u8, u8, i64),
    Update(u8, u8, i64),
    Delete(u8),
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        (0u8..5, 0u8..2, 0i64..10).prop_map(|(i, g, n)| Action::Upsert(i, g, n)),
        (0u8..5, 0u8..2, 0i64..10).prop_map(|(i, g, n)| Action::Update(i, g, n)),
        (0u8..5).prop_map(Action::Delete),
    ]
}

fn expected(rows: &BTreeMap<String, (String, i64)>, grp: &str) -> Json {
    let ns: Vec<i64> = rows
        .values()
        .filter(|(g, _)| g == grp)
        .map(|(_, n)| *n)
        .collect();
    let count = ns.len();
    json!({
        "count": count,
        "big": ns.iter().filter(|n| **n > 4).count(),
        "total": ns.iter().sum::<i64>(),
        "sharded": count,
        "lo": ns.iter().min(),
        "hi": ns.iter().max(),
        "uniq": ns.iter().collect::<BTreeSet<_>>().len(),
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn counters_match_the_rows_they_count(actions in proptest::collection::vec(action(), 1..30)) {
        let valid = parse_and_validate(&manifest(), &Limits::ROUTE).unwrap_or_else(|d| panic!("{d:#?}"));
        let engine = Engine::new(valid, Arc::new(MemoryStore::new()), Arc::new(FixedEnv::new(1, 1)));
        let tenant = TenantId::new("t").unwrap();
        let call = |path: &str, body: Json| engine.handle(&tenant, &Request {
            method: Method::Post,
            path: path.into(),
            query: BTreeMap::new(),
            body: Some(body),
            caller: None,
            ip: None,
            idempotency_key: None,
            headers: BTreeMap::new(),
            raw_body: None,
        });
        let groups = ["a", "b"];
        let mut rows: BTreeMap<String, (String, i64)> = BTreeMap::new();
        for action in &actions {
            let r = match action {
                Action::Upsert(i, g, n) => {
                    rows.insert(format!("r{i}"), (groups[usize::from(*g)].to_owned(), *n));
                    call("/upsert", json!({ "id": format!("r{i}"), "grp": groups[usize::from(*g)], "n": n }))
                }
                Action::Update(i, g, n) => {
                    let id = format!("r{i}");
                    if let Some(row) = rows.get_mut(&id) {
                        *row = (groups[usize::from(*g)].to_owned(), *n);
                    }
                    call("/update", json!({ "id": id, "grp": groups[usize::from(*g)], "n": n }))
                }
                Action::Delete(i) => {
                    rows.remove(&format!("r{i}"));
                    call("/delete", json!({ "id": format!("r{i}") }))
                }
            };
            prop_assert!(r.status < 500, "{action:?}: {:?}", r.body);
            for grp in groups {
                let read = call("/read", json!({ "grp": grp }));
                prop_assert_eq!(read.status, 200, "{:?}", read.body);
                prop_assert_eq!(&read.body, &expected(&rows, grp), "group {} after {:?}", grp, actions);
            }
        }
    }
}
