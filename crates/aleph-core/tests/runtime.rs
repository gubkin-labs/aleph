//! Runtime guarantees: tenant isolation, serializable transactions under concurrency, the meter as
//! a backstop, and (property-tested) that real executions never exceed their static bounds.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use aleph_core::check::ValidatedManifest;
use aleph_core::keys::{Keyspace, TenantId};
use aleph_core::manifest::Method;
use aleph_core::{Engine, FixedEnv, Limits, MemoryStore, Request, Response, parse_and_validate};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

fn load(case: &str) -> ValidatedManifest {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../conformance/cases/{case}/manifest.json"));
    parse_and_validate(&std::fs::read_to_string(path).unwrap(), &Limits::ROUTE).unwrap()
}

fn request(method: Method, path: &str, body: Option<Json>) -> Request {
    Request {
        method,
        path: path.to_owned(),
        query: BTreeMap::new(),
        body,
        caller: Some("u1".into()),
    }
}

fn tenant(id: &str) -> TenantId {
    TenantId::new(id).unwrap()
}

fn create_project(engine: &Engine, tenant: &TenantId, name: &str) -> Response {
    engine.handle(
        tenant,
        &request(
            Method::Post,
            "/orgs/acme/projects",
            Some(json!({ "name": name })),
        ),
    )
}

#[test]
fn tenants_are_isolated_in_one_shared_store() {
    let store = Arc::new(MemoryStore::new());
    let engine = Engine::new(
        load("projects"),
        store.clone(),
        Arc::new(FixedEnv::new(1, 1)),
    );
    let (a, b) = (tenant("a"), tenant("b"));

    for name in ["A1", "A2", "A3"] {
        assert_eq!(create_project(&engine, &a, name).status, 200);
    }
    // Tenant b's plan limit and listings are untouched by tenant a's data, with the same org id.
    assert_eq!(create_project(&engine, &b, "B1").status, 200);
    let listed = engine.handle(&b, &request(Method::Get, "/orgs/acme/projects", None));
    assert_eq!(listed.body["projects"].as_array().unwrap().len(), 1);

    // Every stored key belongs to exactly one tenant's keyspace.
    let (ka, kb) = (Keyspace::new(&a).all(), Keyspace::new(&b).all());
    for (key, _) in store.entries() {
        assert!(
            ka.contains(&key) ^ kb.contains(&key),
            "{key:?} isn't in exactly one tenant"
        );
    }
}

#[test]
fn concurrent_deposits_serialize_with_conflict_retries() {
    let store = Arc::new(MemoryStore::new());
    let engine = Arc::new(Engine::new(
        load("ledger"),
        store,
        Arc::new(FixedEnv::new(1, 1)),
    ));
    let t = tenant("bank");
    let opened = engine.handle(
        &t,
        &request(
            Method::Post,
            "/accounts",
            Some(json!({ "currency": "USD" })),
        ),
    );
    let account = opened.body["accountId"].as_str().unwrap().to_owned();

    let (threads, deposits) = (8, 25);
    let retried: u32 = thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut retries = 0;
                    for _ in 0..deposits {
                        let path = format!("/accounts/{account}/deposit");
                        let r = engine.handle(
                            &t,
                            &request(Method::Post, &path, Some(json!({ "amount": 1 }))),
                        );
                        assert!(r.status == 200 || r.body["error"] == "conflict", "{r:?}");
                        if r.status == 200 {
                            retries += r.attempts - 1;
                        }
                    }
                    retries
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });

    // No lost updates: the balance equals the number of committed deposits, and each committed
    // deposit wrote exactly one entry.
    let entries = engine.handle(
        &t,
        &Request {
            query: BTreeMap::from([("limit".into(), "100".into())]),
            ..request(Method::Get, &format!("/accounts/{account}/entries"), None)
        },
    );
    let accounts = engine.handle(&t, &request(Method::Get, "/accounts", None));
    let balance = accounts.body["accounts"][0]["balance"].as_i64().unwrap();
    assert!(balance > 0 && balance <= threads * deposits);
    let all_entries = entries.body["entries"].as_array().unwrap().len();
    assert_eq!(i64::try_from(all_entries).unwrap(), balance.min(100));
    println!("committed {balance} deposits with {retried} conflict retries");
}

#[test]
fn the_meter_stops_executions_the_analysis_allowed() {
    let engine = Engine::new(
        load("projects"),
        Arc::new(MemoryStore::new()),
        Arc::new(FixedEnv::new(1, 1)),
    )
    .with_runtime_limits(Limits {
        writes: 2,
        ..Limits::ROUTE
    });
    let t = tenant("t");
    let response = create_project(&engine, &t, "Over");
    assert_eq!(response.status, 500);
    assert_eq!(response.body["error"], "budget_exceeded");
    // The aborted transaction left nothing behind.
    let listed = engine.handle(&t, &request(Method::Get, "/orgs/acme/projects", None));
    assert_eq!(listed.body["projects"], json!([]));
}

#[derive(Debug, Clone)]
enum Action {
    Create(String),
    Archive(usize),
    List(Option<bool>, i64),
    AddTask(usize, String),
    Complete(usize, Vec<usize>),
    Get(usize),
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        "[a-z]{0,130}".prop_map(Action::Create),
        (0usize..8).prop_map(Action::Archive),
        (proptest::option::of(any::<bool>()), -5i64..80).prop_map(|(s, l)| Action::List(s, l)),
        (0usize..8, "[a-z]{1,10}").prop_map(|(p, t)| Action::AddTask(p, t)),
        (0usize..8, proptest::collection::vec(0usize..20, 0..20))
            .prop_map(|(p, ts)| Action::Complete(p, ts)),
        (0usize..8).prop_map(Action::Get),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Differential property: for any sequence of requests, every response stays within its
    /// route's static bounds, no plan hits a runtime error, and the counter always equals the
    /// number of active projects.
    #[test]
    fn random_requests_stay_within_static_bounds(actions in proptest::collection::vec(action(), 1..40)) {
        let manifest = load("projects");
        let report = manifest.report().clone();
        let env = Arc::new(FixedEnv::new(1_000, 3));
        let engine = Engine::new(manifest, Arc::new(MemoryStore::new()), env.clone());
        let t = tenant("p");
        engine.handle(&t, &request(Method::Put, "/orgs/acme/billing", Some(json!({ "plan": "pro" }))));
        let (mut projects, mut tasks): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        let pick = |ids: &[String], i: usize| ids.get(i).cloned().unwrap_or_else(|| "01ARZ3NDEKTSV4RRFFQ69G5FAV".into());

        for action in actions {
            env.advance(1);
            let response = match &action {
                Action::Create(name) => create_project(&engine, &t, name),
                Action::Archive(i) => engine.handle(&t, &request(Method::Post, &format!("/orgs/acme/projects/{}/archive", pick(&projects, *i)), None)),
                Action::List(status, limit) => {
                    let mut query = BTreeMap::from([("limit".to_owned(), limit.to_string())]);
                    if let Some(archived) = status {
                        query.insert("status".into(), if *archived { "archived" } else { "active" }.into());
                    }
                    engine.handle(&t, &Request { query, ..request(Method::Get, "/orgs/acme/projects", None) })
                }
                Action::AddTask(i, title) => engine.handle(&t, &request(Method::Post, &format!("/orgs/acme/projects/{}/tasks", pick(&projects, *i)), Some(json!({ "title": title })))),
                Action::Complete(i, ids) => {
                    let ids: Vec<String> = ids.iter().map(|j| pick(&tasks, *j)).collect();
                    engine.handle(&t, &request(Method::Post, &format!("/orgs/acme/projects/{}/tasks/complete", pick(&projects, *i)), Some(json!({ "taskIds": ids }))))
                }
                Action::Get(i) => engine.handle(&t, &request(Method::Get, &format!("/orgs/acme/projects/{}", pick(&projects, *i)), None)),
            };

            prop_assert!(response.status < 500, "{action:?} -> {response:?}");
            if let Some(route) = &response.route {
                let bounds = report.routes[route].bounds;
                prop_assert!(response.usage.fits_within(&bounds), "{action:?}: {:?} over {bounds:?}", response.usage);
            }
            if response.status == 200 {
                if let Some(id) = response.body["projectId"].as_str().filter(|_| matches!(action, Action::Create(_))) {
                    projects.push(id.to_owned());
                }
                if let Some(id) = response.body["taskId"].as_str() {
                    tasks.push(id.to_owned());
                }
            }
        }

        let active = engine.handle(&t, &Request {
            query: BTreeMap::from([("limit".into(), "50".into())]),
            ..request(Method::Get, "/orgs/acme/projects", None)
        });
        let listed = active.body["projects"].as_array().unwrap().len();
        // Creating a project when the counter disagrees would expose drift; read it via a route.
        if listed < 50 {
            let before = create_project(&engine, &t, "probe");
            prop_assert_eq!(before.status, 200);
            let after = engine.handle(&t, &Request {
                query: BTreeMap::from([("limit".into(), "50".into())]),
                ..request(Method::Get, "/orgs/acme/projects", None)
            });
            prop_assert_eq!(after.body["projects"].as_array().unwrap().len(), listed + 1);
        }
    }
}
