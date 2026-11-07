//! The conformance suite, Rust side. Runs the same golden files as `conformance/test` (which runs
//! them through WASM), natively:
//! 1. every case's `manifest.json` validates, and its analysis equals the golden `bounds.json`
//! 2. every scenario step produces the expected response, with usage inside the route's bounds
//! 3. every `invalid/*.json` manifest is rejected with exactly the expected diagnostic codes
//!
//! Keep `substitute`, `subset_mismatch`, and `value_at` in sync with `conformance/test/scenario.ts`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aleph_core::check::CheckReport;
use aleph_core::runtime::{MemorySecrets, MemorySink, MockIntegrations};
use aleph_core::{
    BlobSigner, Engine, FixedEnv, Limits, MemoryStore, Request, TenantId, parse_and_validate,
};
use serde::Deserialize;
use serde_json::{Value as Json, json};

fn conformance_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    seed: u64,
    start_ms: i64,
    step_ms: i64,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    name: String,
    #[serde(default)]
    advance_ms: i64,
    request: Option<Json>,
    mock: Option<Mock>,
    secret: Option<SecretStep>,
    user: Option<UserStep>,
    seed: Option<SeedStep>,
    #[serde(default)]
    process: bool,
    upload: Option<UploadStep>,
    download: Option<DownloadStep>,
    #[serde(default)]
    expect: Expectation,
    #[serde(default)]
    capture: BTreeMap<String, String>,
}

/// A client uploading a file to a signed URL, through the blob gateway.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadStep {
    url: String,
    size: u64,
    content_type: String,
}

/// A client downloading a file from a signed URL, through the blob gateway.
#[derive(Deserialize)]
struct DownloadStep {
    url: String,
}

#[derive(Deserialize)]
struct SecretStep {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct UserStep {
    id: String,
    profile: serde_json::Map<String, Json>,
}

#[derive(Deserialize)]
struct SeedStep {
    entity: String,
    rows: Vec<Json>,
}

#[derive(Deserialize)]
struct Mock {
    integration: String,
    operation: String,
    response: Option<Json>,
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct Expectation {
    status: Option<u16>,
    body: Option<Json>,
    replayed: Option<bool>,
    report: Option<Json>,
    deliveries: Option<Vec<Json>>,
    calls: Option<Vec<Json>>,
}

fn unordered_mismatch(expected: &[Json], actual: &[Json], path: &str) -> Option<String> {
    if expected.len() != actual.len() {
        return Some(format!(
            "{path}: expected {} items, got {}",
            expected.len(),
            actual.len()
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

fn substitute_str(value: &str, vars: &BTreeMap<String, String>) -> String {
    match substitute(&Json::String(value.to_owned()), vars) {
        Json::String(s) => s,
        _ => unreachable!(),
    }
}

fn substitute(value: &Json, vars: &BTreeMap<String, String>) -> Json {
    match value {
        Json::String(s) => {
            let mut out = s.clone();
            for (name, replacement) in vars {
                out = out.replace(&format!("{{{{{name}}}}}"), replacement);
            }
            assert!(!out.contains("{{"), "unknown scenario variable in {s:?}");
            Json::String(out)
        }
        Json::Array(items) => Json::Array(items.iter().map(|v| substitute(v, vars)).collect()),
        Json::Object(map) => Json::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute(v, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn subset_mismatch(expected: &Json, actual: &Json, path: &str) -> Option<String> {
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
        (Json::Array(_), _) => Some(format!("{path}: expected an array")),
        (Json::Object(e), Json::Object(a)) => e.iter().find_map(|(k, v)| {
            subset_mismatch(v, a.get(k).unwrap_or(&Json::Null), &format!("{path}.{k}"))
        }),
        (Json::Object(_), _) => Some(format!("{path}: expected an object")),
        (e, a) if e == a => None,
        (e, a) => Some(format!("{path}: expected {e}, got {a}")),
    }
}

fn value_at<'a>(value: &'a Json, path: &str) -> Option<&'a Json> {
    path.split('.')
        .try_fold(value, |current, key| match current {
            Json::Object(map) => map.get(key),
            Json::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        })
}

fn cases() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(conformance_dir().join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no conformance cases found");
    dirs
}

#[test]
fn cases_validate_with_golden_bounds() {
    for dir in cases() {
        let valid = parse_and_validate(&read(&dir.join("manifest.json")), &Limits::ROUTE)
            .unwrap_or_else(|d| panic!("{}: {d:#?}", dir.display()));
        let golden: CheckReport = serde_json::from_str(&read(&dir.join("bounds.json"))).unwrap();
        assert_eq!(
            valid.report(),
            &golden,
            "{}: analysis differs from bounds.json",
            dir.display()
        );
    }
}

#[test]
fn route_examples_pass() {
    for dir in cases() {
        let valid = parse_and_validate(&read(&dir.join("manifest.json")), &Limits::ROUTE).unwrap();
        let failures: Vec<_> = aleph_core::examples::run_all(&valid)
            .into_iter()
            .filter(|r| !r.failures.is_empty())
            .collect();
        assert!(failures.is_empty(), "{}: {failures:#?}", dir.display());
    }
}

#[test]
fn scenarios_behave_as_expected_within_bounds() {
    for dir in cases() {
        let valid = parse_and_validate(&read(&dir.join("manifest.json")), &Limits::ROUTE).unwrap();
        let report = valid.report().clone();
        let scenario: Scenario = serde_json::from_str(&read(&dir.join("scenarios.json"))).unwrap();
        let env = Arc::new(FixedEnv::new(0, scenario.seed));
        let sink = Arc::new(MemorySink::new());
        let integrations = Arc::new(MockIntegrations::new());
        let secrets = Arc::new(MemorySecrets::new());
        // The WASM dev engine uses tenant "dev"; ids and responses must match it exactly.
        let tenant = TenantId::new("dev").unwrap();
        let engine = Engine::new(valid, Arc::new(MemoryStore::new()), env.clone())
            .with_sink(sink.clone())
            .with_integrations(integrations.clone())
            .with_secrets(secrets.clone())
            // The same base and key as the WASM dev engine, so URLs match.
            .with_blobs(BlobSigner::new("/_aleph/blobs", b"aleph-dev-blob-signing-key").unwrap());
        let mut vars = BTreeMap::new();
        let mut extra = 0;

        for (i, step) in scenario.steps.iter().enumerate() {
            let at = format!("{} step {i} {:?}", dir.display(), step.name);
            extra += step.advance_ms;
            env.set_now(scenario.start_ms + i64::try_from(i).unwrap() * scenario.step_ms + extra);
            if let Some(secret) = &step.secret {
                secrets.set(&secret.name, &secret.value);
                continue;
            }
            if let Some(user) = &step.user {
                engine.put_user(&tenant, &user.id, &user.profile).unwrap();
                continue;
            }
            if let Some(seed) = &step.seed {
                let rows = substitute(&Json::Array(seed.rows.clone()), &vars);
                let rows = rows.as_array().cloned().unwrap_or_default();
                engine.seed(&tenant, &seed.entity, &rows).unwrap();
                continue;
            }
            if let Some(mock) = &step.mock {
                let response = match &mock.error {
                    Some(error) => Err(error.clone()),
                    None => Ok(mock.response.clone().unwrap_or(Json::Null)),
                };
                integrations.respond(&mock.integration, &mock.operation, response);
                continue;
            }
            if step.process {
                check_process(
                    &engine,
                    &tenant,
                    &report,
                    &sink,
                    &integrations,
                    step,
                    &vars,
                    &at,
                );
                continue;
            }
            if step.upload.is_some() || step.download.is_some() {
                check_blob_step(&engine, step, &vars, &at);
                continue;
            }
            let request = step
                .request
                .as_ref()
                .unwrap_or_else(|| panic!("{at}: no request"));
            let request: Request = serde_json::from_value(substitute(request, &vars)).unwrap();
            let response = engine.handle(&tenant, &request);
            let body = &response.body;
            if let Some(status) = step.expect.status {
                assert_eq!(response.status, status, "{at}: {body}");
            }
            if let Some(expected) = &step.expect.body
                && let Some(mismatch) = subset_mismatch(&substitute(expected, &vars), body, "body")
            {
                panic!("{at}: {mismatch}\n{body}");
            }
            if let Some(replayed) = step.expect.replayed {
                assert_eq!(response.replayed, replayed, "{at}: replayed");
            }
            if let Some(route) = &response.route {
                let bounds = report.routes[route].bounds;
                assert!(
                    response.usage.fits_within(&bounds),
                    "{at}: {:?} over {bounds:?}",
                    response.usage
                );
            }
            let as_json = serde_json::to_value(&response).unwrap();
            for (name, path) in &step.capture {
                let value = value_at(&as_json, path).and_then(Json::as_str);
                vars.insert(
                    name.clone(),
                    value
                        .unwrap_or_else(|| panic!("{at}: capture {path}"))
                        .to_owned(),
                );
            }
        }
    }
}

/// Runs an `upload` or `download` step through the blob gateway, and checks its answer like a
/// response: 200 with the file's metadata, or the gateway's error status.
fn check_blob_step(engine: &Engine, step: &Step, vars: &BTreeMap<String, String>, at: &str) {
    let result = match (&step.upload, &step.download) {
        (Some(upload), _) => engine
            .receive_upload(
                &substitute_str(&upload.url, vars),
                upload.size,
                &upload.content_type,
            )
            .map(|(_, info)| info),
        (_, Some(download)) => engine
            .authorize_download(&substitute_str(&download.url, vars))
            .map(|(_, info)| info),
        _ => unreachable!("not a blob step"),
    };
    // The gateway's answer, as the TS runner sees it.
    let (status, body) = match result {
        Ok(info) => (200, serde_json::to_value(info).unwrap()),
        Err(rejection) => (rejection.status, json!({ "message": rejection.message })),
    };
    if let Some(expected) = step.expect.status {
        assert_eq!(status, expected, "{at}: {body}");
    }
    if let Some(expected) = &step.expect.body
        && let Some(mismatch) = subset_mismatch(&substitute(expected, vars), &body, "body")
    {
        panic!("{at}: {mismatch}\n{body}");
    }
}

/// Runs a `process` step and checks its report, deliveries, and calls, and that every handler
/// run stayed within its bounds.
#[allow(clippy::too_many_arguments)]
fn check_process(
    engine: &Engine,
    tenant: &TenantId,
    report: &CheckReport,
    sink: &MemorySink,
    integrations: &MockIntegrations,
    step: &Step,
    vars: &BTreeMap<String, String>,
    at: &str,
) {
    let processed = engine.process(tenant, 1000);
    for run in &processed.runs {
        let bounds = report.handlers[&run.handler].bounds;
        assert!(
            run.usage.fits_within(&bounds),
            "{at}: {} {:?} over {bounds:?}",
            run.handler,
            run.usage
        );
    }
    let processed = serde_json::to_value(&processed).unwrap();
    if let Some(expected) = &step.expect.report
        && let Some(mismatch) = subset_mismatch(expected, &processed, "report")
    {
        panic!("{at}: {mismatch}\n{processed}");
    }
    let deliveries: Vec<Json> = sink
        .take()
        .into_iter()
        .map(|(_, d)| serde_json::to_value(d).unwrap())
        .collect();
    if let Some(expected) = &step.expect.deliveries {
        let expected: Vec<Json> = expected.iter().map(|e| substitute(e, vars)).collect();
        if let Some(mismatch) = unordered_mismatch(&expected, &deliveries, "deliveries") {
            panic!("{at}: {mismatch}\n{deliveries:?}");
        }
    }
    let calls: Vec<Json> = integrations
        .take_calls()
        .into_iter()
        .map(|c| serde_json::to_value(c).unwrap())
        .collect();
    if let Some(expected) = &step.expect.calls {
        let expected: Vec<Json> = expected.iter().map(|e| substitute(e, vars)).collect();
        if let Some(mismatch) = unordered_mismatch(&expected, &calls, "calls") {
            panic!("{at}: {mismatch}\n{calls:?}");
        }
    }
}

#[test]
fn invalid_manifests_are_rejected_with_expected_codes() {
    #[derive(Deserialize)]
    struct Invalid {
        expect: Vec<String>,
        manifest: Json,
    }
    let mut checked = 0;
    for entry in fs::read_dir(conformance_dir().join("invalid")).unwrap() {
        let path = entry.unwrap().path();
        let case: Invalid = serde_json::from_str(&read(&path)).unwrap();
        let diagnostics = parse_and_validate(&case.manifest.to_string(), &Limits::ROUTE)
            .err()
            .unwrap_or_else(|| panic!("{} should be rejected", path.display()));
        let codes: BTreeSet<String> = diagnostics
            .iter()
            .map(|d| {
                serde_json::to_value(d.code)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let expected: BTreeSet<String> = case.expect.into_iter().collect();
        assert_eq!(codes, expected, "{}: {diagnostics:#?}", path.display());
        checked += 1;
    }
    assert!(checked > 0);
}

/// Applies `{ "set": path, "value": v }` and `{ "delete": path }` edits to a JSON value.
fn apply_edits(target: &mut Json, edits: Option<&Json>) {
    for edit in edits.and_then(Json::as_array).into_iter().flatten() {
        let (path, set) = match (edit.get("set"), edit.get("delete")) {
            (Some(path), _) => (path, true),
            (_, Some(path)) => (path, false),
            _ => panic!("bad edit {edit}"),
        };
        let keys: Vec<&str> = path
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        let (last, parents) = keys.split_last().unwrap();
        let mut node = &mut *target;
        for key in parents {
            node = node.get_mut(*key).unwrap();
        }
        let map = node.as_object_mut().unwrap();
        if set {
            map.insert((*last).to_owned(), edit["value"].clone());
        } else {
            map.remove(*last);
        }
    }
}

#[test]
fn schema_evolution_is_classified() {
    let spec: Json =
        serde_json::from_str(&read(&conformance_dir().join("evolution.json"))).unwrap();
    let base: Json = serde_json::from_str(&read(
        &conformance_dir().join(spec["base"].as_str().unwrap()),
    ))
    .unwrap();
    for case in spec["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut deployed = base.clone();
        apply_edits(&mut deployed, case.get("base_edits"));
        let mut next = deployed.clone();
        apply_edits(&mut next, case.get("edits"));
        let old: aleph_core::Manifest = serde_json::from_value(deployed).unwrap();
        let new: aleph_core::Manifest = serde_json::from_value(next).unwrap();
        let report = aleph_core::evolution::compare(&old, &new);
        assert_eq!(
            report.deployable(),
            case["deployable"].as_bool().unwrap(),
            "{name}: {report:#?}"
        );
        let kinds: BTreeMap<String, Json> = report
            .changes
            .iter()
            .map(|c| (c.path.clone(), serde_json::to_value(c.kind).unwrap()))
            .collect();
        let expected: BTreeMap<String, Json> =
            serde_json::from_value(case["expect"].clone()).unwrap();
        assert_eq!(kinds, expected, "{name}");
        if let Some(migration) = case.get("migration") {
            assert_eq!(
                &serde_json::to_value(&report.migration).unwrap(),
                migration,
                "{name}"
            );
        }
    }
}
