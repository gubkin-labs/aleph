//! Input decoding and value validation. Every input is checked against its declared field before
//! the plan runs, and every row is checked against its entity before it's written.

use std::collections::BTreeMap;

use serde::Serialize;

use super::env::{Env, is_ulid};
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
        problems.push(problem(source, name, "unknown input"));
    }
    let mut out = BTreeMap::new();
    for (name, field) in fields {
        let value = match raw.get(name) {
            Some(text) => match parse_string(&field.ty, text) {
                Ok(value) => value,
                Err(message) => {
                    problems.push(problem(source, name, message));
                    continue;
                }
            },
            None => match missing(field, env, now_ms) {
                Some(value) => value,
                None => {
                    problems.push(problem(source, name, "required"));
                    continue;
                }
            },
        };
        if let Err(message) = check_value(field, &value) {
            problems.push(problem(source, name, message));
        } else {
            out.insert(name.clone(), value);
        }
    }
    out
}

/// Decodes a JSON body according to its declared fields.
pub(crate) fn decode_body(
    fields: &BTreeMap<String, Field>,
    body: Option<&serde_json::Value>,
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
    for name in object.keys().filter(|name| !fields.contains_key(*name)) {
        problems.push(problem("body", name, "unknown field"));
    }
    let mut out = BTreeMap::new();
    for (name, field) in fields {
        let value = match object.get(name) {
            Some(json) => match Value::from_json(json.clone()) {
                Ok(value) => value,
                Err(message) => {
                    problems.push(problem("body", name, message));
                    continue;
                }
            },
            None => match missing(field, env, now_ms) {
                Some(value) => value,
                None => {
                    problems.push(problem("body", name, "required"));
                    continue;
                }
            },
        };
        match check_value(field, &value) {
            Ok(()) => {
                out.insert(name.clone(), value);
            }
            Err(message) => problems.push(problem("body", name, message)),
        }
    }
    out
}

fn problem(source: &str, name: &str, message: impl Into<String>) -> InputProblem {
    InputProblem {
        field: format!("{source}.{name}"),
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
        FieldType::String { .. } | FieldType::Enum { .. } | FieldType::Id => {
            Ok(Value::String(text.to_owned()))
        }
        FieldType::Int { .. } | FieldType::Timestamp => text
            .parse()
            .map(Value::Int)
            .map_err(|_| format!("{text:?} isn't an integer")),
        FieldType::Bool => match text {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(format!("{text:?} isn't true or false")),
        },
        FieldType::Array { .. } => Err("arrays can only be sent in the body".into()),
    }
}

/// Checks a value against a field declaration.
pub(crate) fn check_value(field: &Field, value: &Value) -> Result<(), String> {
    if value.is_null() {
        return if field.optional {
            Ok(())
        } else {
            Err("can't be null".into())
        };
    }
    check_type(&field.ty, value)
}

fn check_type(ty: &FieldType, value: &Value) -> Result<(), String> {
    match (ty, value) {
        (FieldType::String { min, max }, Value::String(s)) => {
            let len = s.chars().count();
            if len < *min as usize || len > *max as usize {
                return Err(format!("length must be between {min} and {max}, got {len}"));
            }
            Ok(())
        }
        (FieldType::Int { min, max }, Value::Int(i)) => {
            if min.is_some_and(|m| *i < m) || max.is_some_and(|m| *i > m) {
                let range = |b: Option<i64>| b.map_or_else(|| "∞".to_owned(), |b| b.to_string());
                return Err(format!(
                    "must be between {} and {}, got {i}",
                    range(*min),
                    range(*max)
                ));
            }
            Ok(())
        }
        (FieldType::Bool, Value::Bool(_)) | (FieldType::Timestamp, Value::Int(_)) => Ok(()),
        (FieldType::Enum { values }, Value::String(s)) => {
            if values.contains(s) {
                Ok(())
            } else {
                Err(format!("must be one of {values:?}, got {s:?}"))
            }
        }
        (FieldType::Id, Value::String(s)) => {
            if is_ulid(s) {
                Ok(())
            } else {
                Err(format!("{s:?} isn't a valid id"))
            }
        }
        (FieldType::Array { items, max }, Value::List(list)) => {
            if list.len() > *max as usize {
                return Err(format!("can have at most {max} items, got {}", list.len()));
            }
            for (i, item) in list.iter().enumerate() {
                check_type(items, item).map_err(|e| format!("item {i}: {e}"))?;
            }
            Ok(())
        }
        (ty, value) => Err(format!(
            "expected {}, got {}",
            type_name(ty),
            value.type_name()
        )),
    }
}

fn type_name(ty: &FieldType) -> &'static str {
    match ty {
        FieldType::String { .. } => "string",
        FieldType::Int { .. } => "int",
        FieldType::Bool => "bool",
        FieldType::Enum { .. } => "enum value",
        FieldType::Id => "id",
        FieldType::Timestamp => "timestamp",
        FieldType::Array { .. } => "array",
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
        }
    }

    #[test]
    fn checks_string_lengths_in_characters() {
        let f = field(FieldType::String { min: 1, max: 3 });
        assert!(check_value(&f, &Value::String("héé".into())).is_ok());
        assert!(check_value(&f, &Value::String(String::new())).is_err());
        assert!(check_value(&f, &Value::String("abcd".into())).is_err());
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
