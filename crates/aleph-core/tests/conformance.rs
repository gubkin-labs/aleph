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
use aleph_core::{Engine, FixedEnv, Limits, MemoryStore, Request, TenantId, parse_and_validate};
use serde::Deserialize;
use serde_json::Value as Json;

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
struct Step {
    name: String,
    request: Json,
    expect: Expectation,
    #[serde(default)]
    capture: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Expectation {
    status: u16,
    body: Option<Json>,
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
fn scenarios_behave_as_expected_within_bounds() {
    for dir in cases() {
        let valid = parse_and_validate(&read(&dir.join("manifest.json")), &Limits::ROUTE).unwrap();
        let report = valid.report().clone();
        let scenario: Scenario = serde_json::from_str(&read(&dir.join("scenarios.json"))).unwrap();
        let env = Arc::new(FixedEnv::new(0, scenario.seed));
        // The WASM dev engine uses tenant "dev"; ids and responses must match it exactly.
        let tenant = TenantId::new("dev").unwrap();
        let engine = Engine::new(valid, Arc::new(MemoryStore::new()), env.clone());
        let mut vars = BTreeMap::new();

        for (i, step) in scenario.steps.iter().enumerate() {
            let at = format!("{} step {i} {:?}", dir.display(), step.name);
            let offset = i64::try_from(i).unwrap() * scenario.step_ms;
            env.set_now(scenario.start_ms + offset);
            let request: Request =
                serde_json::from_value(substitute(&step.request, &vars)).unwrap();
            let response = engine.handle(&tenant, &request);
            let body = &response.body;
            assert_eq!(response.status, step.expect.status, "{at}: {body}");
            if let Some(expected) = &step.expect.body
                && let Some(mismatch) = subset_mismatch(expected, body, "body")
            {
                panic!("{at}: {mismatch}\n{body}");
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
