//! File → File Info: the document's descriptive metadata (author, description, keywords, rating,
//! copyright) and its created and modified dates, which the SVG, PDF and PNG writers carry. The
//! document title is [`crate::Document::title`]. Every field is defaulted, so documents saved
//! before File Info existed load unchanged.

use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};

/// File Info → Copyright Status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyrightStatus {
    #[default]
    Unknown,
    Copyrighted,
    PublicDomain,
}

impl CopyrightStatus {
    pub const ALL: [CopyrightStatus; 3] = [CopyrightStatus::Unknown, CopyrightStatus::Copyrighted, CopyrightStatus::PublicDomain];
    pub fn id(self) -> &'static str {
        match self {
            CopyrightStatus::Unknown => "unknown",
            CopyrightStatus::Copyrighted => "copyrighted",
            CopyrightStatus::PublicDomain => "publicDomain",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            CopyrightStatus::Unknown => "Unknown",
            CopyrightStatus::Copyrighted => "Copyrighted",
            CopyrightStatus::PublicDomain => "Public Domain",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(s) || c.label().eq_ignore_ascii_case(s))
    }
}

/// Highest File Info rating (stars).
pub const MAX_RATING: u8 = 5;

/// File → File Info.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocMetadata {
    pub author: String,
    /// The author's position or job title.
    pub author_title: String,
    pub description: String,
    /// Keywords, each once, in the order they were added.
    pub keywords: Vec<String>,
    /// 0 (unrated) to [`MAX_RATING`] stars.
    pub rating: u8,
    pub copyright_status: CopyrightStatus,
    pub copyright_notice: String,
    /// Where to read more about the copyright.
    pub copyright_url: String,
    /// When the document was created, as Unix seconds (UTC).
    pub created: Option<i64>,
    /// When the document was last saved, as Unix seconds (UTC).
    pub modified: Option<i64>,
}

impl DocMetadata {
    /// The PNG text entries (registered keywords where one fits) for a document titled `title`:
    /// only the non-empty ones.
    pub fn png_text<'a>(&'a self, title: &'a str) -> Vec<(&'static str, std::borrow::Cow<'a, str>)> {
        let mut out: Vec<(&'static str, std::borrow::Cow<'a, str>)> = vec![];
        let mut push = |k: &'static str, v: std::borrow::Cow<'a, str>| {
            if !v.trim().is_empty() {
                out.push((k, v));
            }
        };
        push("Title", title.into());
        push("Author", self.author.as_str().into());
        push("Description", self.description.as_str().into());
        push("Keywords", self.keywords.join(", ").into());
        push("Copyright", self.copyright_notice.as_str().into());
        push("Copyright URL", self.copyright_url.as_str().into());
        push("Creation Time", self.created.map(iso8601).unwrap_or_default().into());
        out
    }
}

/// The time exported files record: PDF `CreationDate` and `ModDate`, the PNG `Creation Time`
/// entry, a zip entry's date, and a saved document's `created` and `modified`. It is the clock,
/// unless `SOURCE_DATE_EPOCH` is set, which replaces it so that the same commands produce the same
/// bytes twice (#2880).
///
/// The raw clock stays in [`now_unix`], which is what a heartbeat is judged by and what seeds
/// encryption: neither may be pinned by the environment.
pub fn output_time() -> Option<i64> {
    pinned_or(|n| std::env::var(n).ok(), now_unix)
}

/// [`output_time`] with the environment and the clock supplied, so a test can show that the
/// variable is preferred and that the clock answers whenever it does not.
fn pinned_or(get: impl Fn(&str) -> Option<String>, clock: impl Fn() -> Option<i64>) -> Option<i64> {
    stamp_from_env(get).or_else(clock)
}

/// `SOURCE_DATE_EPOCH` as Unix seconds: the reproducible-builds variable
/// (<https://reproducible-builds.org/specs/source-date-epoch/>), a non-negative decimal integer of
/// seconds since the epoch. `None` when it is unset, blank, negative or not an integer, and then
/// the clock decides — an export is never refused over the environment.
fn stamp_from_env(get: impl Fn(&str) -> Option<String>) -> Option<i64> {
    let value = get("SOURCE_DATE_EPOCH")?;
    let value = value.trim();
    // `i64::from_str` also takes a leading `+`, which `date +%s` never prints.
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<i64>().ok()
}

/// The current time as Unix seconds (UTC); `None` where the platform has no clock (the web build).
#[cfg(not(target_arch = "wasm32"))]
pub fn now_unix() -> Option<i64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok())
}
#[cfg(target_arch = "wasm32")]
pub fn now_unix() -> Option<i64> {
    None
}

/// Unix seconds (UTC) → `[year, month, day, hour, minute, second]` (proleptic Gregorian; years
/// clamped to 0–9999).
pub fn civil(t: i64) -> [i64; 6] {
    // Clamp the input first so every input maps to a representable date (0000-01-01 .. 9999-12-31),
    // exactly as the old code did. The clamped instant is always inside chrono's range, so the
    // conversion below never falls back; the trailing `clamp(0, 9999)` is the old year clamp.
    let t = t.clamp(-62_167_219_200, 253_402_300_799);
    let (days, sod) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let (cycles, rest) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    // `rest + 719_163` (1970-01-01 is CE day 719_163) is always a valid CE day, and `sod` < 86_400.
    let date = i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt);
    let time = u32::try_from(sod).ok().and_then(|s| chrono::NaiveTime::from_num_seconds_from_midnight_opt(s, 0));
    match (date, time) {
        (Some(d), Some(tm)) => {
            let year = i128::from(d.year()) + i128::from(cycles) * 400;
            let year = i64::try_from(year).unwrap_or(if year < 0 { i64::MIN } else { i64::MAX });
            [year.clamp(0, 9999), i64::from(d.month()), i64::from(d.day()), i64::from(tm.hour()), i64::from(tm.minute()), i64::from(tm.second())]
        }
        _ => [1970, 1, 1, 0, 0, 0], // unreachable: the clamped `t` is always a valid date
    }
}

/// Unix seconds → `2026-10-05T14:03:00Z`.
pub fn iso8601(t: i64) -> String {
    let [y, mo, d, h, mi, s] = civil(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_statuses() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_791_208_980), "2026-10-05T14:03:00Z");
        // The input is clamped to 0000-01-01..9999-12-31, then the year is clamped to 0..=9999,
        // exactly as the old code did: inputs far past chrono's range land on the clamp bounds,
        // never on the epoch.
        assert_eq!(civil(i64::MIN)[0], 0);
        assert_eq!(civil(i64::MAX)[0], 9999);
        assert_eq!(iso8601(i64::MIN), "0000-01-01T00:00:00Z");
        assert_eq!(iso8601(i64::MAX), "9999-12-31T23:59:59Z");
        assert_eq!(iso8601(10_000_000_000_000), "9999-12-31T23:59:59Z");
        assert_eq!(iso8601(-10_000_000_000_000), "0000-01-01T00:00:00Z");
        assert_eq!(CopyrightStatus::parse("Public Domain"), Some(CopyrightStatus::PublicDomain));
        assert_eq!(CopyrightStatus::parse("publicdomain"), Some(CopyrightStatus::PublicDomain));
        assert_eq!(CopyrightStatus::parse("maybe"), None);
    }

    #[test]
    fn png_text_skips_empty_entries() {
        let m = DocMetadata { author: "Ada".into(), keywords: vec!["a".into(), "b".into()], created: Some(0), ..Default::default() };
        let t: Vec<(&str, String)> = m.png_text("Poster").into_iter().map(|(k, v)| (k, v.into_owned())).collect();
        assert_eq!(
            t,
            [
                ("Title", "Poster".to_string()),
                ("Author", "Ada".into()),
                ("Keywords", "a, b".into()),
                ("Creation Time", "1970-01-01T00:00:00Z".into())
            ]
        );
        assert!(DocMetadata::default().png_text("").is_empty());
    }

    /// `SOURCE_DATE_EPOCH` replaces the clock in exported output only when it is a non-negative
    /// decimal integer; anything else leaves the clock to decide, so an export is never refused.
    #[test]
    fn source_date_epoch_is_read_only_as_a_non_negative_integer() {
        let env = |v: Option<&'static str>| move |n: &str| v.filter(|_| n == "SOURCE_DATE_EPOCH").map(str::to_owned);
        assert_eq!(stamp_from_env(env(Some("1791208980"))), Some(1_791_208_980));
        assert_eq!(stamp_from_env(env(Some("0"))), Some(0));
        assert_eq!(stamp_from_env(env(Some("  1791208980  "))), Some(1_791_208_980));
        for bad in ["", "   ", "-1", "later", "1791208980.5", "1e9", "+1791208980", "+0", "99999999999999999999"] {
            assert_eq!(stamp_from_env(env(Some(bad))), None, "{bad:?}");
        }
        assert_eq!(stamp_from_env(env(None)), None, "unset");
        // Another variable never stands in for it.
        assert_eq!(stamp_from_env(|_| Some("1791208980".to_owned())), Some(1_791_208_980));
        assert_eq!(stamp_from_env(|n: &str| (n == "TZ").then(|| "1791208980".to_owned())), None);
    }

    /// What a file records: the variable when it is usable, the clock whenever it is not. The
    /// environment and the clock are passed in, so no test changes this process's environment --
    /// that is `unsafe` under edition 2024 and would reach every other test running beside it.
    #[test]
    fn a_pinned_epoch_is_preferred_and_the_clock_answers_otherwise() {
        let env = |v: Option<&'static str>| move |_: &str| v.map(str::to_owned);
        let clock = || Some(1_000_000);
        assert_eq!(pinned_or(env(Some("1791208980")), clock), Some(1_791_208_980), "the variable wins over the clock");
        assert_eq!(pinned_or(env(Some("0")), clock), Some(0), "including the epoch itself");
        for ignored in [None, Some(""), Some("-1"), Some("later")] {
            assert_eq!(pinned_or(env(ignored), clock), Some(1_000_000), "{ignored:?} leaves it to the clock");
        }
        // A platform with no clock and no variable has no date to record.
        assert_eq!(pinned_or(env(None), || None), None);
        assert_eq!(pinned_or(env(Some("1791208980")), || None), Some(1_791_208_980), "the variable needs no clock");
    }
}
