//! Validation diagnostics, designed to be actionable by humans and agents.

use serde::{Deserialize, Serialize};

/// A stable, machine-readable diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum DiagnosticCode {
    InvalidManifest,
    UnsupportedFormatVersion,
    InvalidName,
    InvalidFieldType,
    InvalidDefault,
    InvalidKey,
    InvalidIndex,
    RowTooLarge,
    UnknownEntity,
    UnknownField,
    UnknownIndex,
    UnknownCounter,
    UnknownError,
    UnknownInput,
    DuplicateRouteId,
    DuplicateRoute,
    InvalidPath,
    PathParamMismatch,
    InvalidErrorStatus,
    UndefinedVar,
    DuplicateVar,
    MissingKeyField,
    MissingRequiredField,
    ImmutableKeyField,
    InvalidPrefix,
    InvalidCounterKey,
    UnboundedList,
    UnboundedLimit,
    WriteInPureBlock,
    NonExhaustiveMatch,
    UnknownMatchCase,
    InvalidFieldAccess,
    TypeMismatch,
    InvalidPredicate,
    InvalidAuth,
    UnknownRole,
    UnknownScope,
    InvalidIdempotency,
    ImmutableEntity,
    UnknownEvent,
    UnknownQueue,
    UnknownChannel,
    UnknownIntegration,
    UnknownOperation,
    InvalidPayload,
    InvalidHandler,
    InvalidDeclaration,
    InvalidRoute,
    InvalidRange,
    InvalidExpression,
    OverBudget,
    TriggerCycle,
    UnknownBucket,
}

/// A problem found in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// Stable code, safe to match on.
    pub code: DiagnosticCode,
    /// Human-readable explanation, including how to fix it where possible.
    pub message: String,
    /// Location in the manifest, for example `routes[0].plan.body[2].op`.
    pub path: String,
    /// Location in the source, for example `app.ts:41:5`, when the manifest has a source map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub location: Option<String>,
}

impl Diagnostic {
    /// Creates a diagnostic.
    pub fn new(code: DiagnosticCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: path.into(),
            location: None,
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at {}: {}",
            serde_json::to_string(&self.code)
                .unwrap_or_default()
                .trim_matches('"'),
            self.path,
            self.message
        )
    }
}
