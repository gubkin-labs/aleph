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
    /// The authenticated caller's user id, if any. Authentication happens before the core.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub caller: Option<String>,
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
        if route.auth == Auth::SignedIn && request.caller.is_none() {
            return Response::error(401, "unauthorized", "sign in to call this route");
        }

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

        self.execute(
            &Keyspace::new(tenant),
            route,
            &inputs,
            request.caller.as_deref(),
            now,
        )
    }

    /// Runs a route's plan, re-running it on transaction conflicts.
    fn execute(
        &self,
        keys: &Keyspace,
        route: &crate::manifest::Route,
        inputs: &Inputs,
        caller: Option<&str>,
        now: i64,
    ) -> Response {
        let env = self.env.as_ref();
        let limits = self.limits;
        let max_attempts = limits.max_conflict_retries.saturating_add(1);
        let mut usage = Cost::default();
        for attempt in 1..=max_attempts {
            let mut tx = self.store.begin();
            let result = {
                let mut exec = Exec {
                    manifest: self.manifest.manifest(),
                    route,
                    tx: tx.as_mut(),
                    keys,
                    meter: Meter::new(limits),
                    env,
                    now,
                    caller,
                    inputs,
                    vars: HashMap::new(),
                    metered: true,
                };
                let result = exec.run();
                usage = exec.meter.used();
                result
            };
            let response = match result {
                Ok(value) => match tx.commit() {
                    Ok(()) => Response {
                        route: None,
                        status: 200,
                        body: value.to_json(),
                        usage,
                        attempts: attempt,
                    },
                    Err(CommitError::Conflict) => continue,
                    Err(CommitError::Store(e)) => {
                        Response::error(503, "storage_unavailable", e.to_string())
                    }
                },
                Err(error) => error_response(route, error),
            };
            return Response {
                usage,
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

fn error_response(route: &crate::manifest::Route, error: ExecError) -> Response {
    match error {
        ExecError::Failed(name) => match route.errors.get(&name) {
            Some(decl) => Response::error(decl.status, &name, decl.message.clone()),
            None => Response::error(500, "runtime_error", format!("undeclared error {name}")),
        },
        ExecError::OverBudget(e) => Response::error(500, "budget_exceeded", e.to_string()),
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
