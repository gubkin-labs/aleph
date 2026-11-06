//! Runtime values.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::decimal::Decimal;
use crate::manifest::Literal;

/// How decimals are tagged in stored JSON, so they round-trip without a schema. Field names are
/// identifiers, so no declared object can have this key.
const DECIMAL_TAG: &str = "$dec";

/// A value produced while running a plan. Rows are objects.
///
/// Numbers are 64-bit integers or exact decimals; there are no floats.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Value {
    /// `null`.
    #[default]
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// An exact decimal.
    Decimal(Decimal),
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
            Self::Decimal(_) => "decimal",
            Self::String(_) => "string",
            Self::List(_) => "list",
            Self::Object(_) => "object",
        }
    }

    /// Whether the value is `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Converts a JSON value from a client. Fails on non-integer numbers, which only decimal
    /// fields accept (see [`crate::runtime`]'s input validation).
    pub fn from_json(json: serde_json::Value) -> Result<Self, String> {
        Ok(match json {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Self::Int(i),
                // Kept exact as a decimal; only decimal fields accept it.
                None => Self::Decimal(
                    Decimal::parse(&n.to_string())
                        .ok_or_else(|| format!("{n} is not an integer or a decimal"))?,
                ),
            },
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

    /// Converts to a JSON value for clients. Decimals become strings (`"12.50"`).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Int(i) => serde_json::Value::from(*i),
            Self::Decimal(d) => serde_json::Value::String(d.to_string()),
            Self::String(s) => serde_json::Value::String(s.clone()),
            Self::List(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json).collect())
            }
            Self::Object(map) => serde_json::Value::Object(
                map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect(),
            ),
        }
    }

    /// An approximate size in bytes, for the memory meter. Never more than the static size bound
    /// of the value's type (see `check::size`).
    pub fn size(&self) -> u64 {
        match self {
            Self::Null => 4,
            Self::Bool(_) => 5,
            Self::Int(_) => 20,
            Self::Decimal(_) => 42,
            Self::String(s) => 2 + s.len() as u64,
            Self::List(items) => 2 + items.iter().map(|v| v.size() + 1).sum::<u64>(),
            Self::Object(map) => {
                2 + map
                    .iter()
                    .map(|(k, v)| k.len() as u64 + 4 + v.size().max(4))
                    .sum::<u64>()
            }
        }
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(b) => serializer.serialize_bool(*b),
            Self::Int(i) => serializer.serialize_i64(*i),
            Self::String(s) => serializer.serialize_str(s),
            Self::Decimal(d) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(DECIMAL_TAG, &d.to_string())?;
                map.end()
            }
            Self::List(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Object(fields) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (k, v) in fields {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

/// Reads stored values directly (no intermediate JSON tree): this is on every row read.
struct ValueVisitor;

impl<'de> serde::de::Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a stored Aleph value")
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, b: bool) -> Result<Value, E> {
        Ok(Value::Bool(b))
    }

    fn visit_i64<E>(self, i: i64) -> Result<Value, E> {
        Ok(Value::Int(i))
    }

    fn visit_u64<E: serde::de::Error>(self, u: u64) -> Result<Value, E> {
        i64::try_from(u)
            .map(Value::Int)
            .map_err(|_| E::custom(format!("{u} is not a 64-bit integer")))
    }

    fn visit_f64<E: serde::de::Error>(self, f: f64) -> Result<Value, E> {
        Err(E::custom(format!("{f} is not an integer")))
    }

    fn visit_str<E>(self, s: &str) -> Result<Value, E> {
        Ok(Value::String(s.to_owned()))
    }

    fn visit_string<E>(self, s: String) -> Result<Value, E> {
        Ok(Value::String(s))
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element::<Value>()? {
            items.push(item);
        }
        Ok(Value::List(items))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut fields = BTreeMap::new();
        while let Some((key, value)) = map.next_entry::<String, Value>()? {
            fields.insert(key, value);
        }
        if fields.len() == 1
            && let Some(Value::String(text)) = fields.get(DECIMAL_TAG)
        {
            return Decimal::parse(text)
                .map(Value::Decimal)
                .ok_or_else(|| serde::de::Error::custom("corrupt stored decimal"));
        }
        Ok(Value::Object(fields))
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
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
            Self::Decimal(d) => write!(f, "{d}"),
            other => write!(f, "{}", other.to_json()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_round_trip_through_storage() {
        let value = Value::Object(BTreeMap::from([
            (
                "price".into(),
                Value::Decimal(Decimal::parse("12.50").unwrap()),
            ),
            ("n".into(), Value::Int(3)),
            ("s".into(), Value::String("12.50".into())),
        ]));
        let bytes = serde_json::to_vec(&value).unwrap();
        let back: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, value);
        assert_eq!(
            value.to_json(),
            serde_json::json!({ "price": "12.50", "n": 3, "s": "12.50" })
        );
    }
}
