//! `Date` (ECMA-262 5.1 section 15.9), with `getYear`, `setYear` and `toGMTString` from Annex B.
//!
//! Time values are milliseconds since 1970-01-01 UTC (section 15.9.1). `chrono` turns days into
//! calendar dates and back; past the years it represents, dates are shifted by whole 400-year
//! cycles. The current time comes from the host's clock and local time from the host's
//! time zone ([`DateHost`]): by default the system clock (none in a wasm build, where `Date.now()`
//! is NaN until the host sets one) and UTC. `Date.parse` reads the format of section 15.9.1.15
//! and the formats `toString` and `toUTCString` write.

use std::rc::Rc;

use chrono::{Datelike, NaiveDate};

use crate::builtins::{arg, constructor, data, method, native};
use crate::heap::{Class, NativeFn, NativeKind, Object, key};
use crate::interp::{ErrorType, Hint, Interp, JsResult};
use crate::string_builtins::{charge_units, reserve_units, to_integer};
use crate::value::{Str, Value};

/// The clock and time zone a realm's `Date` reads.
#[derive(Clone)]
pub(crate) struct DateHost {
    /// The current time value.
    pub(crate) clock: Rc<dyn Fn() -> f64>,
    /// The offset of local time from UTC, in milliseconds, at a UTC time value.
    pub(crate) offset: Rc<dyn Fn(f64) -> f64>,
}

impl Default for DateHost {
    fn default() -> Self {
        Self { clock: Rc::new(system_clock), offset: Rc::new(|_| 0.0) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn system_clock() -> f64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as f64,
        Err(e) => -(e.duration().as_millis() as f64),
    }
}

#[cfg(target_arch = "wasm32")]
fn system_clock() -> f64 {
    f64::NAN
}

const MS_PER_DAY: f64 = 86_400_000.0;
const MS_PER_HOUR: f64 = 3_600_000.0;
const MS_PER_MINUTE: f64 = 60_000.0;
const MS_PER_SECOND: f64 = 1000.0;

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// ---- section 15.9.1 ----

fn day(t: f64) -> f64 {
    (t / MS_PER_DAY).floor()
}

fn time_within_day(t: f64) -> f64 {
    t.rem_euclid(MS_PER_DAY)
}

/// `chrono`'s day number (0001-01-01 is day 1) of 1970-01-01.
const EPOCH_CE_DAYS: i64 = 719_163;
/// The days in 400 Gregorian years, after which the calendar repeats.
const CYCLE_DAYS: i64 = 146_097;
/// The days from 1970-01-01 to 2000-01-01, where the 400-year cycle `chrono` is asked about starts.
const DAYS_TO_2000: i64 = 10_957;

/// The year, the month (0 to 11) and the day of the month of a day (days since 1970-01-01).
/// `chrono` computes them for the matching day of the 400-year cycle starting in 2000, and the
/// whole cycles are added back to the year, so days past `chrono`'s range have dates too.
fn civil(day: f64) -> Option<(f64, f64, f64)> {
    if !day.is_finite() || day.abs() > 1e9 {
        return None;
    }
    let d = day as i64;
    let cycles = (d - DAYS_TO_2000).div_euclid(CYCLE_DAYS);
    let date = NaiveDate::from_num_days_from_ce_opt(i32::try_from(d - cycles * CYCLE_DAYS + EPOCH_CE_DAYS).ok()?)?;
    Some((f64::from(date.year()) + 400.0 * cycles as f64, f64::from(date.month0()), f64::from(date.day())))
}

/// The year of the 400-year cycle starting in 2000 that has `year`'s calendar, and the number of
/// whole cycles between the two.
fn cycle_year(year: f64) -> Option<(i32, i64)> {
    if !year.is_finite() || year.abs() > 1e7 {
        return None;
    }
    let y = year as i64;
    let cycles = (y - 2000).div_euclid(400);
    Some((i32::try_from(y - 400 * cycles).ok()?, cycles))
}

/// The day (days since 1970-01-01) on which month `month0` (0 to 11) of `year` starts.
fn first_day_of_month(year: f64, month0: u32) -> Option<f64> {
    let (y, cycles) = cycle_year(year)?;
    let date = NaiveDate::from_ymd_opt(y, month0 + 1, 1)?;
    Some((i64::from(date.num_days_from_ce()) - EPOCH_CE_DAYS + cycles * CYCLE_DAYS) as f64)
}

/// Month `month` (1 to 12) of `year` has a day `date`.
fn is_date(year: f64, month: f64, date: f64) -> bool {
    (1.0..=12.0).contains(&month)
        && (1.0..=31.0).contains(&date)
        && cycle_year(year).is_some_and(|(y, _)| NaiveDate::from_ymd_opt(y, month as u32, date as u32).is_some())
}

fn year_from_time(t: f64) -> f64 {
    civil(day(t)).map_or(f64::NAN, |(y, _, _)| y)
}

fn month_from_time(t: f64) -> f64 {
    civil(day(t)).map_or(f64::NAN, |(_, m, _)| m)
}

fn date_from_time(t: f64) -> f64 {
    civil(day(t)).map_or(f64::NAN, |(_, _, d)| d)
}

fn week_day(t: f64) -> f64 {
    (day(t) + 4.0).rem_euclid(7.0)
}

fn hour_from_time(t: f64) -> f64 {
    (t / MS_PER_HOUR).floor().rem_euclid(24.0)
}

fn min_from_time(t: f64) -> f64 {
    (t / MS_PER_MINUTE).floor().rem_euclid(60.0)
}

fn sec_from_time(t: f64) -> f64 {
    (t / MS_PER_SECOND).floor().rem_euclid(60.0)
}

fn ms_from_time(t: f64) -> f64 {
    t.rem_euclid(MS_PER_SECOND)
}

fn make_time(h: f64, m: f64, s: f64, ms: f64) -> f64 {
    if !(h.is_finite() && m.is_finite() && s.is_finite() && ms.is_finite()) {
        return f64::NAN;
    }
    to_integer(h) * MS_PER_HOUR + to_integer(m) * MS_PER_MINUTE + to_integer(s) * MS_PER_SECOND + to_integer(ms)
}

fn make_day(year: f64, month: f64, date: f64) -> f64 {
    if !(year.is_finite() && month.is_finite() && date.is_finite()) {
        return f64::NAN;
    }
    let (y, m, dt) = (to_integer(year), to_integer(month), to_integer(date));
    let ym = y + (m / 12.0).floor();
    // Years this far out are past every time value TimeClip keeps.
    if ym.abs() > 400_000.0 {
        return f64::NAN;
    }
    first_day_of_month(ym, m.rem_euclid(12.0) as u32).map_or(f64::NAN, |first| first + dt - 1.0)
}

fn make_date(day: f64, time: f64) -> f64 {
    if !(day.is_finite() && time.is_finite()) {
        return f64::NAN;
    }
    day * MS_PER_DAY + time
}

fn time_clip(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > 8.64e15 { f64::NAN } else { to_integer(t) }
}

impl Interp {
    /// The host's offset of local time from UTC at `utc`; an offset that is not finite or is a
    /// day or more counts as 0.
    fn local_offset(&self, utc: f64) -> f64 {
        let o = (self.date_host.offset)(utc);
        if o.is_finite() && o.abs() < MS_PER_DAY { o } else { 0.0 }
    }

    /// LocalTime (section 15.9.1.9).
    fn local_time(&self, t: f64) -> f64 {
        t + self.local_offset(t)
    }

    /// UTC (section 15.9.1.9): the time value of a local time.
    fn utc(&self, t: f64) -> f64 {
        if !t.is_finite() {
            return f64::NAN;
        }
        let guess = t - self.local_offset(t);
        t - self.local_offset(guess)
    }

    fn now(&self) -> f64 {
        time_clip((self.date_host.clock)())
    }
}

// ---- formatting ----

fn year_text(y: f64) -> String {
    if y < 0.0 { format!("-{:06}", -y as i64) } else { format!("{:04}", y as i64) }
}

fn offset_text(offset_ms: f64) -> String {
    let minutes = (offset_ms / MS_PER_MINUTE).round() as i64;
    let sign = if minutes < 0 { '-' } else { '+' };
    format!("GMT{sign}{:02}{:02}", minutes.abs() / 60, minutes.abs() % 60)
}

fn date_text(t: f64) -> String {
    let wd = DAYS.get(week_day(t) as usize).copied().unwrap_or("");
    let mon = MONTHS.get(month_from_time(t) as usize).copied().unwrap_or("");
    format!("{wd} {mon} {:02} {}", date_from_time(t) as i64, year_text(year_from_time(t)))
}

fn time_text(t: f64) -> String {
    format!("{:02}:{:02}:{:02}", hour_from_time(t) as i64, min_from_time(t) as i64, sec_from_time(t) as i64)
}

impl Interp {
    /// `toString`'s format: `Tue Nov 07 2026 12:00:00 GMT+0100`.
    fn date_to_string(&self, t: f64) -> String {
        if t.is_nan() {
            return "Invalid Date".into();
        }
        let local = self.local_time(t);
        format!("{} {} {}", date_text(local), time_text(local), offset_text(local - t))
    }
}

fn utc_text(t: f64) -> String {
    if t.is_nan() {
        return "Invalid Date".into();
    }
    let wd = DAYS.get(week_day(t) as usize).copied().unwrap_or("");
    let mon = MONTHS.get(month_from_time(t) as usize).copied().unwrap_or("");
    format!("{wd}, {:02} {mon} {} {} GMT", date_from_time(t) as i64, year_text(year_from_time(t)), time_text(t))
}

fn iso_text(t: f64) -> String {
    let y = year_from_time(t);
    let year = if (0.0..=9999.0).contains(&y) {
        format!("{:04}", y as i64)
    } else if y < 0.0 {
        format!("-{:06}", -y as i64)
    } else {
        format!("+{:06}", y as i64)
    };
    format!("{year}-{:02}-{:02}T{}.{:03}Z", month_from_time(t) as i64 + 1, date_from_time(t) as i64, time_text(t), ms_from_time(t) as i64)
}

// ---- parsing ----

/// Reads the format of section 15.9.1.15: `YYYY[-MM[-DD]][THH:mm[:ss[.sss]][Z|±HH:mm]]`, with
/// six-digit signed years. A missing offset means UTC.
fn parse_iso(s: &[u8]) -> Option<f64> {
    let mut i = 0;
    let num = |i: &mut usize, n: usize| -> Option<f64> {
        let part = s.get(*i..*i + n)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        *i += n;
        Some(part.iter().fold(0.0, |v, d| v * 10.0 + f64::from(d - b'0')))
    };
    let year = match s.first() {
        Some(b'+') | Some(b'-') => {
            let neg = s.first() == Some(&b'-');
            i = 1;
            let y = num(&mut i, 6)?;
            if neg && y == 0.0 {
                return None;
            }
            if neg { -y } else { y }
        }
        _ => num(&mut i, 4)?,
    };
    let (mut month, mut date) = (1.0, 1.0);
    if s.get(i) == Some(&b'-') {
        i += 1;
        month = num(&mut i, 2)?;
        if s.get(i) == Some(&b'-') {
            i += 1;
            date = num(&mut i, 2)?;
        }
    }
    let (mut h, mut m, mut sec, mut ms) = (0.0, 0.0, 0.0, 0.0);
    let mut offset = 0.0;
    if s.get(i) == Some(&b'T') {
        i += 1;
        h = num(&mut i, 2)?;
        if s.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        m = num(&mut i, 2)?;
        if s.get(i) == Some(&b':') {
            i += 1;
            sec = num(&mut i, 2)?;
            if s.get(i) == Some(&b'.') {
                i += 1;
                let start = i;
                while s.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
                let digits = s.get(start..i)?;
                if digits.is_empty() {
                    return None;
                }
                ms = digits.iter().take(3).fold(0.0, |v, d| v * 10.0 + f64::from(d - b'0')) * 10f64.powi(3 - digits.len().min(3) as i32);
            }
        }
        match s.get(i) {
            Some(b'Z') => i += 1,
            Some(&c @ (b'+' | b'-')) => {
                i += 1;
                let oh = num(&mut i, 2)?;
                if s.get(i) != Some(&b':') {
                    return None;
                }
                i += 1;
                let om = num(&mut i, 2)?;
                if oh > 23.0 || om > 59.0 {
                    return None;
                }
                offset = (oh * 60.0 + om) * MS_PER_MINUTE * if c == b'-' { -1.0 } else { 1.0 };
            }
            _ => {}
        }
    }
    if i != s.len() {
        return None;
    }
    let valid = is_date(year, month, date) && (h < 24.0 || (h == 24.0 && m == 0.0 && sec == 0.0 && ms == 0.0)) && m < 60.0 && sec < 60.0;
    if !valid {
        return None;
    }
    let t = make_date(make_day(year, month - 1.0, date), make_time(h, m, sec, ms)) - offset;
    Some(time_clip(t))
}

/// Reads the formats `toString` and `toUTCString` write: a weekday name, a month name, a day, a
/// year, an optional `HH:mm[:ss]` time and an optional `GMT[±hhmm]` offset, in either order of
/// day and month. Without an offset the time is local.
fn parse_text(interp: &Interp, s: &str) -> Option<f64> {
    let mut words: Vec<&str> = Vec::new();
    for w in s.split(|c: char| c.is_whitespace() || c == ',') {
        if w.starts_with('(') {
            break;
        }
        if !w.is_empty() {
            words.push(w);
        }
    }
    let (mut month, mut date, mut year) = (None, None, None);
    let (mut h, mut m, mut sec) = (0.0, 0.0, 0.0);
    let mut offset: Option<f64> = None;
    for w in words {
        let lower = w.to_ascii_lowercase();
        if DAYS.iter().any(|d| lower.starts_with(&d.to_ascii_lowercase())) && month.is_none() && date.is_none() && w.len() >= 3 && !w.contains(':') {
            continue;
        }
        if let Some(mi) = MONTHS.iter().position(|mo| lower.starts_with(&mo.to_ascii_lowercase())) {
            month = Some(mi as f64);
            continue;
        }
        if let Some(rest) = lower.strip_prefix("gmt").or_else(|| lower.strip_prefix("utc")).or_else(|| (lower == "z").then_some("")) {
            if rest.is_empty() {
                offset = Some(0.0);
                continue;
            }
            let (sign, digits) = match rest.as_bytes().first() {
                Some(b'+') => (1.0, rest.get(1..)?),
                Some(b'-') => (-1.0, rest.get(1..)?),
                _ => return None,
            };
            let digits = digits.replace(':', "");
            if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let v: f64 = digits.parse().ok()?;
            offset = Some(sign * ((v / 100.0).floor() * 60.0 + v % 100.0) * MS_PER_MINUTE);
            continue;
        }
        if w.contains(':') {
            let parts: Vec<&str> = w.split(':').collect();
            let field = |i: usize| -> Option<f64> { parts.get(i).map(|p| p.parse::<f64>().ok()).unwrap_or(Some(0.0)) };
            if parts.len() < 2 || parts.len() > 3 || !parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
                return None;
            }
            (h, m, sec) = (field(0)?, field(1)?, field(2)?);
            if h > 24.0 || m > 59.0 || sec > 59.0 {
                return None;
            }
            continue;
        }
        let (neg, digits) = match w.strip_prefix('-') {
            Some(d) => (true, d),
            None => (false, w),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let v: f64 = digits.parse().ok()?;
        if date.is_none() && !neg && digits.len() <= 2 {
            date = Some(v);
        } else if year.is_none() {
            year = Some(if neg { -v } else { v });
        } else {
            return None;
        }
    }
    let (month, date, year) = (month?, date?, year?);
    if !(1.0..=31.0).contains(&date) {
        return None;
    }
    let local = make_date(make_day(year, month, date), make_time(h, m, sec, 0.0));
    let t = match offset {
        Some(o) => local - o,
        None => interp.utc(local),
    };
    Some(time_clip(t))
}

/// Takes fuel for the units of `s` the parsers read.
fn parse_date(interp: &mut Interp, s: &Str) -> JsResult<f64> {
    charge_units(interp, s.len())?;
    let text = s.to_rust_string();
    let trimmed = text.trim();
    Ok(parse_iso(trimmed.as_bytes()).or_else(|| parse_text(interp, trimmed)).unwrap_or(f64::NAN))
}

// ---- the constructor ----

pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let object_proto = interp.intrinsics.object_proto;
    let proto = interp.heap.alloc_object(Object::new(Class::Date(f64::NAN), Some(object_proto)));
    interp.intrinsics.date_proto = proto;
    let ctor = native(interp, "Date", date_ctor, 7, NativeKind::Plain, true)?;
    constructor(interp, "Date", ctor, proto)?;
    method(interp, ctor, "parse", date_parse, 1)?;
    method(interp, ctor, "UTC", date_utc, 7)?;
    method(interp, ctor, "now", date_now, 0)?;
    let methods: [(&'static str, NativeFn, u32); 44] = [
        ("toString", to_string, 0),
        ("toDateString", to_date_string, 0),
        ("toTimeString", to_time_string, 0),
        ("toLocaleString", to_string, 0),
        ("toLocaleDateString", to_date_string, 0),
        ("toLocaleTimeString", to_time_string, 0),
        ("valueOf", get_time, 0),
        ("getTime", get_time, 0),
        ("getFullYear", get_full_year, 0),
        ("getUTCFullYear", get_utc_full_year, 0),
        ("getMonth", get_month, 0),
        ("getUTCMonth", get_utc_month, 0),
        ("getDate", get_date, 0),
        ("getUTCDate", get_utc_date, 0),
        ("getDay", get_day, 0),
        ("getUTCDay", get_utc_day, 0),
        ("getHours", get_hours, 0),
        ("getUTCHours", get_utc_hours, 0),
        ("getMinutes", get_minutes, 0),
        ("getUTCMinutes", get_utc_minutes, 0),
        ("getSeconds", get_seconds, 0),
        ("getUTCSeconds", get_utc_seconds, 0),
        ("getMilliseconds", get_milliseconds, 0),
        ("getUTCMilliseconds", get_utc_milliseconds, 0),
        ("getTimezoneOffset", get_timezone_offset, 0),
        ("setTime", set_time, 1),
        ("setMilliseconds", set_milliseconds, 1),
        ("setUTCMilliseconds", set_utc_milliseconds, 1),
        ("setSeconds", set_seconds, 2),
        ("setUTCSeconds", set_utc_seconds, 2),
        ("setMinutes", set_minutes, 3),
        ("setUTCMinutes", set_utc_minutes, 3),
        ("setHours", set_hours, 4),
        ("setUTCHours", set_utc_hours, 4),
        ("setDate", set_date, 1),
        ("setUTCDate", set_utc_date, 1),
        ("setMonth", set_month, 2),
        ("setUTCMonth", set_utc_month, 2),
        ("setFullYear", set_full_year, 3),
        ("setUTCFullYear", set_utc_full_year, 3),
        ("toISOString", to_iso_string, 0),
        ("toJSON", to_json, 1),
        ("getYear", get_year, 0),
        ("setYear", set_year, 1),
    ];
    for (name, f, len) in methods {
        method(interp, proto, name, f, len)?;
    }
    // Annex B.2.6: `toGMTString` is the same function object as `toUTCString`.
    method(interp, proto, "toUTCString", to_utc_string, 0)?;
    let utc = interp.get_value(&Value::Object(proto), &key("toUTCString"))?;
    data(interp, proto, "toGMTString", utc, crate::heap::HIDDEN)
}

fn text(interp: &mut Interp, s: &str) -> JsResult<Value> {
    reserve_units(interp, s.len())?;
    Ok(Value::String(Str::from(s)))
}

/// The numbers of the arguments from `year` on, as `new Date(y, m, …)` and `Date.UTC` read them
/// (sections 15.9.3.1 and 15.9.4.3): the local or UTC date they describe, before TimeClip. An
/// omitted month is January, which `Date.UTC(year)` uses (section 15.9.4.3 leaves a call with
/// fewer than two arguments to the implementation).
fn date_from_parts(interp: &mut Interp, args: &[Value]) -> JsResult<f64> {
    let mut n = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    for (i, slot) in n.iter_mut().enumerate() {
        if let Some(v) = args.get(i) {
            *slot = interp.to_number(v)?;
        }
    }
    let [y, m, dt, h, min, s, ms] = n;
    let yi = to_integer(y);
    let yr = if !y.is_nan() && (0.0..=99.0).contains(&yi) { 1900.0 + yi } else { y };
    Ok(make_date(make_day(yr, m, dt), make_time(h, min, s, ms)))
}

/// `Date(…)`, `new Date()`, `new Date(value)` and `new Date(y, m, …)` (sections 15.9.2 and
/// 15.9.3).
fn date_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    if !construct {
        let s = interp.date_to_string(interp.now());
        return text(interp, &s);
    }
    let tv = match args.len() {
        0 => interp.now(),
        1 => {
            let v = interp.to_primitive(&args[0], Hint::Default)?;
            match &v {
                Value::String(s) => parse_date(interp, s)?,
                _ => time_clip(interp.to_number(&v)?),
            }
        }
        _ => {
            let local = date_from_parts(interp, args)?;
            time_clip(interp.utc(local))
        }
    };
    let proto = interp.intrinsics.date_proto;
    Ok(Value::Object(interp.heap.alloc_object(Object::new(Class::Date(tv), Some(proto)))))
}

fn date_parse(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = interp.to_str(&arg(args, 0))?;
    Ok(Value::Number(parse_date(interp, &s)?))
}

fn date_utc(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let t = date_from_parts(interp, args)?;
    Ok(Value::Number(time_clip(t)))
}

fn date_now(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Number(interp.now()))
}

// ---- Date.prototype ----

/// The time value of a Date object `this`.
fn this_time(interp: &mut Interp, this: &Value) -> JsResult<f64> {
    if let Some(Class::Date(t)) = this.as_object().and_then(|o| interp.heap.object(o)).map(|o| &o.class) {
        return Ok(*t);
    }
    Err(interp.type_error("Date.prototype method called on a value that is not a Date object"))
}

fn set_this_time(interp: &mut Interp, this: &Value, t: f64) -> JsResult<Value> {
    if let Some(o) = this.as_object().and_then(|o| interp.heap.object_mut(o))
        && let Class::Date(slot) = &mut o.class
    {
        *slot = t;
    }
    Ok(Value::Number(t))
}

fn to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let t = this_time(interp, &this)?;
    let s = interp.date_to_string(t);
    text(interp, &s)
}

fn to_date_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let t = this_time(interp, &this)?;
    let s = if t.is_nan() { "Invalid Date".into() } else { date_text(interp.local_time(t)) };
    text(interp, &s)
}

fn to_time_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let t = this_time(interp, &this)?;
    let s = if t.is_nan() {
        "Invalid Date".into()
    } else {
        let local = interp.local_time(t);
        format!("{} {}", time_text(local), offset_text(local - t))
    };
    text(interp, &s)
}

fn to_utc_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let t = this_time(interp, &this)?;
    text(interp, &utc_text(t))
}

/// `toISOString` (section 15.9.5.43).
fn to_iso_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let t = this_time(interp, &this)?;
    if !t.is_finite() {
        return Err(interp.throw(ErrorType::Range, "invalid time value"));
    }
    text(interp, &iso_text(t))
}

/// `toJSON` (section 15.9.5.44).
fn to_json(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.to_object(&this)?;
    let ov = Value::Object(o);
    interp.temp_roots.push(ov.clone());
    let r = (|| {
        let tv = interp.to_primitive(&ov, Hint::Number)?;
        if let Value::Number(n) = tv
            && !n.is_finite()
        {
            return Ok(Value::Null);
        }
        let f = interp.get_value(&ov, &key("toISOString"))?;
        if !interp.is_callable(&f) {
            return Err(interp.type_error("toISOString is not a function"));
        }
        interp.call(&f, &ov, &[])
    })();
    interp.temp_roots.pop();
    r
}

fn get_time(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    this_time(interp, &this).map(Value::Number)
}

/// A getter: `f` of the time value, in local time when `local`, NaN for an invalid date.
fn getter(interp: &mut Interp, this: &Value, local: bool, f: fn(f64) -> f64) -> JsResult<Value> {
    let t = this_time(interp, this)?;
    if t.is_nan() {
        return Ok(Value::Number(f64::NAN));
    }
    let t = if local { interp.local_time(t) } else { t };
    Ok(Value::Number(f(t)))
}

macro_rules! getters {
    ($($name:ident, $local:expr, $f:expr;)*) => {
        $(fn $name(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
            getter(interp, &this, $local, $f)
        })*
    };
}

getters! {
    get_full_year, true, year_from_time;
    get_utc_full_year, false, year_from_time;
    get_month, true, month_from_time;
    get_utc_month, false, month_from_time;
    get_date, true, date_from_time;
    get_utc_date, false, date_from_time;
    get_day, true, week_day;
    get_utc_day, false, week_day;
    get_hours, true, hour_from_time;
    get_utc_hours, false, hour_from_time;
    get_minutes, true, min_from_time;
    get_utc_minutes, false, min_from_time;
    get_seconds, true, sec_from_time;
    get_utc_seconds, false, sec_from_time;
    get_milliseconds, true, ms_from_time;
    get_utc_milliseconds, false, ms_from_time;
}

/// `getYear` (Annex B.2.4).
fn get_year(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    getter(interp, &this, true, |t| year_from_time(t) - 1900.0)
}

fn get_timezone_offset(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let t = this_time(interp, &this)?;
    if t.is_nan() {
        return Ok(Value::Number(f64::NAN));
    }
    Ok(Value::Number((t - interp.local_time(t)) / MS_PER_MINUTE))
}

fn set_time(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_time(interp, &this)?;
    let t = interp.to_number(&arg(args, 0))?;
    set_this_time(interp, &this, time_clip(t))
}

/// The time fields a setter starts at: the setter sets that field and the smaller ones after it
/// from its arguments.
#[derive(Clone, Copy)]
enum TimeField {
    Hour = 0,
    Minute = 1,
    Second = 2,
    Ms = 3,
}

/// `setHours`, `setMinutes`, `setSeconds` and `setMilliseconds`, local or UTC (sections
/// 15.9.5.28 to 15.9.5.35).
fn set_time_fields(interp: &mut Interp, this: &Value, args: &[Value], local: bool, first: TimeField, count: usize) -> JsResult<Value> {
    let tv = this_time(interp, this)?;
    let t = if local { interp.local_time(tv) } else { tv };
    let mut fields = [hour_from_time(t), min_from_time(t), sec_from_time(t), ms_from_time(t)];
    let first = first as usize;
    for k in 0..count {
        let given = k == 0 || args.get(k).is_some();
        if given {
            let v = interp.to_number(&arg(args, k))?;
            if let Some(slot) = fields.get_mut(first + k) {
                *slot = v;
            }
        }
    }
    let [h, m, s, ms] = fields;
    let date = make_date(day(t), make_time(h, m, s, ms));
    let u = time_clip(if local { interp.utc(date) } else { date });
    set_this_time(interp, this, u)
}

/// The date fields a setter starts at.
#[derive(Clone, Copy)]
enum DateField {
    Year = 0,
    Month = 1,
    Date = 2,
}

/// `setFullYear`, `setMonth` and `setDate`, local or UTC (sections 15.9.5.36 to 15.9.5.41).
fn set_date_fields(interp: &mut Interp, this: &Value, args: &[Value], local: bool, first: DateField, count: usize) -> JsResult<Value> {
    let tv = this_time(interp, this)?;
    let mut t = if local { interp.local_time(tv) } else { tv };
    // setFullYear starts from +0 on an invalid date.
    if matches!(first, DateField::Year) && t.is_nan() {
        t = 0.0;
    }
    let mut fields = [year_from_time(t), month_from_time(t), date_from_time(t)];
    let first = first as usize;
    for k in 0..count {
        let given = k == 0 || args.get(k).is_some();
        if given {
            let v = interp.to_number(&arg(args, k))?;
            if let Some(slot) = fields.get_mut(first + k) {
                *slot = v;
            }
        }
    }
    let [y, m, d] = fields;
    let date = make_date(make_day(y, m, d), time_within_day(t));
    let u = time_clip(if local { interp.utc(date) } else { date });
    set_this_time(interp, this, u)
}

macro_rules! setters {
    ($($name:ident, $f:ident, $local:expr, $field:expr, $count:expr;)*) => {
        $(fn $name(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
            $f(interp, &this, args, $local, $field, $count)
        })*
    };
}

setters! {
    set_milliseconds, set_time_fields, true, TimeField::Ms, 1;
    set_utc_milliseconds, set_time_fields, false, TimeField::Ms, 1;
    set_seconds, set_time_fields, true, TimeField::Second, 2;
    set_utc_seconds, set_time_fields, false, TimeField::Second, 2;
    set_minutes, set_time_fields, true, TimeField::Minute, 3;
    set_utc_minutes, set_time_fields, false, TimeField::Minute, 3;
    set_hours, set_time_fields, true, TimeField::Hour, 4;
    set_utc_hours, set_time_fields, false, TimeField::Hour, 4;
    set_date, set_date_fields, true, DateField::Date, 1;
    set_utc_date, set_date_fields, false, DateField::Date, 1;
    set_month, set_date_fields, true, DateField::Month, 2;
    set_utc_month, set_date_fields, false, DateField::Month, 2;
    set_full_year, set_date_fields, true, DateField::Year, 3;
    set_utc_full_year, set_date_fields, false, DateField::Year, 3;
}

/// `setYear` (Annex B.2.5).
fn set_year(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let tv = this_time(interp, &this)?;
    let t = if tv.is_nan() { 0.0 } else { interp.local_time(tv) };
    let y = interp.to_number(&arg(args, 0))?;
    if y.is_nan() {
        return set_this_time(interp, &this, f64::NAN);
    }
    let yi = to_integer(y);
    let yr = if (0.0..=99.0).contains(&yi) { 1900.0 + yi } else { y };
    let d = make_day(yr, month_from_time(t), date_from_time(t));
    let u = time_clip(interp.utc(make_date(d, time_within_day(t))));
    set_this_time(interp, &this, u)
}

#[cfg(test)]
mod tests {
    use super::{date_from_time, first_day_of_month, iso_text, make_date, make_day, make_time, month_from_time, parse_iso, week_day, year_from_time};

    #[test]
    fn time_values_split_into_fields_and_back() {
        // 2000-02-29 is a leap day; 1969-12-31 is before the epoch.
        let cases: &[(f64, f64, f64, f64)] = &[(2000.0, 1.0, 29.0, 2.0), (1969.0, 11.0, 31.0, 3.0), (1970.0, 0.0, 1.0, 4.0), (-1.0, 0.0, 1.0, 5.0)];
        for &(y, m, d, wd) in cases {
            let t = make_date(make_day(y, m, d), make_time(13.0, 5.0, 7.0, 9.0));
            assert_eq!((year_from_time(t), month_from_time(t), date_from_time(t), week_day(t)), (y, m, d, wd), "{y}-{m}-{d}");
        }
        assert_eq!(first_day_of_month(1970.0, 0), Some(0.0));
        assert_eq!(make_day(2001.0, -1.0, 1.0), make_day(2000.0, 11.0, 1.0));
        // The ends of the time value range lie past chrono's years; whole 400-year cycles reach them.
        assert_eq!(make_day(275_760.0, 8.0, 13.0), 1e8);
        assert_eq!(make_day(-271_821.0, 3.0, 20.0), -1e8);
        assert_eq!((year_from_time(8.64e15), month_from_time(8.64e15), date_from_time(8.64e15)), (275_760.0, 8.0, 13.0));
        assert_eq!((year_from_time(-8.64e15), month_from_time(-8.64e15), date_from_time(-8.64e15)), (-271_821.0, 3.0, 20.0));
        assert_eq!(make_day(1600.0, 1.0, 29.0) + 1.0, make_day(1600.0, 2.0, 1.0));
        assert_eq!(make_day(1900.0, 1.0, 29.0), make_day(1900.0, 2.0, 1.0));
    }

    #[test]
    fn the_iso_format_reads_back_what_to_iso_string_writes() {
        for t in [0.0, 951_782_400_123.0, -62_198_755_200_000.0, 8.64e15, -8.64e15] {
            let text = iso_text(t);
            assert_eq!(parse_iso(text.as_bytes()), Some(t), "{text}");
        }
        assert_eq!(parse_iso(b"1970"), Some(0.0));
        assert_eq!(parse_iso(b"1970-01-01T00:00+01:00"), Some(-3_600_000.0));
        assert_eq!(parse_iso(b"1970-01-01T24:00:00Z"), Some(86_400_000.0));
        for bad in ["1970-13-01", "1970-02-30", "1970-01-01T24:00:01", "-000000-01-01", "1970-1-1", "1970-01-01T10", "1970-01-01 10:00"] {
            assert_eq!(parse_iso(bad.as_bytes()), None, "{bad}");
        }
    }
}
