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

mod builtins;
mod cron;
mod effects;
mod env;
mod exec;
mod input;
mod meter;
mod process;
mod time;
mod webhook;

pub use effects::{
    Delivery, DeliveryKind, Integrations, MemorySink, MockIntegrations, NoIntegrations, NullSink,
    RecordedCall, Sink,
};
#[cfg(not(target_arch = "wasm32"))]
pub use env::SystemEnv;
pub use env::{Env, FixedEnv};
pub use exec::LogEntry;
pub(crate) use exec::sort_comparisons;
pub use input::InputProblem;
pub use process::{HandlerRun, ProcessReport};
pub use webhook::{MemorySecrets, Secrets, sign_hex};

/// Whether an IANA time zone exists in the bundled database.
pub(crate) fn time_zone_exists(name: &str) -> bool {
    time::zone(name).is_some()
}

/// Checks a cron expression.
pub(crate) fn parse_cron(expr: &str) -> Result<(), String> {
    cron::Cron::parse(expr).map(|_| ())
}

/// Checks a `strftime` pattern.
pub(crate) fn valid_time_pattern(pattern: &str) -> Result<(), String> {
    time::valid_pattern(pattern)
}

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
    /// The client's IP address, for rate limits.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub ip: Option<String>,
    /// The client's idempotency key (from the `Idempotency-Key` header), if any.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub idempotency_key: Option<String>,
    /// Request headers, with lowercase names. Used for webhook signatures and CORS.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(
        feature = "codegen",
        ts(as = "Option<BTreeMap<String, String>>", optional)
    )]
    pub headers: BTreeMap<String, String>,
    /// The exact body text, for webhook signatures. When set and `body` isn't, the body is
    /// parsed from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub raw_body: Option<String>,
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
    /// The signed-in user's profile fields (those declared in `auth.profile`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "codegen", ts(optional, type = "Record<string, unknown>"))]
    pub profile: BTreeMap<String, serde_json::Value>,
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
    /// Response headers (lowercase names), set by the plan or by the platform (CORS,
    /// deprecation).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(
        feature = "codegen",
        ts(as = "Option<BTreeMap<String, String>>", optional)
    )]
    pub headers: BTreeMap<String, String>,
    /// `Set-Cookie` header values, one per cookie.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "codegen", ts(as = "Option<Vec<String>>", optional))]
    pub cookies: Vec<String>,
    /// Values the plan logged with `log`, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "codegen", ts(as = "Option<Vec<LogEntry>>", optional))]
    pub logs: Vec<LogEntry>,
}

impl Default for Response {
    fn default() -> Self {
        Self {
            route: None,
            status: 200,
            body: serde_json::Value::Null,
            usage: Cost::default(),
            attempts: 0,
            replayed: false,
            headers: BTreeMap::new(),
            cookies: Vec::new(),
            logs: Vec::new(),
        }
    }
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
            status,
            body,
            ..Self::default()
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
    sink: Arc<dyn Sink>,
    integrations: Arc<dyn Integrations>,
    secrets: Arc<dyn Secrets>,
    patterns: Vec<RoutePattern>,
    limits: crate::limits::Limits,
    rate_limiter: RateLimiter,
    write_queues: WriteQueues,
}

/// Per-key write queuing for hot spots. Optimistic transactions are cheap until many writers
/// hit the same key; then they mostly retry. After a conflict, a route's executions for that
/// tenant take turns (one at a time) until it cools down, so they commit instead of retrying.
#[derive(Debug)]
struct WriteQueues {
    stripes: Vec<WriteQueue>,
}

#[derive(Debug, Default)]
struct WriteQueue {
    turn: std::sync::Mutex<()>,
    /// How many more executions take turns. Reset on every conflict, counted down on commit.
    hot: std::sync::atomic::AtomicU32,
}

impl WriteQueues {
    /// Executions after a conflict that keep taking turns.
    const COOLDOWN: u32 = 16;

    fn new() -> Self {
        Self {
            stripes: (0..64).map(|_| WriteQueue::default()).collect(),
        }
    }

    fn stripe(&self, tenant: &TenantId, route: &str) -> &WriteQueue {
        let hash = tenant
            .as_str()
            .bytes()
            .chain([0])
            .chain(route.bytes())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            });
        &self.stripes[usize::try_from(hash % 64).unwrap_or(0)]
    }
}

impl WriteQueue {
    fn is_hot(&self) -> bool {
        self.hot.load(std::sync::atomic::Ordering::Relaxed) > 0
    }

    fn take_turn(&self) -> std::sync::MutexGuard<'_, ()> {
        self.turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn conflicted(&self) {
        self.hot
            .store(WriteQueues::COOLDOWN, std::sync::atomic::Ordering::Relaxed);
    }

    fn committed(&self) {
        let _ = self.hot.try_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |h| h.checked_sub(1),
        );
    }
}

/// Fixed-window rate limit counters, in memory. On the platform, rate limits are enforced at the
/// edge before requests reach an engine; this keeps a single engine (and `aleph dev`) faithful.
#[derive(Debug, Default)]
struct RateLimiter {
    counts: std::sync::Mutex<HashMap<(String, String, String, i64), u32>>,
}

impl RateLimiter {
    /// Counts a call. `Err` carries how long until the window resets.
    fn hit(
        &self,
        key: (String, String, String),
        limit: crate::manifest::RateLimit,
        now: i64,
    ) -> Result<(), i64> {
        let window_ms = i64::from(limit.window_ms);
        let window = now.div_euclid(window_ms);
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if counts.len() > 10_000 {
            counts.retain(|(_, _, _, w), _| *w >= window - 1);
        }
        let count = counts.entry((key.0, key.1, key.2, window)).or_insert(0);
        if *count >= limit.max {
            return Err((window + 1) * window_ms - now);
        }
        *count += 1;
        Ok(())
    }
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
            sink: Arc::new(NullSink),
            integrations: Arc::new(NoIntegrations),
            secrets: Arc::new(MemorySecrets::new()),
            patterns,
            limits,
            rate_limiter: RateLimiter::default(),
            write_queues: WriteQueues::new(),
        }
    }

    /// Delivers integration sends and channel messages to `sink` (the default discards them).
    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn Sink>) -> Self {
        self.sink = sink;
        self
    }

    /// Runs integration calls with `integrations` (the default fails every call).
    #[must_use]
    pub fn with_integrations(mut self, integrations: Arc<dyn Integrations>) -> Self {
        self.integrations = integrations;
        self
    }

    /// Runs with tighter runtime limits than the manifest was validated against. The meter then
    /// stops executions the static analysis allowed: useful to test the meter as the backstop it
    /// is meant to be.
    #[must_use]
    pub fn with_runtime_limits(mut self, limits: crate::limits::Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Uses `secrets` for webhook signing keys.
    #[must_use]
    pub fn with_secrets(mut self, secrets: Arc<dyn Secrets>) -> Self {
        self.secrets = secrets;
        self
    }

    /// Stores a user's profile in the tenant's user directory, for `auth.User.get`. Only fields
    /// declared in `auth.profile` are kept; values that don't fit their declaration are errors.
    pub fn put_user(
        &self,
        tenant: &TenantId,
        user_id: &str,
        profile: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let keys = Keyspace::new(tenant);
        let mut stored = BTreeMap::new();
        for (name, field) in &self.manifest.manifest().auth.profile {
            let value = match profile.get(name) {
                Some(json) => crate::value::Value::from_json(json.clone())?,
                None => crate::value::Value::Null,
            };
            let nullable = crate::manifest::Field {
                optional: true,
                ..field.clone()
            };
            let value = input::conform(&nullable, value).map_err(|p| format!("{name}: {p}"))?;
            stored.insert(name.clone(), value);
        }
        let bytes =
            serde_json::to_vec(&crate::value::Value::Object(stored)).map_err(|e| e.to_string())?;
        let mut tx = self.store.begin();
        tx.put(keys.user(user_id), bytes);
        tx.commit().map_err(|e| format!("{e:?}"))
    }

    /// Inserts seed rows into an entity, as if a plan inserted them: defaults, indexes,
    /// counters, invariants, and change triggers all apply. Rows whose key exists are skipped.
    /// Returns how many were inserted.
    pub fn seed(
        &self,
        tenant: &TenantId,
        entity: &str,
        rows: &[serde_json::Value],
    ) -> Result<usize, String> {
        let plan = crate::manifest::Plan {
            body: Vec::new(),
            output: crate::manifest::Expr::Lit {
                value: crate::manifest::Literal::Null,
            },
        };
        let errors = BTreeMap::new();
        let keys = Keyspace::new(tenant);
        let env = self.env.as_ref();
        let now = env.now_ms();
        let inputs = Inputs::default();
        let mut journal = effects::Journal::default();
        let mut tx = self.store.begin();
        // Seeding isn't a plan, so it isn't held to plan limits.
        let mut meter = Meter::new(crate::limits::Limits {
            reads: u64::MAX,
            writes: u64::MAX,
            ops: u64::MAX,
            bytes_read: u64::MAX,
            effects: u64::MAX,
            calls: 0,
            ..self.limits
        });
        let mut exec = Exec {
            manifest: self.manifest.manifest(),
            plan: &plan,
            errors: &errors,
            tenant,
            integrations: self.integrations.as_ref(),
            journal: &mut journal,
            effects: Vec::new(),
            tx: &mut *tx,
            keys: &keys,
            meter: &mut meter,
            env,
            now,
            caller: None,
            inputs: &inputs,
            vars: HashMap::new(),
            metered: false,
            meta: exec::ResponseMeta::default(),
        };
        let mut inserted = 0;
        for (i, row) in rows.iter().enumerate() {
            if exec
                .seed_row(entity, row)
                .map_err(|e| format!("row {i}: {e}"))?
            {
                inserted += 1;
            }
        }
        let effects = std::mem::take(&mut exec.effects);
        process::write_outbox(&mut *tx, &keys, env, now, effects).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| format!("{e:?}"))?;
        Ok(inserted)
    }

    /// The manifest this engine runs.
    pub fn manifest(&self) -> &ValidatedManifest {
        &self.manifest
    }

    /// Handles one request for `tenant`.
    pub fn handle(&self, tenant: &TenantId, request: &Request) -> Response {
        let cors = self.cors_headers(request);
        if request.method == Method::Options {
            // A CORS preflight: answered without running anything.
            return match cors {
                Some(mut headers) => {
                    let cfg = self.manifest.manifest().cors.as_ref();
                    let mut allowed: Vec<String> =
                        ["content-type", "authorization", "idempotency-key"]
                            .iter()
                            .map(|h| (*h).to_owned())
                            .collect();
                    allowed.extend(cfg.map(|c| c.headers.clone()).unwrap_or_default());
                    headers.insert(
                        "access-control-allow-methods".into(),
                        "GET, POST, PUT, PATCH, DELETE".into(),
                    );
                    headers.insert("access-control-allow-headers".into(), allowed.join(", "));
                    if let Some(max_age) = cfg.and_then(|c| c.max_age_s) {
                        headers.insert("access-control-max-age".into(), max_age.to_string());
                    }
                    Response {
                        status: 204,
                        headers,
                        ..Response::default()
                    }
                }
                None => Response::error(405, "method_not_allowed", "OPTIONS isn't allowed here"),
            };
        }
        let mut response = self.route_request(tenant, request);
        if let Some(headers) = cors {
            response.headers.extend(headers);
        }
        response
    }

    /// CORS headers for a request from an allowed origin.
    fn cors_headers(&self, request: &Request) -> Option<BTreeMap<String, String>> {
        let cors = self.manifest.manifest().cors.as_ref()?;
        let origin = request.headers.get("origin")?;
        let any = cors.origins.iter().any(|o| o == "*");
        if !any && !cors.origins.contains(origin) {
            return None;
        }
        let mut headers = BTreeMap::new();
        let allow = if any { "*".to_owned() } else { origin.clone() };
        headers.insert("access-control-allow-origin".into(), allow);
        if !any {
            headers.insert("vary".into(), "Origin".into());
        }
        if cors.credentials {
            headers.insert("access-control-allow-credentials".into(), "true".into());
        }
        Some(headers)
    }

    fn route_request(&self, tenant: &TenantId, request: &Request) -> Response {
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
        let mut response = Response {
            route: Some(route.id.clone()),
            ..self.handle_route(tenant, route, &raw_params, request)
        };
        if route.deprecated.is_some() {
            response.headers.insert("deprecation".into(), "true".into());
        }
        response
    }

    /// Authorizes the caller, decodes inputs, and runs one matched route.
    #[allow(clippy::too_many_lines, reason = "the request pipeline, in order")]
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
        if let Some(limit) = route.rate_limit {
            let subject = rate_subject(limit.per, request);
            let key = (tenant.as_str().to_owned(), route.id.clone(), subject);
            if let Err(retry_after_ms) = self.rate_limiter.hit(key, limit, self.env.now_ms()) {
                return Response::error_with(
                    429,
                    "rate_limited",
                    "too many requests: slow down and retry later",
                    Some(serde_json::json!({ "retryAfterMs": retry_after_ms })),
                );
            }
        }
        let parsed_body;
        let mut body = request.body.as_ref();
        let mut event_id = None;
        if let Some(hook) = &route.webhook {
            let Some(raw) = request.raw_body.as_deref() else {
                return Response::error(400, "invalid_input", "webhooks need the raw request body");
            };
            let secret = self.secrets.get(tenant, hook.verify.secret());
            match webhook::verify(
                &hook.verify,
                secret.as_deref(),
                &request.headers,
                raw,
                self.env.now_ms(),
            ) {
                Ok(()) => {}
                Err(webhook::Rejected::MissingSecret(name)) => {
                    return Response::error(
                        500,
                        "secret_missing",
                        format!("the webhook secret {name:?} isn't configured"),
                    );
                }
                Err(webhook::Rejected::Invalid(why)) => {
                    return Response::error(401, "invalid_signature", why);
                }
            }
            parsed_body = match serde_json::from_str::<serde_json::Value>(raw) {
                Ok(json) => json,
                Err(_) => return Response::error(400, "invalid_json", "the body isn't valid JSON"),
            };
            if let Some(dedupe) = &hook.dedupe {
                let from_header = dedupe
                    .header
                    .as_ref()
                    .and_then(|h| request.headers.get(h))
                    .cloned();
                let from_body = dedupe
                    .body_field
                    .as_ref()
                    .and_then(|f| match parsed_body.get(f) {
                        Some(serde_json::Value::String(s)) => Some(s.clone()),
                        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
                        _ => None,
                    });
                event_id = from_header.or(from_body).map(|id| (dedupe.ttl_ms, id));
            }
            body = Some(&parsed_body);
        }
        let idempotency = match (route.idempotency, request.idempotency_key.as_deref()) {
            _ if event_id.is_some() => None,
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
                body,
                route.webhook.is_some(),
                env,
                now,
                &mut problems,
            ),
            payload: BTreeMap::new(),
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
        let idempotency = match &event_id {
            // Webhook deliveries are deduplicated by event id, through the same replay records.
            Some((ttl_ms, id)) => Some(Replay {
                key: keys.idempotency(&route.id, id),
                fingerprint: fingerprint(&inputs),
                ttl_ms: i64::from(*ttl_ms),
            }),
            None => idempotency.map(|(cfg, key)| Replay {
                key: keys.idempotency(&route.id, key),
                fingerprint: fingerprint(&inputs),
                ttl_ms: i64::from(cfg.ttl_ms),
            }),
        };
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
    #[allow(clippy::too_many_arguments)]
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
        let mut journal = effects::Journal::default();
        let queue = self.write_queues.stripe(keys.tenant(), &route.id);
        // While the route is hot, take turns from the start.
        let mut turn = queue.is_hot().then(|| queue.take_turn());
        for attempt in 1..=max_attempts {
            if attempt > 1 && turn.is_none() {
                // This execution conflicted: queue behind the other writers instead of racing.
                turn = Some(queue.take_turn());
            }
            let mut tx = self.store.begin();
            let mut meter = Meter::new(limits);
            journal.rewind();
            let mut logs = Vec::new();
            let response = match attempt_once(&mut *tx, &mut meter, replay, now, |tx, meter| {
                let mut exec = Exec {
                    manifest: self.manifest.manifest(),
                    plan: &route.plan,
                    errors: &route.errors,
                    tenant: keys.tenant(),
                    integrations: self.integrations.as_ref(),
                    journal: &mut journal,
                    effects: Vec::new(),
                    tx,
                    keys,
                    meter,
                    env,
                    now,
                    caller,
                    inputs,
                    vars: HashMap::new(),
                    metered: true,
                    meta: exec::ResponseMeta::default(),
                };
                let result = exec.run();
                logs = std::mem::take(&mut exec.meta.logs);
                let value = result?;
                let effects = std::mem::take(&mut exec.effects);
                process::write_outbox(exec.tx, keys, env, now, effects)?;
                finish(route, value, std::mem::take(&mut exec.meta))
            }) {
                Ok(Outcome::Replayed(stored)) => Response {
                    status: stored.status,
                    body: stored.body,
                    headers: stored.headers,
                    cookies: stored.cookies,
                    replayed: true,
                    ..Response::default()
                },
                Ok(Outcome::Ran(finished)) => match tx.commit() {
                    Ok(()) => {
                        if turn.is_some() {
                            queue.committed();
                        }
                        Response {
                            status: finished.status,
                            body: finished.body,
                            headers: finished.headers,
                            cookies: finished.cookies,
                            ..Response::default()
                        }
                    }
                    Err(CommitError::Conflict) => {
                        queue.conflicted();
                        usage = meter.used();
                        continue;
                    }
                    Err(CommitError::Store(e)) => {
                        Response::error(503, "storage_unavailable", e.to_string())
                    }
                },
                Err(error) => {
                    // The failure was decided on what this attempt read; if any of it changed
                    // meanwhile, the decision may be wrong, so re-run instead of answering.
                    if tx.validate() == Err(CommitError::Conflict) {
                        queue.conflicted();
                        usage = meter.used();
                        continue;
                    }
                    error_response(route, error)
                }
            };
            return Response {
                usage: meter.used(),
                attempts: attempt,
                logs,
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

/// A successful response, before commit.
struct Finished {
    status: u16,
    body: serde_json::Value,
    headers: BTreeMap<String, String>,
    cookies: Vec<String>,
}

/// Shapes a plan's result into a response: the declared `returns` type (dropping undeclared
/// fields), the status it set, and no body for 204.
fn finish(
    route: &crate::manifest::Route,
    value: crate::value::Value,
    meta: exec::ResponseMeta,
) -> Result<Finished, ExecError> {
    let value = match &route.returns {
        Some(declared) => input::conform_output(declared, value)
            .map_err(|p| ExecError::Runtime(format!("response doesn't match `returns`: {p}")))?,
        None => value,
    };
    let status = meta.status.unwrap_or(200);
    let body = if status == 204 {
        serde_json::Value::Null
    } else {
        value.to_json()
    };
    Ok(Finished {
        status,
        body,
        headers: meta.headers,
        cookies: meta.cookies,
    })
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cookies: Vec<String>,
    expires_at: i64,
}

enum Outcome {
    Ran(Finished),
    Replayed(StoredResponse),
}

/// One attempt: replay a stored response if there is one, otherwise run the plan and (for
/// idempotent routes) store its response in the same transaction.
fn attempt_once(
    tx: &mut dyn crate::store::Transaction,
    meter: &mut Meter,
    replay: Option<&Replay>,
    now: i64,
    run: impl FnOnce(&mut dyn crate::store::Transaction, &mut Meter) -> Result<Finished, ExecError>,
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
    let finished = run(&mut *tx, &mut *meter)?;
    if let Some(replay) = replay {
        let stored = StoredResponse {
            fingerprint: replay.fingerprint.clone(),
            status: finished.status,
            body: finished.body,
            headers: finished.headers,
            cookies: finished.cookies,
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
        return Ok(Outcome::Ran(Finished {
            status: stored.status,
            body: stored.body,
            headers: stored.headers,
            cookies: stored.cookies,
        }));
    }
    Ok(Outcome::Ran(finished))
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

/// Who a rate-limited call counts for.
fn rate_subject(per: crate::manifest::RateSubject, request: &Request) -> String {
    use crate::manifest::RateSubject;
    let ip = || format!("ip:{}", request.ip.as_deref().unwrap_or("unknown"));
    let caller = request.caller.as_ref();
    match per {
        RateSubject::Global => "*".to_owned(),
        RateSubject::Ip => ip(),
        RateSubject::User => caller
            .and_then(|c| c.user_id.clone())
            .map_or_else(ip, |u| format!("user:{u}")),
        RateSubject::ApiKey => caller
            .and_then(|c| c.api_key.as_ref())
            .map_or_else(ip, |k| format!("key:{}", k.org_id)),
    }
}

/// Enforces a route's auth rule. The caller's identity is already authenticated by the platform.
#[allow(clippy::result_large_err, reason = "the error is the response to send")]
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
        ExecError::Failed(name, details) => match route.errors.get(&name) {
            Some(decl) => Response::error_with(
                decl.status,
                &name,
                decl.message.clone(),
                details.map(|d| d.to_json()),
            ),
            None => Response::error(500, "runtime_error", format!("undeclared error {name}")),
        },
        ExecError::OverBudget(e) => Response::error(500, "budget_exceeded", e.to_string()),
        ExecError::BadInput(message) => Response::error(400, "invalid_input", message),
        ExecError::CallFailed(message) => Response::error(502, "integration_error", message),
        ExecError::Unique(name) => Response::error(
            409,
            "unique_violation",
            format!("another row already has this value in {name}"),
        ),
        ExecError::Invariant(name) => Response::error(
            422,
            "invariant_violation",
            format!("this change would break the invariant {name}"),
        ),
        ExecError::KeyReused => Response::error(
            422,
            "idempotency_key_reused",
            "this idempotency key was already used with different inputs",
        ),
        ExecError::VersionConflict(entity) => Response::error(
            409,
            "version_conflict",
            format!("{entity} was changed since you read it; reload and try again"),
        ),
        ExecError::NotFound(entity) => {
            Response::error(404, "not_found", format!("{entity} not found"))
        }
        ExecError::Duplicate(entity) => {
            Response::error(409, "already_exists", format!("{entity} already exists"))
        }
        ExecError::Store(e) => Response::error(503, "storage_unavailable", e.to_string()),
        ExecError::Runtime(message) => Response::error(500, "runtime_error", message),
        // `run` turns an early return into a value; it never reaches here.
        ExecError::Returned(_) => Response::error(500, "runtime_error", "unexpected return"),
    }
}
