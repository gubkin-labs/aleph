//! WASM bindings for `aleph-core`.
//!
//! The interface is JSON strings in, JSON strings out. That keeps the boundary small and
//! language-neutral: `@aleph/core` wraps it with types generated from the same Rust definitions.

use std::sync::Arc;

use aleph_core::{
    CheckReport, Diagnostic, Engine, FixedEnv, Limits, MemoryStore, Request, TenantId,
    parse_and_validate,
    runtime::{MemorySink, MockIntegrations},
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

/// A local, in-memory engine for `aleph dev` and tests. Time only moves when the caller passes a
/// new `now`, so runs are reproducible.
#[wasm_bindgen]
pub struct DevEngine {
    engine: Engine,
    env: Arc<FixedEnv>,
    sink: Arc<MemorySink>,
    integrations: Arc<MockIntegrations>,
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
        let engine = Engine::new(valid, Arc::new(MemoryStore::new()), env.clone())
            .with_sink(sink.clone())
            .with_integrations(integrations.clone());
        let tenant = TenantId::new("dev").map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Self {
            engine,
            env,
            sink,
            integrations,
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
