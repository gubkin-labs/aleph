//! WASM bindings for `aleph-core`.
//!
//! The interface is JSON strings in, JSON strings out. That keeps the boundary small and
//! language-neutral: `@aleph-lang/core` wraps it with types generated from the same Rust definitions.

use std::sync::Arc;

use aleph_core::{
    CheckReport, Diagnostic, Engine, FixedEnv, Limits, Manifest, MemoryStore, Request, TenantId,
    parse_and_validate,
    runtime::{MemorySecrets, MemorySink, MockIntegrations},
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// The result of validating a manifest: `{ ok: true, report }` or `{ ok: false, diagnostics }`.
#[derive(Serialize)]
#[serde(untagged)]
enum ValidateResult {
    Valid {
        ok: bool,
        report: CheckReport,
    },
    Invalid {
        ok: bool,
        diagnostics: Vec<Diagnostic>,
    },
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| format!("{{\"error\":{:?}}}", e.to_string()))
}

/// The manifest format version this core reads.
#[wasm_bindgen(js_name = formatVersion)]
pub fn format_version() -> String {
    aleph_core::manifest::FORMAT_VERSION.to_owned()
}

/// The default route limits, as JSON.
#[wasm_bindgen(js_name = defaultLimits)]
pub fn default_limits() -> String {
    to_json(&Limits::ROUTE)
}

/// The default limits for handlers (queue jobs, triggers, cron runs), as JSON.
#[wasm_bindgen(js_name = jobLimits)]
pub fn job_limits() -> String {
    to_json(&Limits::JOB)
}

/// Validates a manifest. Returns `{"ok":true,"report":...}` or `{"ok":false,"diagnostics":[...]}`.
#[wasm_bindgen]
pub fn validate(manifest_json: &str) -> String {
    to_json(&validate_inner(manifest_json))
}

fn validate_inner(manifest_json: &str) -> ValidateResult {
    match parse_and_validate(manifest_json, &Limits::ROUTE) {
        Ok(valid) => ValidateResult::Valid {
            ok: true,
            report: valid.report().clone(),
        },
        Err(diagnostics) => ValidateResult::Invalid {
            ok: false,
            diagnostics,
        },
    }
}

/// Runs every route example in a manifest (`aleph test`). Returns the results (JSON array), or
/// throws the diagnostics if the manifest is invalid.
#[wasm_bindgen(js_name = runExamples)]
pub fn run_examples(manifest_json: &str) -> Result<String, JsError> {
    let valid = parse_and_validate(manifest_json, &Limits::ROUTE)
        .map_err(|diagnostics| JsError::new(&to_json(&diagnostics)))?;
    Ok(to_json(&aleph_core::examples::run_all(&valid)))
}

#[derive(Serialize)]
struct Evolution<'a> {
    deployable: bool,
    #[serde(flatten)]
    report: &'a aleph_core::evolution::EvolutionReport,
}

/// Compares a deployed manifest with a new one. Returns the evolution report (JSON).
#[wasm_bindgen(js_name = compareManifests)]
pub fn compare_manifests(old_json: &str, new_json: &str) -> Result<String, JsError> {
    let parse = |json: &str, which: &str| -> Result<Manifest, JsError> {
        serde_json::from_str(json).map_err(|e| JsError::new(&format!("{which} manifest: {e}")))
    };
    let report = aleph_core::evolution::compare(&parse(old_json, "old")?, &parse(new_json, "new")?);
    Ok(to_json(&Evolution {
        deployable: report.deployable(),
        report: &report,
    }))
}

/// A local, in-memory engine for `aleph dev` and tests. Time only moves when the caller passes a
/// new `now`, so runs are reproducible.
#[wasm_bindgen]
pub struct DevEngine {
    engine: Engine,
    env: Arc<FixedEnv>,
    sink: Arc<MemorySink>,
    integrations: Arc<MockIntegrations>,
    secrets: Arc<MemorySecrets>,
    store: Arc<MemoryStore>,
    tenant: TenantId,
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "JS timestamps are integral"
)]
fn ms(value: f64) -> i64 {
    value as i64
}

#[wasm_bindgen]
impl DevEngine {
    /// Validates `manifest_json` and creates an engine with an empty store. Throws the
    /// diagnostics as JSON if the manifest is invalid.
    #[wasm_bindgen(constructor)]
    pub fn new(manifest_json: &str, seed: f64) -> Result<DevEngine, JsError> {
        let valid = parse_and_validate(manifest_json, &Limits::ROUTE)
            .map_err(|diagnostics| JsError::new(&to_json(&diagnostics)))?;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "seed is any JS number"
        )]
        let env = Arc::new(FixedEnv::new(0, seed as u64));
        let sink = Arc::new(MemorySink::new());
        let integrations = Arc::new(MockIntegrations::new());
        let secrets = Arc::new(MemorySecrets::new());
        let store = Arc::new(MemoryStore::new());
        let engine = Engine::new(valid, store.clone(), env.clone())
            .with_sink(sink.clone())
            .with_integrations(integrations.clone())
            .with_secrets(secrets.clone());
        let tenant = TenantId::new("dev").map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Self {
            engine,
            env,
            sink,
            integrations,
            secrets,
            store,
            tenant,
        })
    }

    /// Runs due work (cron ticks, handlers, deliveries) at time `now_ms`, at most `max_items`
    /// outbox entries. Returns the process report (JSON).
    pub fn process(&self, now_ms: f64, max_items: u32) -> String {
        self.env.set_now(ms(now_ms));
        to_json(&self.engine.process(&self.tenant, max_items))
    }

    /// Answers `integration.operation` calls with `response_json`, or fails them with `error`
    /// when it's set.
    pub fn mock(
        &self,
        integration: &str,
        operation: &str,
        response_json: &str,
        error: Option<String>,
    ) -> Result<(), JsError> {
        let response = match error {
            Some(error) => Err(error),
            None => Ok(serde_json::from_str(response_json)
                .map_err(|e| JsError::new(&format!("invalid mock response: {e}")))?),
        };
        self.integrations.respond(integration, operation, response);
        Ok(())
    }

    /// Sets a secret (such as a webhook signing key).
    #[wasm_bindgen(js_name = setSecret)]
    pub fn set_secret(&self, name: &str, value: &str) {
        self.secrets.set(name, value);
    }

    /// Stores a user profile (JSON object) in the user directory.
    #[wasm_bindgen(js_name = putUser)]
    pub fn put_user(&self, user_id: &str, profile_json: &str) -> Result<(), JsError> {
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(profile_json)
                .map_err(|e| JsError::new(&format!("invalid profile: {e}")))?;
        self.engine
            .put_user(&self.tenant, user_id, &profile)
            .map_err(|e| JsError::new(&e))
    }

    /// Inserts seed rows (a JSON array) into an entity at time `now_ms`. Returns how many were
    /// inserted (rows whose key exists are skipped).
    pub fn seed(&self, entity: &str, rows_json: &str, now_ms: f64) -> Result<u32, JsError> {
        let rows: Vec<serde_json::Value> = serde_json::from_str(rows_json)
            .map_err(|e| JsError::new(&format!("invalid rows: {e}")))?;
        self.env.set_now(ms(now_ms));
        let n = self
            .engine
            .seed(&self.tenant, entity, &rows)
            .map_err(|e| JsError::new(&format!("{entity}: {e}")))?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Every stored entry as `[[hexKey, hexValue], ...]` (JSON), to persist dev data.
    pub fn snapshot(&self) -> String {
        let entries: Vec<[String; 2]> = self
            .store
            .entries()
            .into_iter()
            .map(|(k, v)| [hex(k.as_bytes()), hex(&v)])
            .collect();
        to_json(&entries)
    }

    /// Replaces the stored data with a snapshot.
    pub fn restore(&self, snapshot_json: &str) -> Result<(), JsError> {
        let entries: Vec<[String; 2]> = serde_json::from_str(snapshot_json)
            .map_err(|e| JsError::new(&format!("invalid snapshot: {e}")))?;
        let decoded = entries
            .into_iter()
            .map(|[k, v]| Some((aleph_core::keys::Key::from_bytes(unhex(&k)?), unhex(&v)?)))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| JsError::new("invalid snapshot: bad hex"))?;
        self.store.restore(decoded);
        Ok(())
    }

    /// Removes and returns everything delivered to the sink so far (JSON array).
    #[wasm_bindgen(js_name = takeDeliveries)]
    pub fn take_deliveries(&self) -> String {
        let deliveries: Vec<_> = self.sink.take().into_iter().map(|(_, d)| d).collect();
        to_json(&deliveries)
    }

    /// Removes and returns every integration call made so far (JSON array).
    #[wasm_bindgen(js_name = takeCalls)]
    pub fn take_calls(&self) -> String {
        to_json(&self.integrations.take_calls())
    }

    /// Handles a request (JSON) at time `now_ms` and returns the response (JSON).
    pub fn handle(&self, request_json: &str, now_ms: f64) -> Result<String, JsError> {
        let request: Request = serde_json::from_str(request_json)
            .map_err(|e| JsError::new(&format!("invalid request: {e}")))?;
        self.env.set_now(ms(now_ms));
        Ok(to_json(&self.engine.handle(&self.tenant, &request)))
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_json_reports_a_diagnostic() {
        let out = validate("{");
        assert!(
            out.contains("\"ok\":false") && out.contains("invalid_manifest"),
            "{out}"
        );
    }
}
