//! Exact decimals: an integer number of units at a scale (`12.50` is 1250 units at scale 2).
//!
//! Magnitudes stay below 10^20 so any value fits in an `i128` at the largest scale (18), which
//! keeps comparisons and key encoding exact.

use std::cmp::Ordering;
use std::fmt;

use crate::manifest::Rounding;

/// The largest scale.
pub const MAX_SCALE: u8 = 18;

/// The largest magnitude at scale 18: values are below 10^20.
const LIMIT_AT_MAX: i128 = 100_000_000_000_000_000_000_000_000_000_000_000_000; // 10^38

fn pow10(n: u8) -> i128 {
    10_i128.pow(u32::from(n))
}

/// An exact decimal number.
#[derive(Debug, Clone, Copy)]
pub struct Decimal {
    units: i128,
    scale: u8,
}

impl Decimal {
    /// `units` at `scale`, if the value is in range.
    pub fn new(units: i128, scale: u8) -> Option<Self> {
        if scale > MAX_SCALE {
            return None;
        }
        let at_max = units.checked_mul(pow10(MAX_SCALE - scale))?;
        (at_max.abs() < LIMIT_AT_MAX).then_some(Self { units, scale })
    }

    /// An integer as a decimal at scale 0.
    pub fn from_int(i: i64) -> Self {
        Self {
            units: i128::from(i),
            scale: 0,
        }
    }

    /// The units at this decimal's scale.
    pub fn units(self) -> i128 {
        self.units
    }

    /// Digits after the point.
    pub fn scale(self) -> u8 {
        self.scale
    }

    /// The value at scale 18, which orders and compares exactly.
    pub fn normalized(self) -> i128 {
        // In range by construction.
        self.units * pow10(MAX_SCALE - self.scale)
    }

    /// Parses `"-12.50"`, `"3"`, or `"1e3"`-free decimal text.
    pub fn parse(text: &str) -> Option<Self> {
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        if whole.is_empty() && fraction.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
            || fraction.len() > usize::from(MAX_SCALE)
            || whole.len() > 21
        {
            return None;
        }
        let scale = u8::try_from(fraction.len()).ok()?;
        let mut units: i128 = 0;
        for b in whole.bytes().chain(fraction.bytes()) {
            units = units.checked_mul(10)?.checked_add(i128::from(b - b'0'))?;
        }
        Self::new(if negative { -units } else { units }, scale)
    }

    /// The value at `scale`, rounded with `mode` if digits are dropped.
    pub fn rescale(self, scale: u8, mode: Rounding) -> Option<Self> {
        if scale >= self.scale {
            return Self::new(self.units.checked_mul(pow10(scale - self.scale))?, scale);
        }
        let divisor = pow10(self.scale - scale);
        Self::new(divide(self.units, divisor, mode)?, scale)
    }

    /// The value at `scale` if that loses no digits.
    pub fn exactly_at(self, scale: u8) -> Option<Self> {
        let scaled = self.rescale(scale, Rounding::Down)?;
        (scaled == self).then_some(scaled)
    }

    /// The value as an int, rounded with `mode`.
    pub fn to_int(self, mode: Rounding) -> Option<i64> {
        i64::try_from(self.rescale(0, mode)?.units).ok()
    }

    /// `self + other`, at the larger scale.
    pub fn plus(self, other: Self) -> Option<Self> {
        let scale = self.scale.max(other.scale);
        let (a, b) = (
            self.rescale(scale, Rounding::Down)?,
            other.rescale(scale, Rounding::Down)?,
        );
        Self::new(a.units.checked_add(b.units)?, scale)
    }

    /// `self - other`, at the larger scale.
    pub fn minus(self, other: Self) -> Option<Self> {
        other.negate().and_then(|n| self.plus(n))
    }

    /// `-self`.
    pub fn negate(self) -> Option<Self> {
        Self::new(self.units.checked_neg()?, self.scale)
    }

    /// `|self|`.
    pub fn abs(self) -> Option<Self> {
        Self::new(self.units.checked_abs()?, self.scale)
    }

    /// `self * other`, at the sum of the scales (at most 18, rounding half-even beyond it).
    pub fn times(self, other: Self) -> Option<Self> {
        let exact = u32::from(self.scale) + u32::from(other.scale);
        let product = self.units.checked_mul(other.units)?;
        let scale = u8::try_from(exact.min(u32::from(MAX_SCALE))).ok()?;
        let dropped = u8::try_from(exact).ok()? - scale;
        Self::new(divide(product, pow10(dropped), Rounding::HalfEven)?, scale)
    }

    /// `self / other` at `scale`, rounded with `mode`. `None` on division by zero.
    pub fn div(self, other: Self, scale: u8, mode: Rounding) -> Option<Self> {
        if other.units == 0 || scale > MAX_SCALE {
            return None;
        }
        // units / 10^s1 ÷ (units2 / 10^s2) at 10^scale = units * 10^(scale + s2 - s1) / units2.
        let shift = i32::from(scale) + i32::from(other.scale) - i32::from(self.scale);
        let (numerator, denominator) = if shift >= 0 {
            (
                self.units
                    .checked_mul(10_i128.checked_pow(shift.cast_unsigned())?)?,
                other.units,
            )
        } else {
            (
                self.units,
                other
                    .units
                    .checked_mul(10_i128.checked_pow(shift.unsigned_abs())?)?,
            )
        };
        Self::new(divide(numerator, denominator, mode)?, scale)
    }

    /// Formats with exactly `decimals` digits after the point, optionally grouping thousands.
    pub fn format(self, decimals: u8, grouping: bool) -> Option<String> {
        let value = self.rescale(decimals, Rounding::HalfEven)?;
        let digits = value.units.unsigned_abs().to_string();
        let split = digits.len().saturating_sub(usize::from(decimals));
        let (whole, fraction) = if split == 0 {
            (
                "0".to_owned(),
                format!("{digits:0>width$}", width = usize::from(decimals)),
            )
        } else {
            (digits[..split].to_owned(), digits[split..].to_owned())
        };
        let whole = if grouping { group(&whole) } else { whole };
        let sign = if value.units < 0 { "-" } else { "" };
        Some(if decimals == 0 {
            format!("{sign}{whole}")
        } else {
            format!("{sign}{whole}.{fraction}")
        })
    }
}

/// Inserts a comma every three digits from the right.
fn group(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `n / d` rounded with `mode`.
pub(crate) fn divide(n: i128, d: i128, mode: Rounding) -> Option<i128> {
    if d == 0 {
        return None;
    }
    let q = n.checked_div(d)?;
    let r = n.checked_rem(d)?;
    if r == 0 {
        return Some(q);
    }
    // The exact quotient is q + r/d, strictly between q and q ± 1.
    let positive = (n < 0) == (d < 0);
    let away = if positive { q + 1 } else { q - 1 };
    let twice = r.unsigned_abs().checked_mul(2)?;
    let half = twice.cmp(&d.unsigned_abs());
    Some(match mode {
        Rounding::Down => q,
        Rounding::Up => away,
        Rounding::Floor => {
            if positive {
                q
            } else {
                away
            }
        }
        Rounding::Ceil => {
            if positive {
                away
            } else {
                q
            }
        }
        Rounding::HalfUp => {
            if half == Ordering::Less {
                q
            } else {
                away
            }
        }
        Rounding::HalfEven => match half {
            Ordering::Less => q,
            Ordering::Greater => away,
            Ordering::Equal => {
                if q % 2 == 0 {
                    q
                } else {
                    away
                }
            }
        },
    })
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.normalized() == other.normalized()
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.normalized().cmp(&other.normalized())
    }
}

impl fmt::Display for Decimal {
    /// Formats at the decimal's own scale: `12.50`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.format(self.scale, false) {
            Some(text) => f.write_str(&text),
            None => write!(f, "{}e-{}", self.units, self.scale),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text).unwrap()
    }

    #[test]
    fn parses_and_formats() {
        assert_eq!(d("12.50").to_string(), "12.50");
        assert_eq!(d("-0.05").to_string(), "-0.05");
        assert_eq!(d("7").to_string(), "7");
        assert!(Decimal::parse("1.2.3").is_none());
        assert!(Decimal::parse("").is_none());
        assert!(Decimal::parse("1e3").is_none());
        assert_eq!(d("1234567.891").format(2, true).unwrap(), "1,234,567.89");
        assert_eq!(d("0.5").format(0, false).unwrap(), "0");
        assert_eq!(d("1.5").format(0, false).unwrap(), "2");
    }

    #[test]
    fn arithmetic_is_exact() {
        assert_eq!(d("0.1").plus(d("0.2")).unwrap(), d("0.3"));
        assert_eq!(d("19.99").times(d("3")).unwrap().to_string(), "59.97");
        assert_eq!(
            d("10")
                .div(d("3"), 4, Rounding::HalfEven)
                .unwrap()
                .to_string(),
            "3.3333"
        );
        assert_eq!(
            d("2").div(d("3"), 2, Rounding::Down).unwrap().to_string(),
            "0.66"
        );
        assert_eq!(d("1.00"), d("1"));
        assert!(d("1.5") > d("1.49"));
    }

    #[test]
    fn rounds_every_way() {
        let cases = [
            (Rounding::HalfEven, ["2", "2", "-2", "3"]),
            (Rounding::HalfUp, ["3", "2", "-3", "3"]),
            (Rounding::Down, ["2", "2", "-2", "2"]),
            (Rounding::Up, ["3", "3", "-3", "3"]),
            (Rounding::Floor, ["2", "2", "-3", "2"]),
            (Rounding::Ceil, ["3", "3", "-2", "3"]),
        ];
        for (mode, expected) in cases {
            for (input, want) in ["2.5", "2.1", "-2.5", "2.7"].iter().zip(expected) {
                assert_eq!(
                    d(input).rescale(0, mode).unwrap(),
                    d(want),
                    "{input} {mode:?}"
                );
            }
        }
    }

    #[test]
    fn stays_in_range() {
        assert!(Decimal::parse("99999999999999999999").is_some());
        assert!(Decimal::parse("100000000000000000000").is_none());
        assert!(d("99999999999999999999").times(d("10")).is_none());
    }
}
