//! Input decoding and value validation. Every input is checked against its declared field before
//! the plan runs, and every row is checked against its entity before it's written.

use std::collections::BTreeMap;

use serde::Serialize;

use super::env::{Env, is_ulid};
use crate::decimal::Decimal;
use crate::manifest::{DefaultValue, Field, FieldType};
use crate::value::Value;

/// One invalid input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputProblem {
    /// The input, for example `body.name`.
    pub field: String,
    /// What's wrong with it.
    pub message: String,
}

/// Decoded, validated inputs of one request.
#[derive(Debug, Clone, Default)]
pub(crate) struct Inputs {
    pub(crate) params: BTreeMap<String, Value>,
    pub(crate) query: BTreeMap<String, Value>,
    pub(crate) body: BTreeMap<String, Value>,
    /// A handler's payload.
    pub(crate) payload: BTreeMap<String, Value>,
}

/// Decodes string inputs (path params, query string) according to their declared fields.
pub(crate) fn decode_strings(
    source: &str,
    fields: &BTreeMap<String, Field>,
    raw: &BTreeMap<String, String>,
    env: &dyn Env,
    now_ms: i64,
    problems: &mut Vec<InputProblem>,
) -> BTreeMap<String, Value> {
    for name in raw.keys().filter(|name| !fields.contains_key(*name)) {
        problems.push(problem(source, name, "", "unknown input"));
    }
    let mut out = BTreeMap::new();
    for (name, field) in fields {
        let value = match raw.get(name) {
            Some(text) => match parse_string(&field.ty, text) {
                Ok(value) => value,
                Err(message) => {
                    problems.push(problem(source, name, "", message));
                    continue;
                }
            },
            None => match missing(field, env, now_ms) {
                Some(value) => value,
                None => {
                    problems.push(problem(source, name, "", "required"));
                    continue;
                }
            },
        };
        match conform(field, value) {
            Ok(value) => {
                out.insert(name.clone(), value);
            }
            Err(p) => problems.push(problem(source, name, &p.at, p.message)),
        }
    }
    out
}

/// Decodes a JSON body according to its declared fields.
/// With `lenient` (webhooks, whose senders add fields over time), undeclared fields are ignored
/// instead of rejected.
pub(crate) fn decode_body(
    fields: &BTreeMap<String, Field>,
    body: Option<&serde_json::Value>,
    lenient: bool,
    env: &dyn Env,
    now_ms: i64,
    problems: &mut Vec<InputProblem>,
) -> BTreeMap<String, Value> {
    let empty = serde_json::Map::new();
    let object = match body {
        None | Some(serde_json::Value::Null) => &empty,
        Some(serde_json::Value::Object(map)) => map,
        Some(_) => {
            problems.push(InputProblem {
                field: "body".into(),
                message: "must be a JSON object".into(),
            });
            return BTreeMap::new();
        }
    };
    if !lenient {
        for name in object.keys().filter(|name| !fields.contains_key(*name)) {
            problems.push(problem("body", name, "", "unknown field"));
        }
    }
    let mut out = BTreeMap::new();
    for (name, field) in fields {
        let value = match object.get(name) {
            Some(json) => match Value::from_json(json.clone()) {
                Ok(value) if lenient => project(&field.ty, value),
                Ok(value) => value,
                Err(message) => {
                    problems.push(problem("body", name, "", message));
                    continue;
                }
            },
            None => match missing(field, env, now_ms) {
                Some(value) => value,
                None => {
                    problems.push(problem("body", name, "", "required"));
                    continue;
                }
            },
        };
        match conform(field, value) {
            Ok(value) => {
                out.insert(name.clone(), value);
            }
            Err(p) => problems.push(problem("body", name, &p.at, p.message)),
        }
    }
    out
}

fn problem(source: &str, name: &str, at: &str, message: impl Into<String>) -> InputProblem {
    InputProblem {
        field: format!("{source}.{name}{at}"),
        message: message.into(),
    }
}

/// The value of a field that wasn't provided: its default, `null` if optional, or `None` if it's
/// required.
pub(crate) fn missing(field: &Field, env: &dyn Env, now_ms: i64) -> Option<Value> {
    match &field.default {
        Some(DefaultValue::Literal { value }) => Some(Value::from(value)),
        Some(DefaultValue::Now) => Some(Value::Int(now_ms)),
        Some(DefaultValue::NewId) => Some(Value::String(env.new_id(now_ms))),
        None if field.optional => Some(Value::Null),
        None => None,
    }
}

fn parse_string(ty: &FieldType, text: &str) -> Result<Value, String> {
    match ty {
        FieldType::String { .. } | FieldType::Enum { .. } | FieldType::Id | FieldType::TimeZone => {
            Ok(Value::String(text.to_owned()))
        }
        FieldType::Int { .. } | FieldType::Timestamp => text
            .parse()
            .map(Value::Int)
            .map_err(|_| format!("{text:?} isn't an integer")),
        FieldType::Decimal { .. } => Decimal::parse(text)
            .map(Value::Decimal)
            .ok_or_else(|| format!("{text:?} isn't a decimal number")),
        FieldType::Bool => match text {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(format!("{text:?} isn't true or false")),
        },
        FieldType::Array { .. } | FieldType::Object { .. } => {
            Err("lists and objects can only be sent in the body".into())
        }
    }
}

/// A value that doesn't fit its field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Problem {
    /// Where inside the value, like `.items.2.qty` (empty for the value itself).
    pub(crate) at: String,
    pub(crate) message: String,
}

impl Problem {
    fn new(message: impl Into<String>) -> Self {
        Self {
            at: String::new(),
            message: message.into(),
        }
    }

    fn inside(mut self, part: &str) -> Self {
        self.at = format!(".{part}{}", self.at);
        self
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.at.strip_prefix('.') {
            Some(at) => write!(f, "{at}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Checks a value against a field declaration and returns it in canonical form: decimals at the
/// field's scale (from strings or numbers), and objects with every declared field present.
pub(crate) fn conform(field: &Field, value: Value) -> Result<Value, Problem> {
    if value.is_null() {
        return if field.optional {
            Ok(Value::Null)
        } else {
            Err(Problem::new("can't be null"))
        };
    }
    conform_type(&field.ty, value)
}

/// Shapes a response to a route's declared `returns` type: object fields that aren't declared
/// are dropped (so a row can be returned as a narrower object), then the value is checked.
pub(crate) fn conform_output(ty: &FieldType, value: Value) -> Result<Value, Problem> {
    let field = Field {
        ty: ty.clone(),
        optional: false,
        default: None,
        deprecated: None,
    };
    conform(&field, project(ty, value))
}

fn project(ty: &FieldType, value: Value) -> Value {
    match (ty, value) {
        (FieldType::Object { fields }, Value::Object(object)) => Value::Object(
            object
                .into_iter()
                .filter_map(|(name, v)| {
                    let field = fields.get(&name)?;
                    Some((name, project(&field.ty, v)))
                })
                .collect(),
        ),
        (FieldType::Array { items, .. }, Value::List(list)) => {
            Value::List(list.into_iter().map(|v| project(items, v)).collect())
        }
        (_, value) => value,
    }
}

#[allow(clippy::too_many_lines, reason = "one arm per field type")]
fn conform_type(ty: &FieldType, value: Value) -> Result<Value, Problem> {
    match (ty, value) {
        (FieldType::String { min, max }, Value::String(s)) => {
            let len = s.chars().count();
            if len < *min as usize || len > *max as usize {
                return Err(Problem::new(format!(
                    "length must be between {min} and {max}, got {len}"
                )));
            }
            Ok(Value::String(s))
        }
        (FieldType::Int { min, max }, Value::Int(i)) => {
            if min.is_some_and(|m| i < m) || max.is_some_and(|m| i > m) {
                let range = |b: Option<i64>| b.map_or_else(|| "∞".to_owned(), |b| b.to_string());
                return Err(Problem::new(format!(
                    "must be between {} and {}, got {i}",
                    range(*min),
                    range(*max)
                )));
            }
            Ok(Value::Int(i))
        }
        (FieldType::Decimal { scale, min, max }, value) => {
            let decimal = match value {
                Value::Decimal(d) => d,
                Value::Int(i) => Decimal::from_int(i),
                Value::String(s) => Decimal::parse(&s)
                    .ok_or_else(|| Problem::new(format!("{s:?} isn't a decimal number")))?,
                other => {
                    return Err(Problem::new(format!(
                        "expected a decimal, got {}",
                        other.type_name()
                    )));
                }
            };
            let decimal = decimal.exactly_at(*scale).ok_or_else(|| {
                Problem::new(format!(
                    "{decimal} has more than {scale} digits after the point"
                ))
            })?;
            let bound = |b: &Option<String>| b.as_deref().and_then(Decimal::parse);
            if bound(min).is_some_and(|m| decimal < m) || bound(max).is_some_and(|m| decimal > m) {
                return Err(Problem::new(format!(
                    "must be between {} and {}, got {decimal}",
                    min.as_deref().unwrap_or("-∞"),
                    max.as_deref().unwrap_or("∞")
                )));
            }
            Ok(Value::Decimal(decimal))
        }
        (FieldType::Bool, Value::Bool(b)) => Ok(Value::Bool(b)),
        (FieldType::Timestamp, Value::Int(i)) => Ok(Value::Int(i)),
        (FieldType::Enum { values }, Value::String(s)) => {
            if values.contains(&s) {
                Ok(Value::String(s))
            } else {
                Err(Problem::new(format!(
                    "must be one of {values:?}, got {s:?}"
                )))
            }
        }
        (FieldType::Id, Value::String(s)) => {
            if is_ulid(&s) {
                Ok(Value::String(s))
            } else {
                Err(Problem::new(format!("{s:?} isn't a valid id")))
            }
        }
        (FieldType::TimeZone, Value::String(s)) => {
            if super::time::zone(&s).is_some() {
                Ok(Value::String(s))
            } else {
                Err(Problem::new(format!("{s:?} isn't an IANA time zone")))
            }
        }
        (FieldType::Array { items, max }, Value::List(list)) => {
            if list.len() > *max as usize {
                return Err(Problem::new(format!(
                    "can have at most {max} items, got {}",
                    list.len()
                )));
            }
            let items_field = Field {
                ty: (**items).clone(),
                optional: false,
                default: None,
                deprecated: None,
            };
            list.into_iter()
                .enumerate()
                .map(|(i, item)| conform(&items_field, item).map_err(|p| p.inside(&i.to_string())))
                .collect::<Result<_, _>>()
                .map(Value::List)
        }
        (FieldType::Object { fields }, Value::Object(mut object)) => {
            if let Some(unknown) = object.keys().find(|k| !fields.contains_key(*k)) {
                return Err(Problem::new("unknown field").inside(unknown));
            }
            let mut out = BTreeMap::new();
            for (name, field) in fields {
                let value = match object.remove(name) {
                    Some(value) => value,
                    None if field.optional => Value::Null,
                    None => return Err(Problem::new("required").inside(name)),
                };
                out.insert(
                    name.clone(),
                    conform(field, value).map_err(|p| p.inside(name))?,
                );
            }
            Ok(Value::Object(out))
        }
        (ty, value) => Err(Problem::new(format!(
            "expected {}, got {}",
            type_name(ty),
            value.type_name()
        ))),
    }
}

fn type_name(ty: &FieldType) -> &'static str {
    match ty {
        FieldType::String { .. } => "string",
        FieldType::Int { .. } => "int",
        FieldType::Decimal { .. } => "decimal",
        FieldType::Bool => "bool",
        FieldType::Enum { .. } => "enum value",
        FieldType::Id => "id",
        FieldType::Timestamp => "timestamp",
        FieldType::TimeZone => "time zone",
        FieldType::Array { .. } => "array",
        FieldType::Object { .. } => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::env::FixedEnv;

    fn field(ty: FieldType) -> Field {
        Field {
            ty,
            optional: false,
            default: None,
            deprecated: None,
        }
    }

    #[test]
    fn checks_string_lengths_in_characters() {
        let f = field(FieldType::String { min: 1, max: 3 });
        assert!(conform(&f, Value::String("héé".into())).is_ok());
        assert!(conform(&f, Value::String(String::new())).is_err());
        assert!(conform(&f, Value::String("abcd".into())).is_err());
    }

    #[test]
    fn reports_nested_problems_with_paths() {
        let item = FieldType::Object {
            fields: BTreeMap::from([
                (
                    "sku".to_owned(),
                    field(FieldType::String { min: 1, max: 8 }),
                ),
                (
                    "qty".to_owned(),
                    field(FieldType::Int {
                        min: Some(1),
                        max: None,
                    }),
                ),
                (
                    "note".to_owned(),
                    Field {
                        optional: true,
                        ..field(FieldType::String { min: 0, max: 8 })
                    },
                ),
            ]),
        };
        let items = BTreeMap::from([(
            "items".to_owned(),
            field(FieldType::Array {
                items: Box::new(item),
                max: 5,
            }),
        )]);
        let body =
            serde_json::json!({ "items": [{ "sku": "a", "qty": 1 }, { "sku": "b", "qty": 0 }] });
        let mut problems = Vec::new();
        decode_body(
            &items,
            Some(&body),
            false,
            &FixedEnv::new(0, 0),
            0,
            &mut problems,
        );
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].field, "body.items.1.qty");
        let ok = serde_json::json!({ "items": [{ "sku": "a", "qty": 2 }] });
        let mut problems = Vec::new();
        let out = decode_body(
            &items,
            Some(&ok),
            false,
            &FixedEnv::new(0, 0),
            0,
            &mut problems,
        );
        assert_eq!(problems, []);
        // Missing optional fields are filled in as null.
        assert_eq!(
            out["items"],
            Value::List(vec![Value::Object(BTreeMap::from([
                ("note".to_owned(), Value::Null),
                ("qty".to_owned(), Value::Int(2)),
                ("sku".to_owned(), Value::String("a".into())),
            ]))])
        );
    }

    #[test]
    fn decimals_conform_to_their_scale() {
        let f = field(FieldType::Decimal {
            scale: 2,
            min: Some("0".into()),
            max: None,
        });
        let d = |s: &str| Value::Decimal(Decimal::parse(s).unwrap());
        assert_eq!(
            conform(&f, Value::String("12.5".into())).unwrap(),
            d("12.50")
        );
        assert_eq!(conform(&f, Value::Int(3)).unwrap(), d("3.00"));
        assert!(conform(&f, Value::String("1.234".into())).is_err());
        assert!(conform(&f, Value::String("-1".into())).is_err());
        let json = Value::from_json(serde_json::json!(19.99)).unwrap();
        assert_eq!(conform(&f, json).unwrap(), d("19.99"));
    }

    #[test]
    fn decodes_query_strings_with_defaults_and_problems() {
        let env = FixedEnv::new(0, 0);
        let fields = BTreeMap::from([
            (
                "limit".to_owned(),
                Field {
                    default: Some(DefaultValue::Literal {
                        value: crate::manifest::Literal::Int(20),
                    }),
                    ..field(FieldType::Int {
                        min: Some(1),
                        max: Some(50),
                    })
                },
            ),
            ("archived".to_owned(), field(FieldType::Bool)),
        ]);
        let mut problems = Vec::new();
        let raw = BTreeMap::from([
            ("archived".to_owned(), "yes".to_owned()),
            ("extra".to_owned(), "1".to_owned()),
        ]);
        let out = decode_strings("query", &fields, &raw, &env, 0, &mut problems);
        assert_eq!(out.get("limit"), Some(&Value::Int(20)));
        let fields_with_problems: Vec<&str> = problems.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(fields_with_problems, ["query.extra", "query.archived"]);
    }
}
