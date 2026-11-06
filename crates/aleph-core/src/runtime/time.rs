//! Calendar arithmetic in IANA time zones, with a bundled time zone database so results are the
//! same on every machine.

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};

use crate::manifest::{TimePart, TimeUnit};

/// Looks up an IANA time zone (`"UTC"` included).
pub(crate) fn zone(name: &str) -> Option<TimeZone> {
    if name.len() > 64 {
        return None;
    }
    jiff::tz::db().get(name).ok()
}

fn zoned(ms: i64, tz: &TimeZone) -> Result<Zoned, String> {
    Timestamp::from_millisecond(ms)
        .map(|t| t.to_zoned(tz.clone()))
        .map_err(|e| format!("timestamp {ms} is out of range: {e}"))
}

fn millis(z: &Zoned) -> i64 {
    z.timestamp().as_millisecond()
}

/// The start of the `unit` containing `ms`, in `tz`.
pub(crate) fn start_of(ms: i64, unit: TimeUnit, tz: &TimeZone) -> Result<i64, String> {
    let z = zoned(ms, tz)?;
    let start = match unit {
        TimeUnit::Second => z.round(trunc(jiff::Unit::Second)),
        TimeUnit::Minute => z.round(trunc(jiff::Unit::Minute)),
        TimeUnit::Hour => z.round(trunc(jiff::Unit::Hour)),
        TimeUnit::Day => z.start_of_day(),
        TimeUnit::Week => {
            let back = i64::from(z.weekday().to_monday_zero_offset());
            z.date()
                .checked_sub(back.days())
                .map_err(|e| e.to_string())?
                .to_zoned(tz.clone())
        }
        TimeUnit::Month => z.first_of_month().and_then(|d| d.start_of_day()),
        TimeUnit::Year => z.first_of_year().and_then(|d| d.start_of_day()),
    }
    .map_err(|e| e.to_string())?;
    Ok(millis(&start))
}

fn trunc(unit: jiff::Unit) -> jiff::ZonedRound {
    jiff::ZonedRound::new()
        .smallest(unit)
        .mode(jiff::RoundMode::Trunc)
}

/// `ms` moved by `amount` units. Days and larger follow the calendar in `tz`.
pub(crate) fn add(ms: i64, amount: i64, unit: TimeUnit, tz: &TimeZone) -> Result<i64, String> {
    let z = zoned(ms, tz)?;
    let span = match unit {
        TimeUnit::Second => amount.seconds(),
        TimeUnit::Minute => amount.minutes(),
        TimeUnit::Hour => amount.hours(),
        TimeUnit::Day => amount.days(),
        TimeUnit::Week => amount.weeks(),
        TimeUnit::Month => amount.months(),
        TimeUnit::Year => amount.years(),
    };
    let moved = z.checked_add(span).map_err(|e| e.to_string())?;
    Ok(millis(&moved))
}

/// One part of `ms` in `tz`.
pub(crate) fn part(ms: i64, part: TimePart, tz: &TimeZone) -> Result<i64, String> {
    let z = zoned(ms, tz)?;
    Ok(match part {
        TimePart::Year => i64::from(z.year()),
        TimePart::Month => i64::from(z.month()),
        TimePart::Day => i64::from(z.day()),
        TimePart::Hour => i64::from(z.hour()),
        TimePart::Minute => i64::from(z.minute()),
        TimePart::Second => i64::from(z.second()),
        TimePart::Weekday => i64::from(z.weekday().to_monday_one_offset()),
        TimePart::DayOfYear => i64::from(z.day_of_year()),
    })
}

/// Formats `ms` in `tz`: ISO 8601 with offset by default, or a `strftime` pattern.
pub(crate) fn format(ms: i64, pattern: Option<&str>, tz: &TimeZone) -> Result<String, String> {
    let z = zoned(ms, tz)?;
    match pattern {
        None => Ok(z.strftime("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()),
        Some(pattern) => jiff::fmt::strtime::format(pattern, &z).map_err(|e| e.to_string()),
    }
}

/// Checks a `strftime` pattern at validation time.
pub(crate) fn valid_pattern(pattern: &str) -> Result<(), String> {
    let z = zoned(0, &TimeZone::UTC)?;
    jiff::fmt::strtime::format(pattern, &z)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Exposes `zoned` for the cron schedule module.
pub(crate) fn at(ms: i64, tz: &TimeZone) -> Result<Zoned, String> {
    zoned(ms, tz)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-03-30T12:00:00Z, the day before Europe's DST change.
    const SAT_NOON: i64 = 1_711_800_000_000;

    #[test]
    fn days_follow_the_calendar_across_dst() {
        let berlin = zone("Europe/Berlin").unwrap();
        let next = add(SAT_NOON, 1, TimeUnit::Day, &berlin).unwrap();
        // Same wall-clock time, but only 23 hours later.
        assert_eq!(next - SAT_NOON, 23 * 3_600_000);
        assert_eq!(part(next, TimePart::Hour, &berlin).unwrap(), 13);
        assert_eq!(part(SAT_NOON, TimePart::Weekday, &berlin).unwrap(), 6);
    }

    #[test]
    fn starts_of_units() {
        let utc = TimeZone::UTC;
        assert_eq!(
            format(start_of(SAT_NOON, TimeUnit::Day, &utc).unwrap(), None, &utc).unwrap(),
            "2024-03-30T00:00:00.000+00:00"
        );
        assert_eq!(
            format(
                start_of(SAT_NOON, TimeUnit::Week, &utc).unwrap(),
                Some("%Y-%m-%d %a"),
                &utc
            )
            .unwrap(),
            "2024-03-25 Mon"
        );
        let tokyo = zone("Asia/Tokyo").unwrap();
        assert_eq!(
            format(
                start_of(SAT_NOON, TimeUnit::Month, &tokyo).unwrap(),
                None,
                &tokyo
            )
            .unwrap(),
            "2024-03-01T00:00:00.000+09:00"
        );
        assert!(zone("Mars/Olympus").is_none());
        assert!(valid_pattern("%Y-%m-%d").is_ok());
    }
}
