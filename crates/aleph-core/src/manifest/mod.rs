//! The manifest: the versioned, declarative contract between authoring tools and platforms.
//!
//! These Rust types are the **single source of truth** for the manifest format. The JSON Schema
//! (`schema/manifest.schema.json`) and the TypeScript types (`@aleph/manifest`) are generated from
//! them by `cargo xtask codegen`, so the two sides can't drift apart.
//!
//! A manifest is plain data. It never contains author code. Logic is expressed as a [`Plan`]: a
//! tree of statements ([`Stmt`]), operations ([`Op`]), and expressions ([`Expr`]) drawn from a
//! closed set of built-ins. That's what makes static cost analysis and safe interpretation
//! possible.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod expr;
mod plan;

pub use expr::{BinaryOp, Expr, InputSource, Literal};
pub use plan::{Op, Order, Plan, Stmt, VarId};

/// The manifest format version this crate reads and writes.
pub const FORMAT_VERSION: &str = "0.1";

/// A complete application manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    /// Manifest format version, for example `"0.1"`.
    pub format_version: String,
    /// Application metadata.
    pub app: App,
    /// Roles and API key scopes the app's routes may require.
    #[serde(default)]
    #[cfg_attr(feature = "codegen", ts(as = "Option<AuthConfig>", optional))]
    pub auth: AuthConfig,
    /// Declared entities, by name.
    #[serde(default)]
    pub entities: BTreeMap<String, Entity>,
    /// Declared counters, by name. Counters are maintained on write and read in O(1).
    #[serde(default)]
    pub counters: BTreeMap<String, Counter>,
    /// Declared HTTP routes.
    #[serde(default)]
    pub routes: Vec<Route>,
}

/// Application metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct App {
    /// Application name.
    pub name: String,
}

/// The roles and API key scopes an app declares. Routes may only require declared ones, so a typo
/// is a validation error instead of a route nobody can call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthConfig {
    /// Org member roles, for example `["owner", "admin", "member"]`.
    #[serde(default)]
    pub roles: Vec<String>,
    /// API key scopes, for example `["projects:read"]`.
    #[serde(default)]
    pub api_key_scopes: Vec<String>,
}

/// A stored entity: a schema plus an ordered primary key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entity {
    /// Primary key fields, in order. Rows are stored sorted by this key, so a prefix of the key
    /// selects a contiguous range.
    pub key: Vec<String>,
    /// Fields, by name.
    pub fields: BTreeMap<String, Field>,
    /// Secondary indexes, by name.
    #[serde(default)]
    pub indexes: BTreeMap<String, Index>,
}

/// A secondary index over a list of fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Index {
    /// Indexed fields, in order. Queries may filter on a prefix of these.
    pub fields: Vec<String>,
}

/// A field declaration: a type, optionality, and an optional default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Field {
    /// The field's type.
    #[serde(rename = "type")]
    pub ty: FieldType,
    /// Whether the field may be absent (stored as `null`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "codegen", ts(as = "Option<bool>", optional))]
    pub optional: bool,
    /// Value used when the field isn't provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub default: Option<DefaultValue>,
}

/// Field types. Every type has a statically known maximum size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub enum FieldType {
    /// A UTF-8 string with a length range in characters. `max` is required.
    String {
        /// Minimum length in characters.
        #[serde(default)]
        min: u32,
        /// Maximum length in characters.
        max: u32,
    },
    /// A 64-bit signed integer with an optional range.
    Int {
        /// Inclusive minimum.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional, type = "number"))]
        min: Option<i64>,
        /// Inclusive maximum.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "codegen", ts(optional, type = "number"))]
        max: Option<i64>,
    },
    /// A boolean.
    Bool,
    /// One of a fixed set of string values.
    Enum {
        /// Allowed values.
        values: Vec<String>,
    },
    /// A generated, time-ordered identifier (26-character ULID).
    Id,
    /// Milliseconds since the Unix epoch.
    Timestamp,
    /// A list with a maximum length.
    Array {
        /// Item type.
        items: Box<FieldType>,
        /// Maximum number of items.
        max: u32,
    },
}

/// How a missing field gets its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub enum DefaultValue {
    /// A constant value.
    Literal {
        /// The value.
        value: Literal,
    },
    /// The current time (timestamp fields only).
    Now,
    /// A newly generated id (id fields only).
    NewId,
}

/// A counter over the rows of an entity, grouped by some of its fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Counter {
    /// The counted entity.
    pub of: String,
    /// Fields the count is grouped by.
    pub group_by: Vec<String>,
    /// Only rows matching this predicate are counted.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub filter: Option<RowPredicate>,
}

/// A boolean expression over one row, bound to `var`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowPredicate {
    /// The variable the row is bound to inside `expr`.
    pub var: VarId,
    /// The predicate.
    pub expr: Expr,
}

/// An HTTP route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Route {
    /// Stable identifier, unique within the manifest.
    pub id: String,
    /// HTTP method.
    pub method: Method,
    /// Path template, for example `/orgs/:orgId/projects`.
    pub path: String,
    /// Who may call the route.
    pub auth: Auth,
    /// Declared inputs.
    #[serde(default)]
    pub input: Input,
    /// Declared business errors the plan may fail with, by name.
    #[serde(default)]
    pub errors: BTreeMap<String, ErrorDecl>,
    /// Replays successful responses for repeated idempotency keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "codegen", ts(optional))]
    pub idempotency: Option<Idempotency>,
    /// The route's logic.
    pub plan: Plan,
}

/// Idempotency settings for a route. A successful response is stored with the request's key in
/// the same transaction as the route's writes, and replayed for the same key within `ttl_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Idempotency {
    /// How long a stored response is replayed, in milliseconds (at most 7 days).
    pub ttl_ms: u32,
    /// Whether requests without an idempotency key are rejected.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "codegen", ts(as = "Option<bool>", optional))]
    pub required: bool,
}

impl Idempotency {
    /// The longest allowed TTL: 7 days.
    pub const MAX_TTL_MS: u32 = 7 * 24 * 60 * 60 * 1000;
}

/// HTTP methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "UPPERCASE")]
#[allow(missing_docs)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    /// The method as an uppercase string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

/// Route authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[serde(deny_unknown_fields)]
pub enum Auth {
    /// Anyone may call the route.
    Public,
    /// The caller must be signed in.
    SignedIn,
    /// The caller must be a member of the org named by a path parameter, or present an API key
    /// for that org.
    Member {
        /// The path parameter holding the org id.
        org: String,
        /// Member roles allowed to call the route. Empty means any member.
        #[serde(default)]
        roles: Vec<String>,
        /// Scopes an API key needs to call the route. Empty means API keys aren't accepted.
        #[serde(default)]
        api_key_scopes: Vec<String>,
    },
}

/// Route inputs. Every input is validated before the plan runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    /// Path parameters. Must match the `:name` segments of the path exactly.
    #[serde(default)]
    pub params: BTreeMap<String, Field>,
    /// Query string parameters.
    #[serde(default)]
    pub query: BTreeMap<String, Field>,
    /// JSON body fields.
    #[serde(default)]
    pub body: BTreeMap<String, Field>,
}

/// A declared business error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorDecl {
    /// HTTP status, 400–599.
    pub status: u16,
    /// Human-readable message returned to the client.
    pub message: String,
}
