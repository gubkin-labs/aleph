//! # aleph-core
//!
//! The trusted core of Aleph: manifest types, the validator and static usage analyzer, and the
//! bounded plan interpreter with its runtime budget meter.
//!
//! The same crate runs natively inside the platform and as WASM inside the CLI, so author
//! feedback, local runs, and production all share one implementation.
//!
//! ```text
//! manifest JSON ──parse_and_validate──▶ ValidatedManifest ──Engine::handle──▶ Response
//!                  (types, scopes,                         (one optimistic transaction,
//!                   worst-case usage)                       metered, tenant-isolated keys)
//! ```

pub mod check;
pub mod decimal;
pub mod diagnostics;
pub mod evolution;
pub mod examples;
pub mod keys;
pub mod limits;
pub mod manifest;
pub mod runtime;
pub mod store;
pub mod value;

pub use check::{CheckReport, ValidatedManifest, parse_and_validate, validate};
pub use diagnostics::{Diagnostic, DiagnosticCode};
pub use keys::TenantId;
pub use limits::{Cost, Limits, Resource};
pub use manifest::Manifest;
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::SystemEnv;
pub use runtime::{
    ApiKey, BlobAccess, BlobGrant, BlobInfo, BlobRejection, BlobSigner, Caller, Engine, Env,
    FixedEnv, Request, Response,
};
pub use store::{MemoryStore, Store};
pub use value::Value;
