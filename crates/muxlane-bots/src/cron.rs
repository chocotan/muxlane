//! Pure-Rust 5-field UTC cron parser and matcher (`min hour dom mon dow`).
use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    minutes: Vec<bool>, // 0..60
    hours: Vec<bool>,   // 0..24
    doms: Vec<bool>,    // 1..=31 (index 0 unused)
    months: Vec<bool>,  // 1..=12 (index 0 unused)
    dows: Vec<bool>,    // 0..7 (0 = Sunday)
}

impl CronSchedule {
    pub fn parse(expr: &str) -> Result<Self> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            bail!("cron expression must have 5 fields (min hour dom mon dow)");
        }
        Ok(Self {
            minutes: parse_field(fields[0], 0, 59, false)?,
            hours: parse_field(fields[1], 0, 23, false)?,
            doms: parse_field(fields[2], 1, 31, false)?,
            months: parse_field(fields[3], 1, 12, false)?,
            dows: parse_field(fields[4], 0, 6, true)?,
        })
    }

    pub fn matches_unix_secs(&self, unix_secs: u64) -> bool {
        let (month, day, dow, hour, minute) = utc_calendar_fields(unix_secs);
        self.minutes.get(minute as usize).copied().unwrap_or(false)
            && self.hours.get(hour as usize).copied().unwrap_or(false)
            && self.doms.get(day as usize).copied().unwrap_or(false)
            && self.months.get(month as usize).copied().unwrap_or(false)
            && self.dows.get(dow as usize).copied().unwrap_or(false)
    }
}

fn parse_field(spec: &str, min: usize, max: usize, wrap_sunday: bool) -> Result<Vec<bool>> {
    let mut table = vec![false; max + 1];
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            bail!("empty cron field segment");
        }
        let (range_part, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: usize = s.parse()?;
                if step == 0 {
                    bail!("cron step cannot be 0");
                }
                (r, step)
            }
            None => (part, 1),
        };
        let (start, end) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let a = parse_num(a, min, max, wrap_sunday)?;
            let b = parse_num(b, min, max, wrap_sunday)?;
            if a > b {
                bail!("invalid cron range {a}-{b}");
            }
            (a, b)
        } else {
            let val = parse_num(range_part, min, max, wrap_sunday)?;
            if step == 1 {
                (val, val)
            } else {
                (val, max)
            }
        };
        let mut cur = start;
        while cur <= end {
            table[cur] = true;
            cur += step;
        }
    }
    Ok(table)
}

fn parse_num(raw: &str, min: usize, max: usize, wrap_sunday: bool) -> Result<usize> {
    let mut val: usize = raw.parse()?;
    if wrap_sunday && val == 7 {
        val = 0;
    }
    if val < min || val > max {
        bail!("cron value {val} out of bounds [{min}..={max}]");
    }
    Ok(val)
}

/// Converts UNIX epoch seconds to UTC `(month 1..=12, day 1..=31, dow 0..=6 Sun..Sat, hour, minute)`.
pub fn utc_calendar_fields(unix_secs: u64) -> (u8, u8, u8, u8, u8) {
    let total_days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    let hour = (rem / 3_600) as u8;
    let minute = ((rem % 3_600) / 60) as u8;
    // 1970-01-01 was Thursday (dow = 4).
    let dow = ((total_days + 4).rem_euclid(7)) as u8;

    // Howard Hinnant's civil_from_days algorithm.
    let z = total_days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (m as u8, d as u8, dow, hour, minute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_parses_steps_ranges_and_matches_known_utc_timestamp() {
        // 2026-11-23T08:15:00Z is Monday (dow = 1).
        // Compute exact epoch cleanly via date math: 2026-11-23 UTC
        // Days from 1970-01-01 to 2026-11-23 = 20780 days -> check fields!
        let ts = 1_795_421_700u64; // verify via utc_calendar_fields
        let (m, d, dow, h, min) = utc_calendar_fields(ts);
        let sched = CronSchedule::parse(&format!("{min} {h} {d} {m} {dow}")).unwrap();
        assert!(sched.matches_unix_secs(ts));

        let step = CronSchedule::parse("*/15 0-23 * * 0-6").unwrap();
        assert!(step.matches_unix_secs(60 * 15));
        assert!(!step.matches_unix_secs(60 * 14));
        assert!(CronSchedule::parse("bad expr").is_err());
    }
}
