//! JSON list files in the vanilla layout (`StoredUserList`) and the
//! `BanListEntry.DATE_FORMAT` date strings. Dates are written in UTC
//! (`+0000`); the server has no time zone database.

use serde::Serialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn split(epoch: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = epoch.div_euclid(86400);
    let rem = epoch.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    (y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// `yyyy-MM-dd HH:mm:ss Z` (UTC).
pub fn format_date(epoch: i64) -> String {
    let (y, m, d, h, mi, s) = split(epoch);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} +0000")
}

/// `yyyy-MM-dd 'at' HH:mm:ss z` (the expiration line of the disconnect text).
pub fn format_expiry(epoch: i64) -> String {
    let (y, m, d, h, mi, s) = split(epoch);
    format!("{y:04}-{m:02}-{d:02} at {h:02}:{mi:02}:{s:02} UTC")
}

/// Parses `yyyy-MM-dd HH:mm:ss Z` into epoch seconds.
pub fn parse_date(s: &str) -> Option<i64> {
    let mut it = s.split_whitespace();
    let date = it.next()?;
    let time = it.next()?;
    let zone = it.next()?;
    let mut dp = date.split('-').map(|v| v.parse::<i64>().ok());
    let (y, m, d) = (dp.next()??, dp.next()??, dp.next()??);
    let mut tp = time.split(':').map(|v| v.parse::<i64>().ok());
    let (h, mi, sec) = (tp.next()??, tp.next()??, tp.next()??);
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let zv: i64 = zone[1..].parse().ok()?;
    let off = sign * ((zv / 100) * 3600 + (zv % 100) * 60);
    Some(days_from_civil(y, m, d) * 86400 + h * 3600 + mi * 60 + sec - off)
}

/// Reads a vanilla list file. Missing file: empty. Unreadable JSON is moved
/// to `<file>.backup` (vanilla `StoredUserList.load`).
pub fn read_list(file: &str) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(v)) => v,
        Ok(_) => Vec::new(),
        Err(e) => {
            tracing::warn!("Unable to read file {file}, backing it up: {e}");
            let _ = std::fs::rename(file, format!("{file}.backup"));
            Vec::new()
        }
    }
}

/// Writes a list as pretty JSON (Gson layout: 2 spaces, `"k": v`).
pub fn write_list<T: Serialize>(file: &str, items: &[T]) {
    match serde_json::to_string_pretty(items) {
        Ok(text) => {
            if let Err(e) = std::fs::write(file, text) {
                tracing::warn!("Could not save {file}: {e}");
            }
        }
        Err(e) => tracing::warn!("Could not serialise {file}: {e}"),
    }
}

/// `JsonObject.get(key).getAsString()` when present.
pub fn str_field(o: &Value, key: &str) -> Option<String> {
    o.get(key).and_then(|v| v.as_str()).map(str::to_string)
}
