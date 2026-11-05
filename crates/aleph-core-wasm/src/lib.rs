//! WASM bindings for `aleph-core`.
//!
//! The interface is JSON strings in, JSON strings out. That keeps the boundary small and
//! language-neutral: `@aleph/core` wraps it with types generated from the same Rust definitions.

use std::sync::Arc;

use aleph_core::{
    CheckReport, Diagnostic, Engine, FixedEnv, Limits, MemoryStore, Request, TenantId,
    parse_and_validate,
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
    tenant: TenantId,
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
        let engine = Engine::new(valid, Arc::new(MemoryStore::new()), env.clone());
        let tenant = TenantId::new("dev").map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Self {
            engine,
            env,
            tenant,
        })
    }

    /// Handles a request (JSON) at time `now_ms` and returns the response (JSON).
    pub fn handle(&self, request_json: &str, now_ms: f64) -> Result<String, JsError> {
        let request: Request = serde_json::from_str(request_json)
            .map_err(|e| JsError::new(&format!("invalid request: {e}")))?;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "JS timestamps are integral"
        )]
        self.env.set_now(now_ms as i64);
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
