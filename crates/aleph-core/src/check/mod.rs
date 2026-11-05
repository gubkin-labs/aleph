//! Validation and static cost analysis.
//!
//! [`validate`] checks a manifest's structure, resolves every reference, infers types, computes
//! the worst-case [`Cost`] of every route, and rejects anything over the [`Limits`]. The only way
//! to get a [`ValidatedManifest`] (which the runtime requires) is through this check.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::diagnostics::{Diagnostic, DiagnosticCode as Code};
use crate::limits::{Cost, Limits, Resource};
use crate::manifest::{
    Auth, AuthConfig, Counter, DefaultValue, Entity, FORMAT_VERSION, Field, FieldType, Idempotency,
    Literal, Manifest, Route,
};

mod plan;
mod size;
mod types;

pub(crate) use plan::DEFAULT_CASE;
pub(crate) use types::Ty;

/// What static analysis found out about a valid manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    /// Per-entity facts, by entity name.
    pub entities: BTreeMap<String, EntityReport>,
    /// Per-route facts, by route id.
    pub routes: BTreeMap<String, RouteReport>,
}

/// Static facts about an entity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EntityReport {
    /// Worst-case encoded size of one row, in bytes.
    #[cfg_attr(feature = "codegen", ts(type = "number"))]
    pub max_row_bytes: u64,
}

/// Static facts about a route.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RouteReport {
    /// Worst-case cost of one execution. The runtime never exceeds it.
    pub bounds: Cost,
}

/// A manifest that passed validation, together with its analysis.
#[derive(Debug, Clone)]
pub struct ValidatedManifest {
    manifest: Manifest,
    report: CheckReport,
    limits: Limits,
}

impl ValidatedManifest {
    /// The manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The analysis.
    pub fn report(&self) -> &CheckReport {
        &self.report
    }

    /// The limits the manifest was validated against.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
}

/// Parses a manifest from JSON and validates it.
pub fn parse_and_validate(
    json: &str,
    limits: &Limits,
) -> Result<ValidatedManifest, Vec<Diagnostic>> {
    let manifest: Manifest = serde_json::from_str(json).map_err(|error| {
        vec![Diagnostic::new(
            Code::InvalidManifest,
            "$",
            format!("not a valid manifest: {error}"),
        )]
    })?;
    validate(manifest, limits)
}

/// Validates a manifest against `limits`.
///
/// Returns every problem found, not just the first, so authors and agents can fix them in one
/// pass.
pub fn validate(manifest: Manifest, limits: &Limits) -> Result<ValidatedManifest, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let report = check_manifest(&manifest, limits, &mut diags);
    if diags.is_empty() {
        Ok(ValidatedManifest {
            manifest,
            report,
            limits: *limits,
        })
    } else {
        diags.sort_by(|a, b| a.path.cmp(&b.path).then(a.code.cmp(&b.code)));
        Err(diags)
    }
}

/// Facts about entities that plan checking needs.
pub(crate) struct EntityInfo {
    pub(crate) row_bytes: u64,
    pub(crate) index_count: u64,
    pub(crate) counter_count: u64,
}

fn check_manifest(m: &Manifest, limits: &Limits, diags: &mut Vec<Diagnostic>) -> CheckReport {
    if m.format_version != FORMAT_VERSION {
        diags.push(Diagnostic::new(
            Code::UnsupportedFormatVersion,
            "formatVersion",
            format!(
                "format version {:?} isn't supported; this core reads {FORMAT_VERSION:?}",
                m.format_version
            ),
        ));
    }
    check_name(&m.app.name, "app.name", diags);

    let mut report = CheckReport::default();
    let mut infos = BTreeMap::new();
    for (name, entity) in &m.entities {
        let path = format!("entities.{name}");
        check_name(name, &path, diags);
        let row_bytes = check_entity(entity, &path, limits, diags);
        let counter_count = m.counters.values().filter(|c| &c.of == name).count() as u64;
        infos.insert(
            name.clone(),
            EntityInfo {
                row_bytes,
                index_count: entity.indexes.len() as u64,
                counter_count,
            },
        );
        report.entities.insert(
            name.clone(),
            EntityReport {
                max_row_bytes: row_bytes,
            },
        );
    }

    for (name, counter) in &m.counters {
        let path = format!("counters.{name}");
        check_name(name, &path, diags);
        check_counter(m, counter, &path, diags);
    }

    check_auth_config(&m.auth, diags);
    check_route_table(&m.routes, diags);
    for (i, route) in m.routes.iter().enumerate() {
        let path = format!("routes[{i}]");
        check_route_auth(&m.auth, route, &path, diags);
        let mut bounds = plan::check_route(m, &infos, route, &path, diags);
        if let Some(idempotency) = route.idempotency {
            check_idempotency(idempotency, &format!("{path}.idempotency"), diags);
            // Reading the stored response, and storing it (it must fit in one row).
            bounds += Cost {
                reads: 1,
                writes: 1,
                bytes_read: limits.max_row_bytes,
                ops: 0,
            };
        }
        for resource in Resource::ALL {
            let (bound, limit) = (bounds.get(resource), resource.limit(limits));
            if bound > limit {
                diags.push(Diagnostic::new(
                    Code::OverBudget,
                    format!("{path}.plan"),
                    format!(
                        "route {:?} can use up to {bound} {resource} in the worst case, over the limit of {limit}. \
                         Lower the bounds that drive it (query limits, list sizes) or split the work.",
                        route.id
                    ),
                ));
            }
        }
        report
            .routes
            .insert(route.id.clone(), RouteReport { bounds });
    }
    report
}

pub(crate) fn check_name(name: &str, path: &str, diags: &mut Vec<Diagnostic>) {
    let mut chars = name.chars();
    let valid = name.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        diags.push(Diagnostic::new(
            Code::InvalidName,
            path,
            format!("{name:?} isn't a valid name: use 1-64 characters of [A-Za-z0-9_], not starting with a digit"),
        ));
    }
}

/// Validates an entity and returns its worst-case row size.
fn check_entity(entity: &Entity, path: &str, limits: &Limits, diags: &mut Vec<Diagnostic>) -> u64 {
    for (name, field) in &entity.fields {
        let field_path = format!("{path}.fields.{name}");
        check_name(name, &field_path, diags);
        check_field(field, &field_path, diags);
    }

    if entity.key.is_empty() {
        diags.push(Diagnostic::new(
            Code::InvalidKey,
            format!("{path}.key"),
            "an entity needs at least one key field",
        ));
    }
    let mut seen = HashSet::new();
    for name in &entity.key {
        let key_path = format!("{path}.key");
        if !seen.insert(name) {
            diags.push(Diagnostic::new(
                Code::InvalidKey,
                &key_path,
                format!("key field {name:?} is listed twice"),
            ));
        }
        match entity.fields.get(name) {
            None => diags.push(unknown_field(&key_path, name, entity)),
            Some(field) if field.optional => diags.push(Diagnostic::new(
                Code::InvalidKey,
                &key_path,
                format!("key field {name:?} can't be optional"),
            )),
            Some(field) if !is_keyable(&field.ty) => diags.push(Diagnostic::new(
                Code::InvalidKey,
                &key_path,
                format!("key field {name:?} must be a string, int, bool, enum, id, or timestamp"),
            )),
            Some(_) => {}
        }
    }

    for (index_name, index) in &entity.indexes {
        let index_path = format!("{path}.indexes.{index_name}");
        check_name(index_name, &index_path, diags);
        if index.fields.is_empty() {
            diags.push(Diagnostic::new(
                Code::InvalidIndex,
                &index_path,
                "an index needs at least one field",
            ));
        }
        for name in &index.fields {
            match entity.fields.get(name) {
                None => diags.push(unknown_field(&index_path, name, entity)),
                Some(field) if !is_keyable(&field.ty) => diags.push(Diagnostic::new(
                    Code::InvalidIndex,
                    &index_path,
                    format!("field {name:?} can't be indexed: arrays aren't orderable"),
                )),
                Some(_) => {}
            }
        }
    }

    let row_bytes = size::row_bytes(entity);
    if row_bytes > limits.max_row_bytes {
        diags.push(Diagnostic::new(
            Code::RowTooLarge,
            path,
            format!(
                "a row can be up to {row_bytes} bytes, over the {} byte limit. Lower string and array maximums, \
                 or keep large content in a bucket and store its key.",
                limits.max_row_bytes
            ),
        ));
    }
    row_bytes
}

fn is_keyable(ty: &FieldType) -> bool {
    !matches!(ty, FieldType::Array { .. })
}

pub(crate) fn check_field(field: &Field, path: &str, diags: &mut Vec<Diagnostic>) {
    check_field_type(&field.ty, path, diags);
    let Some(default) = &field.default else {
        return;
    };
    let ok = match (default, &field.ty) {
        (DefaultValue::Now, FieldType::Timestamp) | (DefaultValue::NewId, FieldType::Id) => true,
        (DefaultValue::Literal { value }, ty) => literal_fits(value, ty, field.optional),
        _ => false,
    };
    if !ok {
        diags.push(Diagnostic::new(
            Code::InvalidDefault,
            format!("{path}.default"),
            "default doesn't match the field type (`now` is only for timestamps, `newId` only for ids)",
        ));
    }
}

fn check_field_type(ty: &FieldType, path: &str, diags: &mut Vec<Diagnostic>) {
    let problem = match ty {
        FieldType::String { min, max } if min > max => {
            Some("string min is greater than max".to_owned())
        }
        FieldType::Int {
            min: Some(min),
            max: Some(max),
        } if min > max => Some("int min is greater than max".to_owned()),
        FieldType::Enum { values } if values.is_empty() => {
            Some("an enum needs at least one value".to_owned())
        }
        FieldType::Enum { values }
            if values.iter().collect::<BTreeSet<_>>().len() != values.len() =>
        {
            Some("enum values must be unique".to_owned())
        }
        FieldType::Array { items, .. } => {
            if matches!(**items, FieldType::Array { .. }) {
                Some("nested arrays aren't supported".to_owned())
            } else {
                check_field_type(items, &format!("{path}.items"), diags);
                None
            }
        }
        _ => None,
    };
    if let Some(message) = problem {
        diags.push(Diagnostic::new(Code::InvalidFieldType, path, message));
    }
}

fn literal_fits(value: &Literal, ty: &FieldType, optional: bool) -> bool {
    match (value, ty) {
        (Literal::Null, _) => optional,
        (Literal::Bool(_), FieldType::Bool) | (Literal::Int(_), FieldType::Timestamp) => true,
        (Literal::Int(i), FieldType::Int { min, max }) => {
            min.is_none_or(|m| *i >= m) && max.is_none_or(|m| *i <= m)
        }
        (Literal::String(s), FieldType::String { min, max }) => {
            let len = s.chars().count();
            len >= *min as usize && len <= *max as usize
        }
        (Literal::String(s), FieldType::Enum { values }) => values.contains(s),
        _ => false,
    }
}

fn check_counter(m: &Manifest, counter: &Counter, path: &str, diags: &mut Vec<Diagnostic>) {
    let Some(entity) = m.entities.get(&counter.of) else {
        diags.push(unknown_entity(&format!("{path}.of"), &counter.of, m));
        return;
    };
    for name in &counter.group_by {
        match entity.fields.get(name) {
            None => diags.push(unknown_field(&format!("{path}.groupBy"), name, entity)),
            Some(field) if !is_keyable(&field.ty) => diags.push(Diagnostic::new(
                Code::InvalidCounterKey,
                format!("{path}.groupBy"),
                format!("can't group by array field {name:?}"),
            )),
            Some(_) => {}
        }
    }
    if let Some(filter) = &counter.filter {
        plan::check_row_predicate(m, &counter.of, filter, &format!("{path}.where"), diags);
    }
}

fn check_auth_config(auth: &AuthConfig, diags: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    for role in &auth.roles {
        check_name(role, "auth.roles", diags);
        if !seen.insert(role) {
            diags.push(Diagnostic::new(
                Code::InvalidAuth,
                "auth.roles",
                format!("role {role:?} is declared twice"),
            ));
        }
    }
    let mut seen = HashSet::new();
    for scope in &auth.api_key_scopes {
        let valid = (1..=64).contains(&scope.len())
            && scope
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_:.-".contains(&b));
        if !valid {
            diags.push(Diagnostic::new(
                Code::InvalidAuth,
                "auth.apiKeyScopes",
                format!("{scope:?} isn't a valid scope: use 1-64 characters of [A-Za-z0-9_:.-]"),
            ));
        }
        if !seen.insert(scope) {
            diags.push(Diagnostic::new(
                Code::InvalidAuth,
                "auth.apiKeyScopes",
                format!("scope {scope:?} is declared twice"),
            ));
        }
    }
}

fn check_route_auth(config: &AuthConfig, route: &Route, path: &str, diags: &mut Vec<Diagnostic>) {
    let Auth::Member {
        org,
        roles,
        api_key_scopes,
    } = &route.auth
    else {
        return;
    };
    let auth_path = format!("{path}.auth");
    if !route.input.params.contains_key(org) {
        diags.push(Diagnostic::new(
            Code::InvalidAuth,
            &auth_path,
            format!("member auth names org param {org:?}, but the path has no :{org} parameter"),
        ));
    }
    for role in roles.iter().filter(|r| !config.roles.contains(r)) {
        diags.push(Diagnostic::new(
            Code::UnknownRole,
            &auth_path,
            format!(
                "role {role:?} isn't declared (auth.roles: {:?})",
                config.roles
            ),
        ));
    }
    for scope in api_key_scopes
        .iter()
        .filter(|s| !config.api_key_scopes.contains(s))
    {
        diags.push(Diagnostic::new(
            Code::UnknownScope,
            &auth_path,
            format!(
                "scope {scope:?} isn't declared (auth.apiKeyScopes: {:?})",
                config.api_key_scopes
            ),
        ));
    }
}

fn check_idempotency(idempotency: Idempotency, path: &str, diags: &mut Vec<Diagnostic>) {
    if idempotency.ttl_ms == 0 || idempotency.ttl_ms > Idempotency::MAX_TTL_MS {
        diags.push(Diagnostic::new(
            Code::InvalidIdempotency,
            path,
            format!(
                "ttlMs must be between 1 and {} (7 days), got {}",
                Idempotency::MAX_TTL_MS,
                idempotency.ttl_ms
            ),
        ));
    }
}

fn check_route_table(routes: &[Route], diags: &mut Vec<Diagnostic>) {
    let mut ids = HashSet::new();
    let mut shapes = HashSet::new();
    for (i, route) in routes.iter().enumerate() {
        let path = format!("routes[{i}]");
        if !ids.insert(&route.id) {
            diags.push(Diagnostic::new(
                Code::DuplicateRouteId,
                format!("{path}.id"),
                format!("route id {:?} is used twice", route.id),
            ));
        }
        check_name(&route.id, &format!("{path}.id"), diags);

        let segments: Vec<&str> = route.path.split('/').skip(1).collect();
        if !route.path.starts_with('/')
            || (route.path.len() > 1 && segments.iter().any(|s| s.is_empty()))
        {
            diags.push(Diagnostic::new(
                Code::InvalidPath,
                format!("{path}.path"),
                format!(
                    "{:?} isn't a valid path: it must start with / and have no empty segments",
                    route.path
                ),
            ));
            continue;
        }
        let params: BTreeSet<&str> = segments
            .iter()
            .filter_map(|s| s.strip_prefix(':'))
            .collect();
        let declared: BTreeSet<&str> = route.input.params.keys().map(String::as_str).collect();
        if params != declared {
            diags.push(Diagnostic::new(
                Code::PathParamMismatch,
                format!("{path}.input.params"),
                format!("path parameters {params:?} don't match declared params {declared:?}"),
            ));
        }
        // Two routes conflict if they match the same requests, whatever their param names.
        let shape: Vec<&str> = segments
            .iter()
            .map(|s| if s.starts_with(':') { ":" } else { s })
            .collect();
        if !shapes.insert((route.method, shape)) {
            diags.push(Diagnostic::new(
                Code::DuplicateRoute,
                path.clone(),
                format!(
                    "another route already handles {} {}",
                    route.method.as_str(),
                    route.path
                ),
            ));
        }

        for (source, fields) in [
            ("params", &route.input.params),
            ("query", &route.input.query),
            ("body", &route.input.body),
        ] {
            for (name, field) in fields {
                let field_path = format!("{path}.input.{source}.{name}");
                check_name(name, &field_path, diags);
                check_field(field, &field_path, diags);
            }
        }
        for (name, error) in &route.errors {
            let error_path = format!("{path}.errors.{name}");
            check_name(name, &error_path, diags);
            if !(400..=599).contains(&error.status) {
                diags.push(Diagnostic::new(
                    Code::InvalidErrorStatus,
                    error_path,
                    format!("error status {} must be between 400 and 599", error.status),
                ));
            }
        }
    }
}

pub(crate) fn unknown_entity(path: &str, name: &str, m: &Manifest) -> Diagnostic {
    let known: Vec<&str> = m.entities.keys().map(String::as_str).collect();
    Diagnostic::new(
        Code::UnknownEntity,
        path,
        format!("unknown entity {name:?} (declared: {known:?})"),
    )
}

pub(crate) fn unknown_field(path: &str, name: &str, entity: &Entity) -> Diagnostic {
    let known: Vec<&str> = entity.fields.keys().map(String::as_str).collect();
    Diagnostic::new(
        Code::UnknownField,
        path,
        format!("unknown field {name:?} (fields: {known:?})"),
    )
}
