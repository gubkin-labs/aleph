//! Shared pieces of the aleph-core fuzz targets: structure-aware manifest mutation, request
//! generation from a route's declared inputs, and the execution invariants.
//!
//! The execution invariants are the ones `crates/aleph-core/tests/generated_plans.rs` asserts,
//! checked after every request and every `process` call:
//! 1. nothing panics;
//! 2. actual usage never exceeds the static bounds, on every resource (and the runtime meter
//!    never stops a validated route);
//! 3. only the executing tenant's keys change;
//! 4. a failed request (status ≥ 400) leaves the store exactly as it was.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use aleph_core::check::CheckReport;
use aleph_core::keys::{Key, Keyspace};
use aleph_core::manifest::{Auth, FieldType, Method, Route};
use aleph_core::runtime::{ApiKey, MemorySecrets, MemorySink, MockIntegrations};
use aleph_core::{
    Caller, Cost, Engine, FixedEnv, Limits, MemoryStore, Request, Response, TenantId,
    ValidatedManifest, parse_and_validate,
};
use arbitrary::{Result, Unstructured};
use serde_json::{Map, Value as Json, json};

/// A golden conformance case: its manifest and scenario, embedded at build time.
pub struct Case {
    /// The case name (its directory under `conformance/cases`).
    pub name: &'static str,
    /// `manifest.json`.
    pub manifest: &'static str,
    /// `scenarios.json`.
    pub scenarios: &'static str,
}

macro_rules! case {
    ($name:literal) => {
        Case {
            name: $name,
            manifest: include_str!(concat!("../../conformance/cases/", $name, "/manifest.json")),
            scenarios: include_str!(concat!(
                "../../conformance/cases/",
                $name,
                "/scenarios.json"
            )),
        }
    };
}

/// Every golden case. Keep in sync with `conformance/cases`.
pub const CASES: &[Case] = &[
    case!("commerce"),
    case!("fulfillment"),
    case!("ledger"),
    case!("pipeline"),
    case!("projects"),
];

/// The parsed manifests of [`CASES`], and the strings they and their scenarios contain.
struct Parsed {
    manifests: Vec<Json>,
    scenarios: Vec<Json>,
    pools: Vec<Vec<String>>,
}

fn parsed() -> &'static Parsed {
    static PARSED: OnceLock<Parsed> = OnceLock::new();
    PARSED.get_or_init(|| {
        let manifests: Vec<Json> = CASES
            .iter()
            .map(|c| serde_json::from_str(c.manifest).unwrap_or(Json::Null))
            .collect();
        let scenarios: Vec<Json> = CASES
            .iter()
            .map(|c| serde_json::from_str(c.scenarios).unwrap_or(Json::Null))
            .collect();
        let pools = manifests
            .iter()
            .zip(&scenarios)
            .map(|(m, s)| {
                let mut pool = Vec::new();
                collect_strings(m, &mut pool);
                collect_strings(s, &mut pool);
                pool.sort();
                pool.dedup();
                pool
            })
            .collect();
        Parsed {
            manifests,
            scenarios,
            pools,
        }
    })
}

fn collect_strings(value: &Json, out: &mut Vec<String>) {
    match value {
        Json::String(s) if s.len() <= 48 => out.push(s.clone()),
        Json::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        Json::Object(map) => {
            for (k, v) in map {
                out.push(k.clone());
                collect_strings(v, out);
            }
        }
        _ => {}
    }
}

/// The golden manifest of case `index`, as JSON.
pub fn golden(index: usize) -> &'static Json {
    &parsed().manifests[index]
}

/// The strings of case `index`: names, enum values, ids, roles, and scenario data.
pub fn pool(index: usize) -> &'static [String] {
    &parsed().pools[index]
}

fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// Every JSON pointer in `value`, the root included.
fn pointers(value: &Json, at: &mut String, out: &mut Vec<String>) {
    out.push(at.clone());
    let len = at.len();
    match value {
        Json::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                at.push('/');
                at.push_str(&i.to_string());
                pointers(item, at, out);
                at.truncate(len);
            }
        }
        Json::Object(map) => {
            for (k, v) in map {
                at.push('/');
                at.push_str(&escape(k));
                pointers(v, at, out);
                at.truncate(len);
            }
        }
        _ => {}
    }
}

const NUMBERS: &[i64] = &[
    0,
    1,
    -1,
    2,
    3,
    7,
    8,
    64,
    100,
    255,
    256,
    1000,
    65_535,
    1 << 31,
    (1 << 31) - 1,
    -(1 << 31),
    u32::MAX as i64,
    1 << 53,
    i64::MAX,
    i64::MIN,
    i64::MAX - 1,
];

fn number(u: &mut Unstructured<'_>, around: Option<i64>) -> Result<Json> {
    Ok(match u.int_in_range(0..=5)? {
        0 => json!(u.arbitrary::<i64>()?),
        1 => json!(u.arbitrary::<f64>()?),
        2 => match around {
            Some(n) => json!(n.saturating_add(i64::from(u.int_in_range(-2_i8..=2)?))),
            None => json!(*u.choose(NUMBERS)?),
        },
        3 => json!(u64::MAX - u64::from(u.arbitrary::<u8>()?)),
        _ => json!(*u.choose(NUMBERS)?),
    })
}

/// A small arbitrary JSON value, for replacing a subtree.
fn small(u: &mut Unstructured<'_>, pool: &[String]) -> Result<Json> {
    Ok(match u.int_in_range(0..=8)? {
        0 => Json::Null,
        1 => json!(u.arbitrary::<bool>()?),
        2 => number(u, None)?,
        3 => json!([]),
        4 => json!({}),
        5 => json!(u.arbitrary::<String>()?),
        6 => json!("x".repeat(usize::from(u.arbitrary::<u8>()?) * 4)),
        _ => match u.choose(pool) {
            Ok(s) => json!(s),
            Err(_) => json!(""),
        },
    })
}

/// Applies one structure-aware mutation to `root`: replace a scalar with a nearby or
/// interesting one, swap in a string the manifest uses elsewhere, graft another subtree of the
/// manifest, drop, duplicate, or swap elements, or rename a key.
pub fn mutate(root: &mut Json, u: &mut Unstructured<'_>, pool: &[String]) -> Result<()> {
    let mut all = Vec::new();
    pointers(root, &mut String::new(), &mut all);
    let target = u.choose(&all)?.clone();
    let op = u.int_in_range(0..=9)?;
    let graft = if op <= 1 {
        let source = u.choose(&all)?;
        root.pointer(source).cloned()
    } else {
        None
    };
    let replacement = if op == 2 { Some(small(u, pool)?) } else { None };
    let Some(node) = root.pointer_mut(&target) else {
        return Ok(());
    };
    if let Some(graft) = graft {
        *node = graft;
        return Ok(());
    }
    if let Some(replacement) = replacement {
        *node = replacement;
        return Ok(());
    }
    match node {
        Json::Number(n) => {
            let around = n.as_i64();
            *node = number(u, around)?;
        }
        Json::String(s) => {
            *s = match u.int_in_range(0..=4)? {
                0 => format!("{s}{}", u.arbitrary::<char>()?),
                1 => s.chars().skip(1).collect(),
                _ => u.choose(pool).cloned().unwrap_or_default(),
            };
        }
        Json::Bool(b) => *b = !*b,
        Json::Array(items) if !items.is_empty() => match op {
            3..=5 => {
                let i = u.choose_index(items.len())?;
                items.remove(i);
            }
            6 | 7 => {
                let i = u.choose_index(items.len())?;
                let copy = items[i].clone();
                let at = u.choose_index(items.len() + 1)?;
                items.insert(at, copy);
            }
            _ => {
                let (i, j) = (u.choose_index(items.len())?, u.choose_index(items.len())?);
                items.swap(i, j);
            }
        },
        Json::Object(map) if !map.is_empty() => {
            let keys: Vec<String> = map.keys().cloned().collect();
            let key = u.choose(&keys)?.clone();
            if op <= 6 {
                map.remove(&key);
            } else {
                let renamed = u.choose(pool).cloned().unwrap_or_default();
                if let Some(value) = map.remove(&key) {
                    map.insert(renamed, value);
                }
            }
        }
        node => *node = small(u, pool)?,
    }
    Ok(())
}

/// A golden manifest with up to `max` structure-aware mutations, chosen by the fuzzer. Returns
/// the case index and the mutated manifest.
pub fn mutated_manifest(u: &mut Unstructured<'_>, max: u8) -> Result<(usize, Json)> {
    let case = u.choose_index(CASES.len())?;
    let mut manifest = golden(case).clone();
    for _ in 0..u.int_in_range(0..=max)? {
        mutate(&mut manifest, u, pool(case))?;
    }
    Ok((case, manifest))
}

/// Validates a manifest and checks what the validator promises about its result: an accepted
/// manifest has a report entry, with bounds within the limits, for every route, and a rejected
/// one has at least one diagnostic.
pub fn validate_checked(text: &str) -> Option<ValidatedManifest> {
    match parse_and_validate(text, &Limits::ROUTE) {
        Ok(valid) => {
            let report = valid.report();
            let limits = limits_as_cost(&Limits::ROUTE);
            for route in &valid.manifest().routes {
                let Some(entry) = report.routes.get(&route.id) else {
                    panic!("route {} validated without a report entry", route.id);
                };
                assert!(
                    entry.bounds.fits_within(&limits),
                    "route {} validated with bounds over the limits: {:?}",
                    route.id,
                    entry.bounds
                );
            }
            Some(valid)
        }
        Err(diagnostics) => {
            assert!(!diagnostics.is_empty(), "rejected without a diagnostic");
            None
        }
    }
}

fn limits_as_cost(limits: &Limits) -> Cost {
    Cost {
        reads: limits.reads,
        writes: limits.writes,
        ops: limits.ops,
        bytes_read: limits.bytes_read,
        effects: limits.effects,
        calls: limits.calls,
        memory: limits.memory,
    }
}

// ---------------------------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------------------------

/// An engine over a fresh store, with another tenant's data already in it.
pub struct Harness {
    engine: Engine,
    store: Arc<MemoryStore>,
    env: Arc<FixedEnv>,
    integrations: Arc<MockIntegrations>,
    secrets: Arc<MemorySecrets>,
    sink: Arc<MemorySink>,
    report: CheckReport,
    mine: TenantId,
    pool: &'static [String],
    case: usize,
    vars: BTreeMap<String, String>,
}

const START_MS: i64 = 1_700_000_000_000;

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap_or_else(|e| panic!("bad tenant id {name}: {e:?}"))
}

/// The store contents after running case `index`'s golden scenario as tenant `other`: data
/// every fuzzed execution must leave untouched.
fn other_tenant_data(index: usize) -> &'static [(Key, Vec<u8>)] {
    static DATA: OnceLock<Vec<Vec<(Key, Vec<u8>)>>> = OnceLock::new();
    &DATA.get_or_init(|| {
        (0..CASES.len())
            .map(|i| {
                let Ok(valid) = parse_and_validate(CASES[i].manifest, &Limits::ROUTE) else {
                    return Vec::new();
                };
                let mut harness = Harness::with_tenant(valid, i, tenant("other"), &[]);
                harness.replay_scenario(usize::MAX, false);
                harness.store.entries()
            })
            .collect()
    })[index]
}

impl Harness {
    fn with_tenant(
        valid: ValidatedManifest,
        case: usize,
        mine: TenantId,
        initial: &[(Key, Vec<u8>)],
    ) -> Self {
        let report = valid.report().clone();
        let store = Arc::new(MemoryStore::new());
        store.restore(initial.to_vec());
        let seed = parsed().scenarios[case]["seed"].as_u64().unwrap_or(1);
        let env = Arc::new(FixedEnv::new(START_MS, seed));
        let integrations = Arc::new(MockIntegrations::new());
        let secrets = Arc::new(MemorySecrets::new());
        let sink = Arc::new(MemorySink::new());
        let engine = Engine::new(valid, store.clone(), env.clone())
            .with_sink(sink.clone())
            .with_integrations(integrations.clone())
            .with_secrets(secrets.clone());
        Self {
            engine,
            store,
            env,
            integrations,
            secrets,
            sink,
            report,
            mine,
            pool: pool(case),
            case,
            vars: BTreeMap::new(),
        }
    }

    /// A harness for a validated (possibly mutated) manifest of case `case`, running as tenant
    /// `mine`, with tenant `other`'s golden data already in the store.
    pub fn new(valid: ValidatedManifest, case: usize) -> Self {
        Self::with_tenant(valid, case, tenant("mine"), other_tenant_data(case))
    }

    fn snapshot(&self) -> Vec<(Key, Vec<u8>)> {
        self.store.entries()
    }

    /// Checks that only `mine`'s keys changed between `before` and the store now, and returns
    /// whether anything changed.
    fn check_isolation(&self, before: &[(Key, Vec<u8>)], what: &str) -> bool {
        let after = self.snapshot();
        let own = Keyspace::new(&self.mine).all();
        let before_map: BTreeMap<&Key, &Vec<u8>> = before.iter().map(|(k, v)| (k, v)).collect();
        let after_map: BTreeMap<&Key, &Vec<u8>> = after.iter().map(|(k, v)| (k, v)).collect();
        let mut changed = false;
        for key in before_map.keys().chain(after_map.keys()) {
            if before_map.get(key) != after_map.get(key) {
                changed = true;
                assert!(
                    own.contains(key),
                    "{what} changed a key outside its tenant: {:?}",
                    String::from_utf8_lossy(key.as_bytes())
                );
            }
        }
        changed
    }

    /// Runs one request and checks every invariant.
    pub fn request(&mut self, request: &Request) -> Response {
        let before = self.snapshot();
        let response = self.engine.handle(&self.mine, request);
        let what = format!(
            "{} {} (status {}, body {})",
            request.method.as_str(),
            request.path,
            response.status,
            response.body
        );
        if let Some(route) = &response.route {
            let Some(entry) = self.report.routes.get(route) else {
                panic!("{what}: ran route {route}, which has no report entry");
            };
            assert!(
                response.usage.fits_within(&entry.bounds),
                "{what}: usage {:?} over the static bounds {:?}",
                response.usage,
                entry.bounds
            );
        }
        let code = response.body["error"].as_str().unwrap_or_default();
        assert_ne!(
            code, "budget_exceeded",
            "{what}: the runtime meter stopped a validated route"
        );
        if strict() {
            assert_ne!(
                code, "runtime_error",
                "{what}: a validated route hit a runtime error"
            );
        }
        let changed = self.check_isolation(&before, &what);
        if response.status >= 400 {
            assert!(!changed, "{what}: a failed request changed the store");
        }
        response
    }

    /// Runs due work and checks every handler run stayed within its bounds and its tenant.
    pub fn process(&mut self, max_items: u32) {
        let before = self.snapshot();
        let processed = self.engine.process(&self.mine, max_items);
        for run in &processed.runs {
            let Some(entry) = self.report.handlers.get(&run.handler) else {
                panic!(
                    "process ran handler {}, which has no report entry",
                    run.handler
                );
            };
            assert!(
                run.usage.fits_within(&entry.bounds),
                "handler {}: usage {:?} over the static bounds {:?}",
                run.handler,
                run.usage,
                entry.bounds
            );
        }
        self.check_isolation(&before, "process");
        drop(self.sink.take());
        drop(self.integrations.take_calls());
    }

    /// Replays the first `steps` steps of the case's golden scenario (requests, seeds, users,
    /// mocks, secrets, process), checking invariants on each when `check` is set.
    pub fn replay_scenario(&mut self, steps: usize, check: bool) {
        let scenario = &parsed().scenarios[self.case];
        let step_ms = scenario["stepMs"].as_i64().unwrap_or(1000);
        let Some(all) = scenario["steps"].as_array() else {
            return;
        };
        for step in all.iter().take(steps) {
            self.env
                .advance(step_ms.saturating_add(step["advanceMs"].as_i64().unwrap_or(0)));
            if let Some(secret) = step.get("secret") {
                self.secrets.set(
                    secret["name"].as_str().unwrap_or_default(),
                    secret["value"].as_str().unwrap_or_default(),
                );
            } else if let Some(user) = step.get("user") {
                if let (Some(id), Some(profile)) =
                    (user["id"].as_str(), user["profile"].as_object())
                {
                    let _ = self.engine.put_user(&self.mine, id, profile);
                }
            } else if let Some(seed) = step.get("seed") {
                let rows = substitute(&seed["rows"], &self.vars);
                let rows = rows.as_array().cloned().unwrap_or_default();
                let before = self.snapshot();
                let _ = self.engine.seed(
                    &self.mine,
                    seed["entity"].as_str().unwrap_or_default(),
                    &rows,
                );
                if check {
                    self.check_isolation(&before, "seed");
                }
            } else if let Some(mock) = step.get("mock") {
                let response = match mock.get("error").and_then(Json::as_str) {
                    Some(error) => Err(error.to_owned()),
                    None => Ok(mock.get("response").cloned().unwrap_or(Json::Null)),
                };
                self.integrations.respond(
                    mock["integration"].as_str().unwrap_or_default(),
                    mock["operation"].as_str().unwrap_or_default(),
                    response,
                );
            } else if step["process"].as_bool() == Some(true) {
                if check {
                    self.process(1000);
                } else {
                    self.engine.process(&self.mine, 1000);
                }
            } else if let Some(request) = step.get("request") {
                let Ok(request) =
                    serde_json::from_value::<Request>(substitute(request, &self.vars))
                else {
                    continue;
                };
                let response = if check {
                    self.request(&request)
                } else {
                    self.engine.handle(&self.mine, &request)
                };
                self.capture(step, &response);
            }
        }
    }

    fn capture(&mut self, step: &Json, response: &Response) {
        let Some(captures) = step["capture"].as_object() else {
            return;
        };
        let Ok(as_json) = serde_json::to_value(response) else {
            return;
        };
        for (name, path) in captures {
            let value = path.as_str().and_then(|p| {
                p.split('.')
                    .try_fold(&as_json, |current, key| match current {
                        Json::Object(map) => map.get(key),
                        Json::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
                        _ => None,
                    })
            });
            if let Some(value) = value.and_then(Json::as_str) {
                self.vars.insert(name.clone(), value.to_owned());
            }
        }
    }

    /// The number of steps in the case's golden scenario.
    pub fn scenario_len(&self) -> usize {
        parsed().scenarios[self.case]["steps"]
            .as_array()
            .map_or(0, Vec::len)
    }

    /// Runs one fuzzer-chosen action: a generated request, a mutated scenario request, a
    /// `process` call, a clock advance, a bulk seed, a user profile, or an integration mock.
    pub fn act(&mut self, u: &mut Unstructured<'_>) -> Result<()> {
        match u.int_in_range(0..=10)? {
            10 => self.bulk_seed(u)?,
            0..=4 => {
                let routes = &self.engine.manifest().manifest().routes;
                if routes.is_empty() {
                    return Ok(());
                }
                let route = routes[u.choose_index(routes.len())?].clone();
                let request = self.generate_request(&route, u)?;
                self.request(&request);
            }
            5 | 6 => {
                let scenario = &parsed().scenarios[self.case];
                let requests: Vec<&Json> = scenario["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.get("request"))
                    .collect();
                if requests.is_empty() {
                    return Ok(());
                }
                let mut request = substitute(u.choose(&requests)?, &self.vars);
                for _ in 0..u.int_in_range(0..=3)? {
                    mutate(&mut request, u, self.pool)?;
                }
                if let Ok(request) = serde_json::from_value::<Request>(request) {
                    self.request(&request);
                }
            }
            7 => {
                let max = u.int_in_range(1..=64)?;
                self.process(max);
            }
            8 => {
                let ms = match u.int_in_range(0..=3)? {
                    0 => i64::from(u.arbitrary::<u16>()?),
                    1 => 60_000 * i64::from(u.arbitrary::<u8>()?),
                    2 => 86_400_000 * i64::from(u.int_in_range(1_u8..=40)?),
                    _ => i64::from(u.arbitrary::<u32>()?) * 1000,
                };
                self.env.advance(ms);
            }
            _ => {
                if u.arbitrary()? {
                    let profile = self.engine.manifest().manifest().auth.profile.clone();
                    let mut fields = Map::new();
                    for (name, field) in &profile {
                        if u.ratio(4, 5)? {
                            fields.insert(name.clone(), self.value(&field.ty, u, 0)?);
                        }
                    }
                    let id = self.token(u)?;
                    let before = self.snapshot();
                    let _ = self.engine.put_user(&self.mine, &id, &fields);
                    self.check_isolation(&before, "put_user");
                } else {
                    let integration = self.token(u)?;
                    let operation = self.token(u)?;
                    let response = if u.arbitrary()? {
                        Err(self.token(u)?)
                    } else {
                        Ok(small(u, self.pool)?)
                    };
                    self.integrations
                        .respond(&integration, &operation, response);
                }
            }
        }
        Ok(())
    }

    /// Seeds up to 150 generated rows into one entity, mostly sharing every key field but the
    /// last, so queries and loops over one partition run up against their limits.
    fn bulk_seed(&mut self, u: &mut Unstructured<'_>) -> Result<()> {
        let entities = &self.engine.manifest().manifest().entities;
        if entities.is_empty() {
            return Ok(());
        }
        let names: Vec<&String> = entities.keys().collect();
        let name = (*u.choose(&names)?).clone();
        let entity = entities[&name].clone();
        let mut base = Map::new();
        for (field, decl) in &entity.fields {
            if !decl.optional || u.arbitrary()? {
                base.insert(field.clone(), self.value(&decl.ty, u, 0)?);
            }
        }
        let mut rows = Vec::new();
        for _ in 0..u.int_in_range(1..=150)? {
            let mut row = base.clone();
            for field in entity
                .key
                .iter()
                .rev()
                .take(if u.ratio(7, 8)? { 1 } else { 2 })
            {
                if let Some(decl) = entity.fields.get(field) {
                    row.insert(field.clone(), self.value(&decl.ty, u, 0)?);
                }
            }
            rows.push(Json::Object(row));
        }
        let before = self.snapshot();
        let _ = self.engine.seed(&self.mine, &name, &rows);
        self.check_isolation(&before, "seed");
        Ok(())
    }

    /// A string the manifest or scenario uses, or a short arbitrary one.
    fn token(&self, u: &mut Unstructured<'_>) -> Result<String> {
        if u.ratio(7, 8)? {
            if let Ok(s) = u.choose(self.pool) {
                return Ok(s.clone());
            }
        }
        let s: String = u.arbitrary()?;
        Ok(s.chars().take(24).collect())
    }

    /// A value for `ty`: usually one that conforms, sometimes at or just past an edge, now and
    /// then of the wrong type entirely.
    fn value(&self, ty: &FieldType, u: &mut Unstructured<'_>, depth: u32) -> Result<Json> {
        if depth > 6 || u.ratio(1, 24)? {
            return small(u, self.pool);
        }
        Ok(match ty {
            FieldType::String { min, max } => {
                if u.ratio(1, 3)? {
                    json!(self.token(u)?)
                } else {
                    let (min, max) = (*min, (*max).max(*min));
                    let len = match u.int_in_range(0..=3)? {
                        0 => min,
                        1 => max,
                        2 => max.saturating_add(1),
                        _ => u.int_in_range(min..=max.min(min.saturating_add(40)))?,
                    };
                    let c = *u.choose(&['a', 'Z', '0', ' ', 'é', '😀', '-', '/'])?;
                    let len = usize::try_from(len.min(4096)).unwrap_or(0);
                    json!(std::iter::repeat_n(c, len).collect::<String>())
                }
            }
            FieldType::Int { min, max } => {
                let (lo, hi) = (min.unwrap_or(i64::MIN), max.unwrap_or(i64::MAX));
                json!(match u.int_in_range(0..=5)? {
                    0 => lo,
                    1 => hi,
                    2 => lo.saturating_sub(1),
                    3 => hi.saturating_add(1),
                    _ => {
                        let (lo, hi) = (lo.max(-1000), hi.min(1000));
                        if lo <= hi {
                            u.int_in_range(lo..=hi)?
                        } else {
                            lo
                        }
                    }
                })
            }
            FieldType::Bool => json!(u.arbitrary::<bool>()?),
            FieldType::Enum { values } => match u.choose(values) {
                Ok(v) if u.ratio(9, 10)? => json!(v),
                _ => json!(self.token(u)?),
            },
            FieldType::Id => {
                if u.ratio(1, 6)? {
                    json!(self.token(u)?)
                } else {
                    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
                    let mut id = String::with_capacity(26);
                    id.push(char::from(ALPHABET[u.choose_index(8)?]));
                    for _ in 1..26 {
                        id.push(char::from(ALPHABET[u.choose_index(ALPHABET.len())?]));
                    }
                    json!(id)
                }
            }
            FieldType::Timestamp => json!(match u.int_in_range(0..=3)? {
                0 => START_MS,
                1 => u.arbitrary::<i64>()?,
                _ => START_MS + i64::from(u.arbitrary::<i32>()?) * 1000,
            }),
            FieldType::Array { items, max } => {
                let len = u.int_in_range(0..=(*max).min(6).saturating_add(1))?;
                let mut out = Vec::new();
                for _ in 0..len {
                    out.push(self.value(items, u, depth + 1)?);
                }
                Json::Array(out)
            }
            FieldType::Object { fields } => {
                let mut out = Map::new();
                for (name, field) in fields {
                    if !field.optional || u.arbitrary()? {
                        out.insert(name.clone(), self.value(&field.ty, u, depth + 1)?);
                    }
                }
                Json::Object(out)
            }
            FieldType::Decimal { scale, min, max } => json!(match u.int_in_range(0..=4)? {
                0 => min.clone().unwrap_or_else(|| "0".into()),
                1 => max.clone().unwrap_or_else(|| "99999999999999999999".into()),
                2 => format!(
                    "{}.{}",
                    u.int_in_range(-5_i32..=1000)?,
                    "9".repeat(usize::from(*scale).saturating_add(1))
                ),
                _ => format!("{}", u.int_in_range(0_u32..=500)?),
            }),
            FieldType::TimeZone => json!(*u.choose(&[
                "UTC",
                "Europe/Paris",
                "America/New_York",
                "Asia/Kathmandu",
                "Etc/GMT+12",
                "Not/AZone",
            ])?),
        })
    }

    /// A value for `ty` as it would appear in a path segment or query string.
    fn text(&self, ty: &FieldType, u: &mut Unstructured<'_>) -> Result<String> {
        Ok(match self.value(ty, u, 0)? {
            Json::String(s) => s,
            other => other.to_string(),
        })
    }

    /// A request to `route`, built from its declared inputs and auth.
    fn generate_request(&self, route: &Route, u: &mut Unstructured<'_>) -> Result<Request> {
        let mut segments = Vec::new();
        for segment in route.path.split('/') {
            segments.push(match segment.strip_prefix(':') {
                Some(name) => match route.input.params.get(name) {
                    Some(field) if u.ratio(15, 16)? => self.text(&field.ty, u)?,
                    _ => self.token(u)?,
                },
                None => segment.to_owned(),
            });
        }
        let mut query = BTreeMap::new();
        for (name, field) in &route.input.query {
            if !field.optional || u.arbitrary()? {
                query.insert(name.clone(), self.text(&field.ty, u)?);
            }
        }
        let mut body = Map::new();
        for (name, field) in &route.input.body {
            if !field.optional || u.ratio(2, 3)? {
                body.insert(name.clone(), self.value(&field.ty, u, 0)?);
            }
        }
        if u.ratio(1, 16)? {
            body.insert(self.token(u)?, small(u, self.pool)?);
        }
        let body = Json::Object(body);
        let caller = match u.int_in_range(0..=5)? {
            0 => None,
            1 => Some(Caller::user(self.token(u)?)),
            _ => Some(self.member(route, &segments, u)?),
        };
        let mut headers = BTreeMap::new();
        for _ in 0..u.int_in_range(0..=2)? {
            headers.insert(self.token(u)?.to_lowercase(), self.token(u)?);
        }
        if u.ratio(1, 8)? {
            headers.insert("origin".into(), self.token(u)?);
        }
        let method = if u.ratio(1, 32)? {
            *u.choose(&[Method::Get, Method::Post, Method::Options, Method::Delete])?
        } else {
            route.method
        };
        let raw_body = (route.webhook.is_some() || u.ratio(1, 16)?).then(|| body.to_string());
        Ok(Request {
            method,
            path: segments.join("/"),
            query,
            body: Some(body),
            caller,
            ip: if u.arbitrary()? {
                Some(format!("10.0.0.{}", u.int_in_range(1_u8..=4)?))
            } else {
                None
            },
            idempotency_key: if u.ratio(1, 4)? {
                Some(self.token(u)?)
            } else {
                None
            },
            headers,
            raw_body,
        })
    }

    /// A signed-in caller with a profile: for member routes, most often a member of the org the
    /// path names, with a role the route allows (or an API key for that org).
    fn member(
        &self,
        route: &Route,
        segments: &[String],
        u: &mut Unstructured<'_>,
    ) -> Result<Caller> {
        let mut caller = Caller::user(self.token(u)?);
        if let Auth::Member {
            org,
            roles,
            api_key_scopes,
        } = &route.auth
        {
            let from_path = segments
                .iter()
                .zip(route.path.split('/'))
                .find(|(_, s)| s.strip_prefix(':') == Some(org.as_str()))
                .map(|(v, _)| v.clone());
            let org_id = match from_path {
                Some(id) if u.ratio(7, 8)? => id,
                _ => self.token(u)?,
            };
            let role = match u.choose(roles) {
                Ok(r) if u.ratio(4, 5)? => r.clone(),
                _ => self.token(u)?,
            };
            if u.ratio(1, 4)? {
                caller.user_id = None;
                caller.api_key = Some(ApiKey {
                    org_id,
                    scopes: if u.arbitrary()? {
                        api_key_scopes.clone()
                    } else {
                        vec![self.token(u)?]
                    },
                });
            } else {
                caller.orgs.insert(org_id, role);
            }
        }
        for (name, field) in &self.engine.manifest().manifest().auth.profile {
            if u.ratio(3, 4)? {
                caller
                    .profile
                    .insert(name.clone(), self.value(&field.ty, u, 0)?);
            }
        }
        Ok(caller)
    }
}

/// Whether to also fail on `runtime_error` responses (`ALEPH_FUZZ_STRICT=1`): a validated
/// manifest the interpreter can't run is a checker gap, though not a broken promise.
fn strict() -> bool {
    static STRICT: OnceLock<bool> = OnceLock::new();
    *STRICT.get_or_init(|| std::env::var("ALEPH_FUZZ_STRICT").is_ok_and(|v| v == "1"))
}

/// Replaces `{{name}}` scenario variables; unknown ones stay as they are.
fn substitute(value: &Json, vars: &BTreeMap<String, String>) -> Json {
    match value {
        Json::String(s) => {
            let mut out = s.clone();
            for (name, replacement) in vars {
                out = out.replace(&format!("{{{{{name}}}}}"), replacement);
            }
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
