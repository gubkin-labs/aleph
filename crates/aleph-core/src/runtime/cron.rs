//! Calendar schedules: five-field cron expressions (`minute hour day-of-month month weekday`)
//! evaluated in an IANA time zone, so `0 9 * * 1` means 09:00 local time every Monday, across
//! daylight-saving changes.

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;

/// A parsed cron expression: the allowed values of each field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cron {
    minutes: Vec<i8>,
    hours: Vec<i8>,
    days: Vec<i8>,
    months: Vec<i8>,
    weekdays: Vec<i8>,
    /// Whether day-of-month and weekday were both restricted (then either may match).
    either_day: bool,
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

impl Cron {
    /// Parses `minute hour day month weekday`. Fields take `*`, numbers, names (`mon`, `jan`),
    /// ranges `a-b`, steps `*/n` and `a-b/n`, and lists `a,b`. Weekday 0 and 7 are Sunday.
    pub(crate) fn parse(expr: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err(format!(
                "{expr:?} needs 5 fields: minute hour day-of-month month weekday"
            ));
        };
        let mut weekdays = field(weekday, 0, 7, &WEEKDAYS)?;
        for w in &mut weekdays {
            if *w == 7 {
                *w = 0;
            }
        }
        weekdays.sort_unstable();
        weekdays.dedup();
        Ok(Self {
            minutes: field(minute, 0, 59, &[])?,
            hours: field(hour, 0, 23, &[])?,
            days: field(day, 1, 31, &[])?,
            months: field(month, 1, 12, &MONTHS)?,
            weekdays,
            either_day: *day != "*" && *weekday != "*",
        })
    }

    fn day_matches(&self, date: Date) -> bool {
        if !self.months.contains(&date.month()) {
            return false;
        }
        let dom = self.days.contains(&date.day());
        let dow = self
            .weekdays
            .contains(&date.weekday().to_sunday_zero_offset());
        if self.either_day {
            dom || dow
        } else {
            dom && dow
        }
    }

    /// The first scheduled time strictly after `after_ms`, in ms, looking at most ~5 years ahead.
    pub(crate) fn next_after(&self, after_ms: i64, tz: &TimeZone) -> Option<i64> {
        let start = super::time::at(after_ms, tz).ok()?;
        let mut date = start.date();
        for _ in 0..(366 * 5) {
            if self.day_matches(date) {
                for &hour in &self.hours {
                    for &minute in &self.minutes {
                        let local = DateTime::new(
                            date.year(),
                            date.month(),
                            date.day(),
                            hour,
                            minute,
                            0,
                            0,
                        )
                        .ok()?;
                        // In a DST gap, the time moves forward past the gap.
                        let Ok(zoned) = local.to_zoned(tz.clone()) else {
                            continue;
                        };
                        let ms = zoned.timestamp().as_millisecond();
                        if ms > after_ms {
                            return Some(ms);
                        }
                    }
                }
            }
            date = date.tomorrow().ok()?;
        }
        None
    }
}

fn field(text: &str, min: i8, max: i8, names: &[&str]) -> Result<Vec<i8>, String> {
    let value = |s: &str| -> Result<i8, String> {
        let lower = s.to_ascii_lowercase();
        if let Some(i) = names.iter().position(|n| *n == lower) {
            let offset = usize::from(names.len() == 12);
            return i8::try_from(i + offset).map_err(|e| e.to_string());
        }
        let n: i8 = s
            .parse()
            .map_err(|_| format!("{s:?} isn't a valid cron value"))?;
        if n < min || n > max {
            return Err(format!("{n} is outside {min}-{max}"));
        }
        Ok(n)
    };
    let mut out = Vec::new();
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step: i8 = step
                    .parse()
                    .ok()
                    .filter(|s| *s > 0)
                    .ok_or_else(|| format!("{step:?} isn't a valid step"))?;
                (range, step)
            }
            None => (part, 1),
        };
        let (lo, hi) = match range {
            "*" => (min, max),
            r => match r.split_once('-') {
                Some((a, b)) => (value(a)?, value(b)?),
                None if step > 1 => (value(r)?, max),
                None => {
                    let v = value(r)?;
                    (v, v)
                }
            },
        };
        if lo > hi {
            return Err(format!("{range:?} is an empty range"));
        }
        let mut v = lo;
        while v <= hi {
            out.push(v);
            v = v.saturating_add(step);
            if v == i8::MAX {
                break;
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso(ms: i64, tz: &TimeZone) -> String {
        super::super::time::format(ms, Some("%Y-%m-%d %H:%M %a"), tz).unwrap()
    }

    #[test]
    fn mondays_at_nine_in_berlin_across_dst() {
        let berlin = super::super::time::zone("Europe/Berlin").unwrap();
        let cron = Cron::parse("0 9 * * mon").unwrap();
        // Saturday 2024-03-30, before the DST change on Sunday.
        let first = cron.next_after(1_711_800_000_000, &berlin).unwrap();
        assert_eq!(iso(first, &berlin), "2024-04-01 09:00 Mon");
        let second = cron.next_after(first, &berlin).unwrap();
        assert_eq!(iso(second, &berlin), "2024-04-08 09:00 Mon");
    }

    #[test]
    fn parses_steps_ranges_and_lists() {
        let cron = Cron::parse("*/15 9-17 * * 1-5").unwrap();
        assert_eq!(cron.minutes, [0, 15, 30, 45]);
        assert_eq!(cron.hours.len(), 9);
        assert_eq!(cron.weekdays, [1, 2, 3, 4, 5]);
        assert!(Cron::parse("0 9 * *").is_err());
        assert!(Cron::parse("61 * * * *").is_err());
        assert_eq!(Cron::parse("0 0 1 jan,jul *").unwrap().months, [1, 7]);
    }
}
