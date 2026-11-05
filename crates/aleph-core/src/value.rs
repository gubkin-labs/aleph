//! Runtime values.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::manifest::Literal;

/// A value produced while running a plan. Rows are objects.
///
/// Numbers are 64-bit integers only. Floats aren't part of the v0 format.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// `null`.
    #[default]
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A string.
    String(String),
    /// A list.
    List(Vec<Value>),
    /// An object (also used for rows).
    Object(BTreeMap<String, Value>),
}

impl Value {
    /// A short name for the value's type, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::String(_) => "string",
            Self::List(_) => "list",
            Self::Object(_) => "object",
        }
    }

    /// Whether the value is `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Converts a JSON value. Fails on non-integer numbers.
    pub fn from_json(json: serde_json::Value) -> Result<Self, String> {
        Ok(match json {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(b),
            serde_json::Value::Number(n) => Self::Int(
                n.as_i64()
                    .ok_or_else(|| format!("{n} is not a 64-bit integer"))?,
            ),
            serde_json::Value::String(s) => Self::String(s),
            serde_json::Value::Array(items) => Self::List(
                items
                    .into_iter()
                    .map(Self::from_json)
                    .collect::<Result<_, _>>()?,
            ),
            serde_json::Value::Object(map) => Self::Object(
                map.into_iter()
                    .map(|(k, v)| Ok((k, Self::from_json(v)?)))
                    .collect::<Result<_, String>>()?,
            ),
        })
    }

    /// Converts to a JSON value.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Int(i) => serde_json::Value::from(*i),
            Self::String(s) => serde_json::Value::String(s.clone()),
            Self::List(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json).collect())
            }
            Self::Object(map) => serde_json::Value::Object(
                map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect(),
            ),
        }
    }
}

impl From<&Literal> for Value {
    fn from(literal: &Literal) -> Self {
        match literal {
            Literal::Null => Self::Null,
            Literal::Bool(b) => Self::Bool(*b),
            Literal::Int(i) => Self::Int(*i),
            Literal::String(s) => Self::String(s.clone()),
        }
    }
}

impl fmt::Display for Value {
    /// Formats the value for string concatenation: strings unquoted, everything else as JSON.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) => f.write_str(s),
            other => write!(f, "{}", other.to_json()),
        }
    }
}
