//! Cron expressions for the Schedule feature.
//!
//! Supports the standard 5-field form (`min hour day-of-month month day-of-week`)
//! and the 6-field form with a leading seconds column, plus the common
//! `@macros`. Each field accepts `*`, `*\/n`, `a`, `a-b`, `a-b\/n` and comma
//! lists. Day-of-month and day-of-week follow Vixie cron's OR semantics when both
//! are restricted, which is what users expect from "0 0 1 * mon".

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};

/// How often the scheduler wakes up to look for due jobs.
pub const TICK_SECS: u64 = 20;

/// How often the memory auto-formation queue wakes up (memory spec §6).
///
/// Its own cadence, not the cron tick's: extraction jobs retry on a 30 s
/// exponential backoff, so a tick at the same rate keeps the recorded
/// `nextAttemptAt` honest without spinning on rows whose window is closed.
pub const MEMORY_JOB_TICK_SECS: u64 = 30;

/// How often the memory decay pass runs (memory spec §3 減衰).
///
/// Once a day, by design: the pass's own floor (`DECAY_MIN_DAYS`, half a day)
/// means a more frequent tick would only re-read rows it must refuse to touch,
/// and §3's half-life is measured in days, not in seconds.
pub const MEMORY_DECAY_TICK_SECS: u64 = 86_400;

/// A parsed cron expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronSchedule {
    seconds: Vec<bool>,
    minutes: Vec<bool>,
    hours: Vec<bool>,
    days: Vec<bool>,
    months: Vec<bool>,
    weekdays: Vec<bool>,
    /// `true` when day-of-month was `*`, which changes the DOW combination rule.
    dom_restricted: bool,
    /// `true` when day-of-week was `*`.
    dow_restricted: bool,
    /// Whether the input carried a seconds column.
    with_seconds: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronError(String);

impl std::fmt::Display for CronError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid cron expression: {}", self.0)
    }
}
impl std::error::Error for CronError {}

/// Expand one field into a membership table.
fn parse_field(field: &str, min: u32, max: u32, names: &[&str]) -> Result<Vec<bool>, CronError> {
    let size = (max - min + 1) as usize;
    let mut table = vec![false; size];
    let field = field.trim();
    if field.is_empty() {
        return Err(CronError("empty field".into()));
    }

    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(CronError("empty list element".into()));
        }
        let (range_part, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s
                    .parse()
                    .map_err(|_| CronError(format!("bad step in `{part}`")))?;
                if step == 0 {
                    return Err(CronError(format!("step must not be zero in `{part}`")));
                }
                (r, step)
            }
            None => (part, 1),
        };

        let (start, end) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            (
                parse_value(a, min, max, names)?,
                parse_value(b, min, max, names)?,
            )
        } else {
            let v = parse_value(range_part, min, max, names)?;
            // A bare value with a step means "from here to the end".
            if step > 1 {
                (v, max)
            } else {
                (v, v)
            }
        };

        if start > end {
            return Err(CronError(format!(
                "range {start}-{end} is inverted in `{part}`"
            )));
        }
        let mut v = start;
        while v <= end {
            table[(v - min) as usize] = true;
            // Guard against overflow on a pathological max.
            v = v.saturating_add(step);
        }
    }
    Ok(table)
}

fn parse_value(token: &str, min: u32, max: u32, names: &[&str]) -> Result<u32, CronError> {
    let token = token.trim();
    if let Ok(v) = token.parse::<u32>() {
        if v < min || v > max {
            return Err(CronError(format!("value {v} is outside {min}-{max}")));
        }
        return Ok(v);
    }
    let lower = token.to_ascii_lowercase();
    if let Some(idx) = names.iter().position(|n| *n == lower) {
        return Ok(min + idx as u32);
    }
    // Three-letter abbreviations, e.g. "mon".
    if names.len() == 7 {
        for (idx, name) in names.iter().enumerate() {
            if name.starts_with(&lower) && lower.len() == 3 {
                return Ok(min + idx as u32);
            }
        }
    }
    Err(CronError(format!("unknown value `{token}`")))
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
/// Sunday is 0 in cron and 1 in `chrono`, so the table is indexed from 0.
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

impl CronSchedule {
    /// Parse a cron expression or an `@macro`.
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let expr = expr.trim();
        let expanded = match expr {
            "@yearly" | "@annually" => "0 0 1 1 *",
            "@monthly" => "0 0 1 * *",
            "@weekly" => "0 0 * * 0",
            "@daily" | "@midnight" => "0 0 * * *",
            "@hourly" => "0 * * * *",
            other => other,
        };
        let fields: Vec<&str> = expanded.split_whitespace().collect();
        let with_seconds = match fields.len() {
            5 => false,
            6 => true,
            n => return Err(CronError(format!("expected 5 or 6 fields, found {n}"))),
        };

        let offset = usize::from(with_seconds);
        let seconds = if with_seconds {
            parse_field(fields[0], 0, 59, &[])?
        } else {
            vec![true; 60]
        };
        let minutes = parse_field(fields[offset], 0, 59, &[])?;
        let hours = parse_field(fields[offset + 1], 0, 23, &[])?;
        let days = parse_field(fields[offset + 2], 1, 31, &[])?;
        let months = parse_field(fields[offset + 3], 1, 12, &MONTHS)?;
        let weekdays = parse_field(fields[offset + 4], 0, 6, &WEEKDAYS)?;

        Ok(Self {
            seconds,
            minutes,
            hours,
            days,
            months,
            weekdays,
            dom_restricted: fields[offset + 2] != "*",
            dow_restricted: fields[offset + 4] != "*",
            with_seconds,
        })
    }

    /// Does this schedule fire at exactly `at`?
    pub fn matches(&self, at: DateTime<Utc>) -> bool {
        if !self.seconds[at.second() as usize]
            || !self.minutes[at.minute() as usize]
            || !self.hours[at.hour() as usize]
            || !self.months[(at.month() - 1) as usize]
        {
            return false;
        }
        let dom = self.days[(at.day() - 1) as usize];
        let dow = self.weekdays[at.weekday().num_days_from_sunday() as usize];
        // Vixie cron: when both day fields are restricted, either may match.
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom || dow,
            (true, false) => dom,
            (false, true) => dow,
            (false, false) => true,
        }
    }

    /// The first firing time strictly after `from`.
    ///
    /// Scans forward at the schedule's resolution rather than doing clever
    /// field-jumping, which keeps the semantics obviously correct. The search is
    /// bounded to five years, past which a schedule like `0 0 30 2 *` (February
    /// 30th) simply has no next occurrence.
    pub fn next_after(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let step = chrono::Duration::seconds(if self.with_seconds { 1 } else { 60 });
        // Advance to the next whole unit so a minute schedule does not fire
        // again on the same minute it was created.
        let mut candidate = if self.with_seconds {
            from + chrono::Duration::seconds(1)
        } else {
            let floored = from
                .with_second(0)
                .and_then(|d| d.with_nanosecond(0))
                .unwrap_or(from);
            floored + chrono::Duration::minutes(1)
        };
        let limit = from + chrono::Duration::days(365 * 5);
        while candidate <= limit {
            if self.matches(candidate) {
                return Some(candidate);
            }
            candidate += step;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, s).unwrap()
    }

    #[test]
    fn parses_five_and_six_fields() {
        assert!(CronSchedule::parse("0 9 * * *").is_ok());
        assert!(CronSchedule::parse("30 0 9 * * *").is_ok());
        assert!(CronSchedule::parse("0 9 * *").is_err());
    }

    #[test]
    fn wildcards_every_second_and_hour() {
        let sched = CronSchedule::parse("@hourly").expect("parse");
        assert!(sched.matches(at(2026, 1, 1, 5, 0, 0)));
        assert!(!sched.matches(at(2026, 1, 1, 5, 1, 0)));
    }

    #[test]
    fn daily_at_midnight() {
        let sched = CronSchedule::parse("@daily").expect("parse");
        assert!(sched.matches(at(2026, 3, 4, 0, 0, 0)));
        assert!(!sched.matches(at(2026, 3, 4, 0, 1, 0)));
    }

    #[test]
    fn every_fifteen_minutes() {
        let sched = CronSchedule::parse("*/15 * * * *").expect("parse");
        assert!(sched.matches(at(2026, 1, 1, 10, 0, 0)));
        assert!(sched.matches(at(2026, 1, 1, 10, 15, 0)));
        assert!(sched.matches(at(2026, 1, 1, 10, 45, 0)));
        assert!(!sched.matches(at(2026, 1, 1, 10, 16, 0)));
    }

    #[test]
    fn explicit_ranges_and_lists() {
        let sched = CronSchedule::parse("0 9-17 * * mon-fri").expect("parse");
        assert!(sched.matches(at(2026, 1, 5, 9, 0, 0))); // Monday
        assert!(sched.matches(at(2026, 1, 9, 17, 0, 0))); // Friday
        assert!(!sched.matches(at(2026, 1, 10, 9, 0, 0))); // Saturday
        assert!(!sched.matches(at(2026, 1, 5, 18, 0, 0))); // out of hours

        let list = CronSchedule::parse("0 0,12 * * *").expect("parse");
        assert!(list.matches(at(2026, 1, 1, 0, 0, 0)));
        assert!(list.matches(at(2026, 1, 1, 12, 0, 0)));
        assert!(!list.matches(at(2026, 1, 1, 6, 0, 0)));
    }

    #[test]
    fn month_and_weekday_names() {
        let sched = CronSchedule::parse("0 0 1 jan mon").expect("parse");
        assert!(sched.months[0], "january must be selected");
        assert!(sched.weekdays[1], "monday must be selected");
    }

    #[test]
    fn dom_and_dow_are_or_ed_when_both_restricted() {
        // The 1st OR any Monday.
        let sched = CronSchedule::parse("0 0 1 * mon").expect("parse");
        assert!(sched.matches(at(2026, 1, 1, 0, 0, 0))); // 1st (a Thursday)
        assert!(sched.matches(at(2026, 1, 5, 0, 0, 0))); // Monday
        assert!(!sched.matches(at(2026, 1, 6, 0, 0, 0))); // Tuesday
    }

    #[test]
    fn rejects_bad_input() {
        assert!(CronSchedule::parse("60 * * * *").is_err(), "minute 60");
        assert!(CronSchedule::parse("* 24 * * *").is_err(), "hour 24");
        assert!(CronSchedule::parse("* * 32 * *").is_err(), "day 32");
        assert!(CronSchedule::parse("* * * 13 *").is_err(), "month 13");
        assert!(CronSchedule::parse("*/0 * * * *").is_err(), "zero step");
        assert!(
            CronSchedule::parse("10-5 * * * *").is_err(),
            "inverted range"
        );
        assert!(CronSchedule::parse("nope * * * *").is_err(), "garbage");
    }

    #[test]
    fn next_after_is_strictly_later() {
        let sched = CronSchedule::parse("0 * * * *").expect("parse");
        let base = at(2026, 1, 1, 10, 0, 0);
        // Firing exactly now must advance, not return the same instant.
        assert_eq!(sched.next_after(base), Some(at(2026, 1, 1, 11, 0, 0)));
        assert_eq!(
            sched.next_after(base + chrono::Duration::seconds(30)),
            Some(at(2026, 1, 1, 11, 0, 0))
        );
    }

    #[test]
    fn next_after_finds_the_next_real_occurrence() {
        let sched = CronSchedule::parse("30 2 * * *").expect("parse");
        assert_eq!(
            sched.next_after(at(2026, 1, 1, 3, 0, 0)),
            Some(at(2026, 1, 2, 2, 30, 0))
        );
    }

    #[test]
    fn next_after_handles_month_and_dst_boundaries() {
        // 00:30 on the 1st of each month, UTC (no DST in UTC).
        let sched = CronSchedule::parse("30 0 1 * *").expect("parse");
        assert_eq!(
            sched.next_after(at(2026, 1, 15, 0, 0, 0)),
            Some(at(2026, 2, 1, 0, 30, 0))
        );
        // Leap year.
        let leap = CronSchedule::parse("0 0 29 2 *").expect("parse");
        assert_eq!(
            leap.next_after(at(2024, 3, 1, 0, 0, 0)),
            Some(at(2028, 2, 29, 0, 0, 0))
        );
    }

    #[test]
    fn impossible_schedules_have_no_next() {
        // February 30th never happens.
        let sched = CronSchedule::parse("0 0 30 2 *").expect("parse");
        assert_eq!(sched.next_after(at(2026, 1, 1, 0, 0, 0)), None);
    }

    #[test]
    fn six_field_schedule_resolves_seconds() {
        let sched = CronSchedule::parse("15 30 4 * * *").expect("parse");
        assert_eq!(
            sched.next_after(at(2026, 5, 5, 4, 30, 0)),
            Some(at(2026, 5, 5, 4, 30, 15))
        );
    }
}
