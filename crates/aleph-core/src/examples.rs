//! Running route examples (`aleph test`): each example gets a fresh store, deterministic time
//! and ids, its `given` rows and users, and one request. The response must match, and what the
//! request actually used must stay within the route's static bounds.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::check::ValidatedManifest;
use crate::keys::TenantId;
use crate::manifest::{Route, RouteExample};
use crate::runtime::{
    Caller, Engine, FixedEnv, MemorySecrets, MemorySink, MockIntegrations, Request,
};
use crate::store::MemoryStore;

/// The time examples run at: 2023-11-14T22:13:20Z.
pub const EXAMPLE_NOW_MS: i64 = 1_700_000_000_000;

/// One example's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExampleResult {
    /// The route id.
    pub route: String,
    /// The example's name.
    pub name: String,
    /// What didn't match; empty if it passed.
    pub failures: Vec<String>,
    /// The response status.
    pub status: u16,
    /// The response body.
    #[cfg_attr(feature = "codegen", ts(type = "unknown"))]
    pub body: Json,
}

/// Runs every route's examples.
pub fn run_all(manifest: &ValidatedManifest) -> Vec<ExampleResult> {
    manifest
        .manifest()
        .routes
        .iter()
        .flat_map(|route| {
            route
                .examples
                .iter()
                .map(move |example| run(manifest, route, example))
        })
        .collect()
}

#[allow(
    clippy::too_many_lines,
    reason = "setup, request, and each kind of expectation"
)]
fn run(manifest: &ValidatedManifest, route: &Route, example: &RouteExample) -> ExampleResult {
    let mut failures = Vec::new();
    let env = Arc::new(FixedEnv::new(EXAMPLE_NOW_MS, 1));
    let sink = Arc::new(MemorySink::new());
    let integrations = Arc::new(MockIntegrations::new());
    let secrets = Arc::new(MemorySecrets::new());
    for (name, value) in &example.secrets {
        secrets.set(name, value);
    }
    let engine = Engine::new(manifest.clone(), Arc::new(MemoryStore::new()), env.clone())
        .with_sink(sink.clone())
        .with_integrations(integrations.clone())
        .with_secrets(secrets);
    let Ok(tenant) = TenantId::new("example") else {
        return ExampleResult {
            route: route.id.clone(),
            name: example.name.clone(),
            failures: vec!["internal: invalid example tenant".into()],
            status: 0,
            body: Json::Null,
        };
    };
    for mock in &example.mocks {
        let field = |name: &str| mock.get(name).and_then(Json::as_str).unwrap_or_default();
        let response = match mock.get("error").and_then(Json::as_str) {
            Some(error) => Err(error.to_owned()),
            None => Ok(mock.get("response").cloned().unwrap_or(Json::Null)),
        };
        integrations.respond(field("integration"), field("operation"), response);
    }
    for (user, profile) in &example.users {
        let empty = serde_json::Map::new();
        let profile = profile.as_object().unwrap_or(&empty);
        if let Err(e) = engine.put_user(&tenant, user, profile) {
            failures.push(format!("users.{user}: {e}"));
        }
    }
    for (entity, rows) in &example.given {
        if let Err(e) = engine.seed(&tenant, entity, rows) {
            failures.push(format!("given.{entity}: {e}"));
        }
    }
    // Seeding may have queued change handlers; they're part of the setup, not the example.
    engine.process(&tenant, 1000);
    let _ = sink.take();
    let caller = match &example.caller {
        None => None,
        Some(json) => match serde_json::from_value::<Caller>(json.clone()) {
            Ok(caller) => Some(caller),
            Err(e) => {
                failures.push(format!("caller: {e}"));
                None
            }
        },
    };
    let path = route
        .path
        .split('/')
        .map(|segment| match segment.strip_prefix(':') {
            Some(param) => example
                .request
                .params
                .get(param)
                .cloned()
                .unwrap_or_else(|| {
                    failures.push(format!("request.params.{param} is missing"));
                    String::new()
                }),
            None => segment.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("/");
    let raw_body = route
        .webhook
        .as_ref()
        .and(example.request.body.as_ref())
        .map(Json::to_string);
    let request = Request {
        method: route.method,
        path,
        query: example.request.query.clone(),
        body: if raw_body.is_some() {
            None
        } else {
            example.request.body.clone()
        },
        caller,
        ip: None,
        idempotency_key: example.request.headers.get("idempotency-key").cloned(),
        headers: example.request.headers.clone(),
        raw_body,
    };
    let response = engine.handle(&tenant, &request);
    let expected_status = example.expect.status.unwrap_or(200);
    if response.status != expected_status {
        failures.push(format!(
            "status: expected {expected_status}, got {} ({})",
            response.status, response.body
        ));
    }
    if let Some(body) = &example.expect.body
        && let Some(mismatch) = subset_mismatch(body, &response.body, "body")
    {
        failures.push(mismatch);
    }
    if let Some(bounds) = manifest.report().routes.get(&route.id).map(|r| r.bounds)
        && !response.usage.fits_within(&bounds)
    {
        failures.push(format!(
            "used {:?}, over the static bounds {bounds:?}",
            response.usage
        ));
    }
    if example.process {
        let report = engine.process(&tenant, 1000);
        failures.extend(
            report
                .failures
                .iter()
                .map(|f| format!("async work failed: {f}")),
        );
        if let Some(runs) = &example.expect.runs {
            let mut actual: Vec<String> = report.runs.iter().map(|r| r.handler.clone()).collect();
            let mut expected = runs.clone();
            actual.sort();
            expected.sort();
            if actual != expected {
                failures.push(format!("runs: expected {expected:?}, got {actual:?}"));
            }
        }
        if let Some(deliveries) = &example.expect.deliveries {
            let actual: Vec<Json> = sink
                .take()
                .into_iter()
                .map(|(_, d)| serde_json::to_value(d).unwrap_or(Json::Null))
                .collect();
            if let Some(mismatch) = unordered_mismatch(deliveries, &actual, "deliveries") {
                failures.push(mismatch);
            }
        }
    } else if example.expect.runs.is_some() || example.expect.deliveries.is_some() {
        failures.push("expect.runs and expect.deliveries need process: true".into());
    }
    ExampleResult {
        route: route.id.clone(),
        name: example.name.clone(),
        failures,
        status: response.status,
        body: response.body,
    }
}

/// Describes the first difference, or `None` if `expected` is a subset of `actual`: objects
/// match by the listed keys, arrays item by item, everything else by equality.
pub fn subset_mismatch(expected: &Json, actual: &Json, path: &str) -> Option<String> {
    match (expected, actual) {
        (Json::Array(e), Json::Array(a)) => {
            if e.len() != a.len() {
                return Some(format!(
                    "{path}: expected {} items, got {}",
                    e.len(),
                    a.len()
                ));
            }
            e.iter()
                .zip(a)
                .enumerate()
                .find_map(|(i, (e, a))| subset_mismatch(e, a, &format!("{path}.{i}")))
        }
        (Json::Array(_), _) => Some(format!("{path}: expected an array, got {actual}")),
        (Json::Object(e), Json::Object(a)) => e.iter().find_map(|(k, v)| {
            subset_mismatch(v, a.get(k).unwrap_or(&Json::Null), &format!("{path}.{k}"))
        }),
        (Json::Object(_), _) => Some(format!("{path}: expected an object, got {actual}")),
        (e, a) if e == a => None,
        (e, a) => Some(format!("{path}: expected {e}, got {a}")),
    }
}

/// Like [`subset_mismatch`] for lists in any order: each expected item matches a distinct one.
pub fn unordered_mismatch(expected: &[Json], actual: &[Json], path: &str) -> Option<String> {
    if expected.len() != actual.len() {
        return Some(format!(
            "{path}: expected {} items, got {}: {}",
            expected.len(),
            actual.len(),
            Json::Array(actual.to_vec())
        ));
    }
    let mut remaining: Vec<&Json> = actual.iter().collect();
    for (i, item) in expected.iter().enumerate() {
        let Some(index) = remaining
            .iter()
            .position(|c| subset_mismatch(item, c, "").is_none())
        else {
            return Some(format!("{path}.{i}: no match for {item}"));
        };
        remaining.remove(index);
    }
    None
}

/// Runs examples and groups failures by route, for reports.
pub fn summarize(results: &[ExampleResult]) -> BTreeMap<String, (usize, usize)> {
    let mut out: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for result in results {
        let entry = out.entry(result.route.clone()).or_default();
        if result.failures.is_empty() {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }
    out
}
