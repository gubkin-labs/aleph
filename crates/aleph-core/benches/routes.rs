//! Interpreter benchmarks on the conformance apps (ticket 026): one request per iteration on a
//! store seeded with realistic data. `cargo bench -p aleph-core`.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::Path;
use std::sync::Arc;

use aleph_core::manifest::Method;
use aleph_core::{
    Caller, Engine, FixedEnv, Limits, MemoryStore, Request, TenantId, parse_and_validate,
};
use criterion::{Criterion, criterion_group, criterion_main};
use serde_json::{Value as Json, json};

fn engine(case: &str) -> Engine {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../conformance/cases/{case}/manifest.json"));
    let valid =
        parse_and_validate(&std::fs::read_to_string(path).unwrap(), &Limits::ROUTE).unwrap();
    Engine::new(
        valid,
        Arc::new(MemoryStore::new()),
        Arc::new(FixedEnv::new(1_700_000_000_000, 1)),
    )
}

fn request(method: Method, path: &str, caller: Caller, body: Option<Json>) -> Request {
    Request {
        method,
        path: path.to_owned(),
        query: BTreeMap::new(),
        body,
        caller: Some(caller),
        ip: None,
        idempotency_key: None,
        headers: BTreeMap::new(),
        raw_body: None,
    }
}

fn owner() -> Caller {
    Caller {
        orgs: BTreeMap::from([("acme".into(), "owner".into())]),
        ..Caller::user("u1")
    }
}

fn benches(c: &mut Criterion) {
    let tenant = TenantId::new("bench").unwrap();

    // projects: a read-heavy list and a create with a unique index and counters.
    let projects = engine("projects");
    for i in 0..200 {
        let body = json!({ "name": format!("Project {i}") });
        projects.handle(
            &tenant,
            &request(Method::Post, "/orgs/acme/projects", owner(), Some(body)),
        );
    }
    c.bench_function("projects/list 20", |b| {
        b.iter(|| {
            let r = projects.handle(
                &tenant,
                &request(Method::Get, "/orgs/acme/projects", owner(), None),
            );
            assert_eq!(r.status, 200, "{}", r.body);
            black_box(r.status)
        });
    });

    // ledger: a transfer reads two accounts, writes two entries and two balances.
    let ledger = engine("ledger");
    let open = |user: &str| {
        let r = ledger.handle(
            &tenant,
            &request(
                Method::Post,
                "/accounts",
                Caller::user(user),
                Some(json!({ "currency": "USD" })),
            ),
        );
        r.body["accountId"].as_str().unwrap().to_owned()
    };
    let (from, to) = (open("u1"), open("u2"));
    ledger.handle(
        &tenant,
        &request(
            Method::Post,
            &format!("/accounts/{from}/deposit"),
            Caller::user("u1"),
            Some(json!({ "amount": 1_000_000 })),
        ),
    );
    let mut key = 0_u64;
    c.bench_function("ledger/transfer", |b| {
        b.iter(|| {
            key += 1;
            let r = ledger.handle(
                &tenant,
                &Request {
                    idempotency_key: Some(format!("k{key}")),
                    ..request(
                        Method::Post,
                        "/transfers",
                        Caller::user("u1"),
                        Some(json!({ "from": from, "to": to, "amount": 1 })),
                    )
                },
            );
            assert_eq!(r.status, 200, "{}", r.body);
            black_box(r.status)
        });
    });

    // commerce: pricing a 5-line order (gets in a map, decimal math, aggregates, objects).
    let commerce = engine("commerce");
    let products: Vec<Json> = (0..5)
        .map(|i| json!({ "sku": format!("S{i}"), "name": "P", "price": "19.99", "category": "books" }))
        .collect();
    commerce.seed(&tenant, "Product", &products).unwrap();
    let items: Vec<Json> = (0..5)
        .map(|i| json!({ "sku": format!("S{i}"), "qty": 2 }))
        .collect();
    let order = json!({ "items": items, "shipTo": { "city": "Paris", "zip": "75001" } });
    c.bench_function("commerce/place order (5 lines)", |b| {
        b.iter(|| {
            let r = commerce.handle(
                &tenant,
                &request(
                    Method::Post,
                    "/orders",
                    Caller::user("c1"),
                    Some(order.clone()),
                ),
            );
            assert_eq!(r.status, 201, "{}", r.body);
            black_box(r.status)
        });
    });
    c.bench_function("commerce/deals (anyOf merge)", |b| {
        b.iter(|| {
            let r = commerce.handle(
                &tenant,
                &request(Method::Get, "/deals", Caller::user("c1"), None),
            );
            assert_eq!(r.status, 200, "{}", r.body);
            black_box(r.status)
        });
    });
}

criterion_group!(routes, benches);
criterion_main!(routes);
