//! The runtime: routes requests to plans and runs them, isolated per tenant and metered.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::check::ValidatedManifest;
use crate::keys::{Keyspace, TenantId};
use crate::limits::Cost;
use crate::manifest::{Auth, Method};
use crate::store::{CommitError, Store};

mod env;
mod exec;
mod input;
mod meter;

#[cfg(not(target_arch = "wasm32"))]
pub use env::SystemEnv;
pub use env::{Env, FixedEnv};
pub use input::InputProblem;

use exec::{Exec, ExecError};
use input::Inputs;
use meter::Meter;

/// An HTTP-shaped request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    /// HTTP method.
    pub method: Method,
    /// Request path, without the query string.
    pub path: String,
    /// Query string parameters.
    #[serde(default)]
    #[cfg_attr(
        feature = "codegen",
        ts(as = "Option<BTreeMap<String, String>>", optional)
    )]
    pub query: BTreeMap<String, String>,
    /// JSON body.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(optional, type = "unknown"))]
    pub body: Option<serde_json::Value>,
    /// Who is calling, as resolved by the platform. Authentication happens before the core;
    /// the core only enforces each route's declared rule.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub caller: Option<Caller>,
    /// The client's idempotency key (from the `Idempotency-Key` header), if any.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub idempotency_key: Option<String>,
}

/// An authenticated caller.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Caller {
    /// The signed-in user, if any.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub user_id: Option<String>,
    /// The user's org memberships: org id → role.
    #[serde(default)]
    #[cfg_attr(
        feature = "codegen",
        ts(as = "Option<BTreeMap<String, String>>", optional)
    )]
    pub orgs: BTreeMap<String, String>,
    /// The API key used, if the caller is a machine.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub api_key: Option<ApiKey>,
}

impl Caller {
    /// A signed-in user with no org memberships.
    pub fn user(id: impl Into<String>) -> Self {
        Self {
            user_id: Some(id.into()),
            ..Self::default()
        }
    }
}

/// An API key, as resolved by the platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKey {
    /// The org the key belongs to.
    pub org_id: String,
    /// The key's scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// An HTTP-shaped response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Response {
    /// The id of the route that handled the request, if one matched.
    #[cfg_attr(feature = "codegen", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// HTTP status.
    pub status: u16,
    /// JSON body. Errors are `{ "error": code, "message": text }`.
    #[cfg_attr(feature = "codegen", ts(type = "unknown"))]
    pub body: serde_json::Value,
    /// What the final attempt actually used. Always within the route's static bounds.
    pub usage: Cost,
    /// How many times the plan ran (more than 1 after transaction conflicts).
    pub attempts: u32,
    /// `true` if this is a stored response replayed for a repeated idempotency key.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "codegen", ts(as = "Option<bool>", optional))]
    pub replayed: bool,
}

impl Response {
    fn error(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self::error_with(status, code, message, None)
    }

    fn error_with(
        status: u16,
        code: &str,
        message: impl Into<String>,
        details: Option<serde_json::Value>,
    ) -> Self {
        let mut body = json!({ "error": code, "message": message.into() });
        if let (Some(details), Some(map)) = (details, body.as_object_mut()) {
            map.insert("details".into(), details);
        }
        Self {
            route: None,
            status,
            body,
            usage: Cost::default(),
            attempts: 0,
            replayed: false,
        }
    }
}

#[derive(Debug)]
enum Segment {
    Literal(String),
    Param(String),
}

#[derive(Debug)]
struct RoutePattern {
    method: Method,
    segments: Vec<Segment>,
    route: usize,
}

impl RoutePattern {
    fn matches(&self, segments: &[&str]) -> Option<BTreeMap<String, String>> {
        if segments.len() != self.segments.len() {
            return None;
        }
        let mut params = BTreeMap::new();
        for (pattern, actual) in self.segments.iter().zip(segments) {
            match pattern {
                Segment::Literal(literal) if literal == actual => {}
                Segment::Literal(_) => return None,
                Segment::Param(name) => {
                    params.insert(name.clone(), (*actual).to_owned());
                }
            }
        }
        Some(params)
    }
}

fn split_path(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// Runs a validated manifest's routes against a store.
///
/// One engine can serve many tenants: every request names its tenant, and all of its keys are
/// built inside that tenant's keyspace.
pub struct Engine {
    manifest: Arc<ValidatedManifest>,
    store: Arc<dyn Store>,
    env: Arc<dyn Env>,
    patterns: Vec<RoutePattern>,
    limits: crate::limits::Limits,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("app", &self.manifest.manifest().app.name)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// An engine for `manifest`.
    pub fn new(manifest: ValidatedManifest, store: Arc<dyn Store>, env: Arc<dyn Env>) -> Self {
        let patterns = manifest
            .manifest()
            .routes
            .iter()
            .enumerate()
            .map(|(route, r)| RoutePattern {
                method: r.method,
                segments: split_path(&r.path)
                    .into_iter()
                    .map(|s| match s.strip_prefix(':') {
                        Some(name) => Segment::Param(name.to_owned()),
                        None => Segment::Literal(s.to_owned()),
                    })
                    .collect(),
                route,
            })
            .collect();
        let limits = *manifest.limits();
        Self {
            manifest: Arc::new(manifest),
            store,
            env,
            patterns,
            limits,
        }
    }

    /// Runs with tighter runtime limits than the manifest was validated against. The meter then
    /// stops executions the static analysis allowed: useful to test the meter as the backstop it
    /// is meant to be.
    #[must_use]
    pub fn with_runtime_limits(mut self, limits: crate::limits::Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The manifest this engine runs.
    pub fn manifest(&self) -> &ValidatedManifest {
        &self.manifest
    }

    /// Handles one request for `tenant`.
    pub fn handle(&self, tenant: &TenantId, request: &Request) -> Response {
        let segments = split_path(&request.path);
        let mut path_matched = false;
        let mut found = None;
        for pattern in &self.patterns {
            if let Some(params) = pattern.matches(&segments) {
                path_matched = true;
                if pattern.method == request.method {
                    found = Some((pattern.route, params));
                    break;
                }
            }
        }
        let Some((route_index, raw_params)) = found else {
            return if path_matched {
                Response::error(
                    405,
                    "method_not_allowed",
                    format!(
                        "{} isn't allowed on {}",
                        request.method.as_str(),
                        request.path
                    ),
                )
            } else {
                Response::error(
                    404,
                    "route_not_found",
                    format!("no route matches {}", request.path),
                )
            };
        };
        let route = &self.manifest.manifest().routes[route_index];
        Response {
            route: Some(route.id.clone()),
            ..self.handle_route(tenant, route, &raw_params, request)
        }
    }

    /// Authorizes the caller, decodes inputs, and runs one matched route.
    fn handle_route(
        &self,
        tenant: &TenantId,
        route: &crate::manifest::Route,
        raw_params: &BTreeMap<String, String>,
        request: &Request,
    ) -> Response {
        if let Err(response) = authorize(&route.auth, request.caller.as_ref(), raw_params) {
            return response;
        }
        let idempotency = match (route.idempotency, request.idempotency_key.as_deref()) {
            (Some(cfg), None) if cfg.required => {
                return Response::error(
                    400,
                    "idempotency_key_required",
                    "this route needs an Idempotency-Key",
                );
            }
            (None, _) | (Some(_), None) => None,
            (Some(cfg), Some(key)) => {
                if key.is_empty() || key.len() > 255 || !key.bytes().all(|b| b.is_ascii_graphic()) {
                    return Response::error(
                        400,
                        "invalid_idempotency_key",
                        "idempotency keys are 1-255 printable ASCII characters",
                    );
                }
                Some((cfg, key))
            }
        };

        let now = self.env.now_ms();
        let env = self.env.as_ref();
        let mut problems = Vec::new();
        let inputs = Inputs {
            params: input::decode_strings(
                "params",
                &route.input.params,
                raw_params,
                env,
                now,
                &mut problems,
            ),
            query: input::decode_strings(
                "query",
                &route.input.query,
                &request.query,
                env,
                now,
                &mut problems,
            ),
            body: input::decode_body(
                &route.input.body,
                request.body.as_ref(),
                env,
                now,
                &mut problems,
            ),
        };
        if !problems.is_empty() {
            let details = serde_json::to_value(&problems).ok();
            return Response::error_with(
                400,
                "invalid_input",
                "the request has invalid inputs",
                details,
            );
        }

        let keys = Keyspace::new(tenant);
        let idempotency = idempotency.map(|(cfg, key)| Replay {
            key: keys.idempotency(&route.id, key),
            fingerprint: fingerprint(&inputs),
            ttl_ms: i64::from(cfg.ttl_ms),
        });
        self.execute(
            &keys,
            route,
            &inputs,
            request.caller.as_ref(),
            now,
            idempotency.as_ref(),
        )
    }

    /// Runs a route's plan, re-running it on transaction conflicts. With `replay`, a stored
    /// response for the same idempotency key is returned instead, and a new successful response
    /// is stored in the same transaction as the plan's writes.
    fn execute(
        &self,
        keys: &Keyspace,
        route: &crate::manifest::Route,
        inputs: &Inputs,
        caller: Option<&Caller>,
        now: i64,
        replay: Option<&Replay>,
    ) -> Response {
        let env = self.env.as_ref();
        let limits = self.limits;
        let max_attempts = limits.max_conflict_retries.saturating_add(1);
        let mut usage = Cost::default();
        for attempt in 1..=max_attempts {
            let mut tx = self.store.begin();
            let mut meter = Meter::new(limits);
            let response = match attempt_once(&mut *tx, &mut meter, replay, now, |tx, meter| {
                Exec {
                    manifest: self.manifest.manifest(),
                    route,
                    tx,
                    keys,
                    meter,
                    env,
                    now,
                    caller,
                    inputs,
                    vars: HashMap::new(),
                    metered: true,
                }
                .run()
            }) {
                Ok(Outcome::Replayed(stored)) => Response {
                    route: None,
                    status: stored.status,
                    body: stored.body,
                    usage: Cost::default(),
                    attempts: attempt,
                    replayed: true,
                },
                Ok(Outcome::Ran(body)) => match tx.commit() {
                    Ok(()) => Response {
                        route: None,
                        status: 200,
                        body,
                        usage: Cost::default(),
                        attempts: attempt,
                        replayed: false,
                    },
                    Err(CommitError::Conflict) => {
                        usage = meter.used();
                        continue;
                    }
                    Err(CommitError::Store(e)) => {
                        Response::error(503, "storage_unavailable", e.to_string())
                    }
                },
                Err(error) => error_response(route, error),
            };
            return Response {
                usage: meter.used(),
                attempts: attempt,
                ..response
            };
        }
        Response {
            usage,
            attempts: max_attempts,
            ..Response::error(
                503,
                "conflict",
                "too much contention: the transaction kept conflicting, retry later",
            )
        }
    }
}

/// Where and how to store a route's response for idempotent replay.
struct Replay {
    key: crate::keys::Key,
    fingerprint: String,
    ttl_ms: i64,
}

/// A stored idempotent response.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredResponse {
    fingerprint: String,
    status: u16,
    body: serde_json::Value,
    expires_at: i64,
}

enum Outcome {
    Ran(serde_json::Value),
    Replayed(StoredResponse),
}

/// One attempt: replay a stored response if there is one, otherwise run the plan and (for
/// idempotent routes) store its response in the same transaction.
fn attempt_once(
    tx: &mut dyn crate::store::Transaction,
    meter: &mut Meter,
    replay: Option<&Replay>,
    now: i64,
    run: impl FnOnce(
        &mut dyn crate::store::Transaction,
        &mut Meter,
    ) -> Result<crate::value::Value, ExecError>,
) -> Result<Outcome, ExecError> {
    if let Some(replay) = replay
        && let Some(bytes) = tx.get(&replay.key)?
    {
        meter.read(bytes.len())?;
        let stored: StoredResponse = serde_json::from_slice(&bytes)
            .map_err(|e| ExecError::Runtime(format!("corrupt idempotency record: {e}")))?;
        if stored.expires_at > now {
            if stored.fingerprint != replay.fingerprint {
                return Err(ExecError::KeyReused);
            }
            return Ok(Outcome::Replayed(stored));
        }
    } else if replay.is_some() {
        meter.read(0)?;
    }
    let body = run(&mut *tx, &mut *meter)?.to_json();
    if let Some(replay) = replay {
        let stored = StoredResponse {
            fingerprint: replay.fingerprint.clone(),
            status: 200,
            body,
            expires_at: now.saturating_add(replay.ttl_ms),
        };
        let bytes = serde_json::to_vec(&stored).map_err(|e| ExecError::Runtime(e.to_string()))?;
        if bytes.len() as u64 > meter.limits().max_row_bytes {
            return Err(ExecError::Runtime(format!(
                "idempotent responses must fit in {} bytes; this one is {}",
                meter.limits().max_row_bytes,
                bytes.len()
            )));
        }
        tx.put(replay.key.clone(), bytes);
        meter.write(1)?;
        return Ok(Outcome::Ran(stored.body));
    }
    Ok(Outcome::Ran(body))
}

/// A stable fingerprint of a request's decoded inputs (FNV-1a over canonical JSON), so a reused
/// idempotency key with different inputs is detected.
fn fingerprint(inputs: &Inputs) -> String {
    let canonical = serde_json::json!([
        crate::value::Value::Object(inputs.params.clone()).to_json(),
        crate::value::Value::Object(inputs.query.clone()).to_json(),
        crate::value::Value::Object(inputs.body.clone()).to_json(),
    ])
    .to_string();
    let hash = canonical.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// Enforces a route's auth rule. The caller's identity is already authenticated by the platform.
fn authorize(
    auth: &Auth,
    caller: Option<&Caller>,
    params: &BTreeMap<String, String>,
) -> Result<(), Response> {
    let unauthorized = || Response::error(401, "unauthorized", "sign in to call this route");
    let forbidden = |message: &str| Response::error(403, "forbidden", message);
    let caller = caller.filter(|c| c.user_id.is_some() || c.api_key.is_some());
    match auth {
        Auth::Public => Ok(()),
        Auth::SignedIn => match caller.and_then(|c| c.user_id.as_ref()) {
            Some(_) => Ok(()),
            None => Err(unauthorized()),
        },
        Auth::Member {
            org,
            roles,
            api_key_scopes,
        } => {
            let Some(caller) = caller else {
                return Err(unauthorized());
            };
            let Some(org_id) = params.get(org) else {
                return Err(forbidden("this route's org parameter is missing"));
            };
            if let Some(key) = &caller.api_key
                && !api_key_scopes.is_empty()
                && key.org_id == *org_id
                && api_key_scopes
                    .iter()
                    .all(|scope| key.scopes.contains(scope))
            {
                return Ok(());
            }
            match caller.orgs.get(org_id) {
                Some(role) if roles.is_empty() || roles.contains(role) => Ok(()),
                Some(role) => Err(forbidden(&format!(
                    "role {role:?} can't call this route (allowed: {roles:?})"
                ))),
                None if caller.api_key.is_some() && caller.user_id.is_none() => {
                    Err(forbidden("this API key doesn't have access to this route"))
                }
                None => Err(forbidden("you aren't a member of this org")),
            }
        }
    }
}

fn error_response(route: &crate::manifest::Route, error: ExecError) -> Response {
    match error {
        ExecError::Failed(name) => match route.errors.get(&name) {
            Some(decl) => Response::error(decl.status, &name, decl.message.clone()),
            None => Response::error(500, "runtime_error", format!("undeclared error {name}")),
        },
        ExecError::OverBudget(e) => Response::error(500, "budget_exceeded", e.to_string()),
        ExecError::BadInput(message) => Response::error(400, "invalid_input", message),
        ExecError::KeyReused => Response::error(
            422,
            "idempotency_key_reused",
            "this idempotency key was already used with different inputs",
        ),
        ExecError::NotFound(entity) => {
            Response::error(404, "not_found", format!("{entity} not found"))
        }
        ExecError::Duplicate(entity) => {
            Response::error(409, "already_exists", format!("{entity} already exists"))
        }
        ExecError::Store(e) => Response::error(503, "storage_unavailable", e.to_string()),
        ExecError::Runtime(message) => Response::error(500, "runtime_error", message),
    }
}
