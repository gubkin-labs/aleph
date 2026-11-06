//! Pure built-ins: arithmetic over ints and decimals, ordering, and string helpers. The
//! interpreter (`exec.rs`) evaluates operands and charges the meter; these only compute.

use std::cmp::Ordering;

use crate::decimal::{Decimal, divide};
use crate::manifest::{BinaryOp, Rounding};
use crate::value::Value;

/// The longest string a plan may build, in bytes. Static bounds keep real strings far smaller;
/// this stops pathological ones early.
pub(crate) const MAX_STRING_BYTES: usize = 1024 * 1024;

fn overflow() -> String {
    "number out of range".into()
}

/// Arithmetic. Ints stay ints; if either side is a decimal, both are.
pub(crate) fn arithmetic(op: BinaryOp, a: &Value, b: &Value) -> Result<Value, String> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => int_arithmetic(op, *x, *y).map(Value::Int),
        (Value::Int(_) | Value::Decimal(_), Value::Int(_) | Value::Decimal(_)) => {
            let (x, y) = (as_decimal(a), as_decimal(b));
            let result = match op {
                BinaryOp::Add => x.plus(y),
                BinaryOp::Sub => x.minus(y),
                BinaryOp::Mul => x.times(y),
                BinaryOp::Div => {
                    if y.units() == 0 {
                        return Err("division by zero".into());
                    }
                    x.div(y, x.scale().max(y.scale()), Rounding::HalfEven)
                }
                BinaryOp::Min => Some(x.min(y)),
                BinaryOp::Max => Some(x.max(y)),
                other => return Err(format!("{other:?} needs ints, not decimals")),
            };
            result.map(Value::Decimal).ok_or_else(overflow)
        }
        _ => Err(format!(
            "arithmetic needs numbers, got {} and {}",
            a.type_name(),
            b.type_name()
        )),
    }
}

fn int_arithmetic(op: BinaryOp, x: i64, y: i64) -> Result<i64, String> {
    let needs_divisor = matches!(
        op,
        BinaryOp::Div | BinaryOp::Mod | BinaryOp::DivFloor | BinaryOp::DivCeil
    );
    if needs_divisor && y == 0 {
        return Err("division by zero".into());
    }
    let result = match op {
        BinaryOp::Add => x.checked_add(y),
        BinaryOp::Sub => x.checked_sub(y),
        BinaryOp::Mul => x.checked_mul(y),
        BinaryOp::Div => x.checked_div(y),
        BinaryOp::Mod => x.checked_rem_euclid(y),
        BinaryOp::DivFloor => divide(i128::from(x), i128::from(y), Rounding::Floor)
            .and_then(|q| i64::try_from(q).ok()),
        BinaryOp::DivCeil => {
            divide(i128::from(x), i128::from(y), Rounding::Ceil).and_then(|q| i64::try_from(q).ok())
        }
        BinaryOp::Pow => {
            let exponent = u32::try_from(y)
                .ok()
                .filter(|e| *e <= 63)
                .ok_or_else(|| format!("exponent must be 0-63, got {y}"))?;
            x.checked_pow(exponent)
        }
        BinaryOp::Min => Some(x.min(y)),
        BinaryOp::Max => Some(x.max(y)),
        other => return Err(format!("{other:?} isn't arithmetic")),
    };
    result.ok_or_else(|| "integer overflow".into())
}

pub(crate) fn as_decimal(value: &Value) -> Decimal {
    match value {
        Value::Decimal(d) => *d,
        Value::Int(i) => Decimal::from_int(*i),
        _ => Decimal::from_int(0),
    }
}

/// Orders two values of the same kind. Ints and decimals compare numerically.
pub(crate) fn compare(a: &Value, b: &Value) -> Result<Ordering, String> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::Int(_) | Value::Decimal(_), Value::Int(_) | Value::Decimal(_)) => {
            Ok(as_decimal(a).cmp(&as_decimal(b)))
        }
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        _ => Err(format!(
            "can't order {} and {}",
            a.type_name(),
            b.type_name()
        )),
    }
}

/// Equality, with ints and decimals compared numerically.
pub(crate) fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(_) | Value::Decimal(_), Value::Int(_) | Value::Decimal(_)) => {
            as_decimal(a) == as_decimal(b)
        }
        _ => a == b,
    }
}

/// Rounds an int or decimal to a decimal at `scale`.
pub(crate) fn round(value: &Value, scale: u8, mode: Rounding) -> Result<Value, String> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Int(_) | Value::Decimal(_) => as_decimal(value)
            .rescale(scale, mode)
            .map(Value::Decimal)
            .ok_or_else(overflow),
        other => Err(format!("can't round {}", other.type_name())),
    }
}

/// Converts an int or decimal to an int.
pub(crate) fn to_int(value: &Value, mode: Rounding) -> Result<Value, String> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Int(i) => Ok(Value::Int(*i)),
        Value::Decimal(d) => d.to_int(mode).map(Value::Int).ok_or_else(overflow),
        other => Err(format!("can't convert {} to an int", other.type_name())),
    }
}

/// Formats a number with `decimals` digits after the point.
pub(crate) fn format_number(
    value: &Value,
    decimals: Option<u8>,
    grouping: bool,
) -> Result<Value, String> {
    let (decimal, own) = match value {
        Value::Null => return Ok(Value::Null),
        Value::Int(i) => (Decimal::from_int(*i), 0),
        Value::Decimal(d) => (*d, d.scale()),
        other => return Err(format!("can't format {} as a number", other.type_name())),
    };
    decimal
        .format(decimals.unwrap_or(own), grouping)
        .map(Value::String)
        .ok_or_else(overflow)
}

/// Characters `start..end` of a string, or items of a list, with JavaScript `slice` semantics.
pub(crate) fn slice_range(len: usize, start: i64, end: Option<i64>) -> (usize, usize) {
    let len_i = i64::try_from(len).unwrap_or(i64::MAX);
    let clamp = |i: i64| {
        let i = if i < 0 {
            (len_i + i).max(0)
        } else {
            i.min(len_i)
        };
        usize::try_from(i).unwrap_or(0)
    };
    let (s, e) = (clamp(start), clamp(end.unwrap_or(len_i)));
    (s, e.max(s))
}

/// Splits into at most `max` parts; the last keeps the rest.
pub(crate) fn split(text: &str, sep: &str, max: u32) -> Result<Vec<Value>, String> {
    if sep.is_empty() {
        return Err("split needs a separator that isn't empty".into());
    }
    Ok(text
        .splitn(max as usize, sep)
        .map(|s| Value::String(s.to_owned()))
        .collect())
}

/// Replaces every occurrence of `find`.
pub(crate) fn replace(text: &str, find: &str, with: &str) -> Result<String, String> {
    if find.is_empty() {
        return Err("replace needs text to find that isn't empty".into());
    }
    let out = text.replace(find, with);
    if out.len() > MAX_STRING_BYTES {
        return Err(format!("strings are limited to {MAX_STRING_BYTES} bytes"));
    }
    Ok(out)
}

/// Pads to `length` characters with repetitions of `fill`.
pub(crate) fn pad(text: &str, length: u32, fill: &str, end: bool) -> String {
    let have = text.chars().count();
    let want = length as usize;
    if have >= want || fill.is_empty() {
        return text.to_owned();
    }
    let padding: String = fill.chars().cycle().take(want - have).collect();
    if end {
        format!("{text}{padding}")
    } else {
        format!("{padding}{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_rounding_division() {
        assert_eq!(int_arithmetic(BinaryOp::DivFloor, -7, 2), Ok(-4));
        assert_eq!(int_arithmetic(BinaryOp::DivCeil, 7, 2), Ok(4));
        assert_eq!(int_arithmetic(BinaryOp::Div, -7, 2), Ok(-3));
        assert_eq!(int_arithmetic(BinaryOp::Mod, -7, 3), Ok(2));
        assert_eq!(int_arithmetic(BinaryOp::Pow, 2, 10), Ok(1024));
        assert!(int_arithmetic(BinaryOp::Pow, 2, 64).is_err());
        assert!(int_arithmetic(BinaryOp::Mod, 1, 0).is_err());
    }

    #[test]
    fn mixed_arithmetic_promotes_to_decimal() {
        let price = Value::Decimal(Decimal::parse("19.99").unwrap());
        let total = arithmetic(BinaryOp::Mul, &price, &Value::Int(3)).unwrap();
        assert_eq!(total.to_json(), serde_json::json!("59.97"));
        assert!(equal(
            &Value::Int(2),
            &Value::Decimal(Decimal::parse("2.00").unwrap())
        ));
        assert_eq!(
            compare(&Value::Int(2), &price).unwrap(),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn slices_like_javascript() {
        assert_eq!(slice_range(5, 1, Some(3)), (1, 3));
        assert_eq!(slice_range(5, -2, None), (3, 5));
        assert_eq!(slice_range(5, 4, Some(2)), (4, 4));
        assert_eq!(slice_range(5, 0, Some(99)), (0, 5));
    }

    #[test]
    fn string_helpers() {
        assert_eq!(pad("7", 3, "0", false), "007");
        assert_eq!(pad("ab", 5, "xy", true), "abxyx");
        assert_eq!(split("a,b,c", ",", 2).unwrap().len(), 2);
        assert_eq!(replace("a-b-c", "-", "+").unwrap(), "a+b+c");
    }
}
